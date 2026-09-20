//! Server-owned Twitch device authorization and bounded followed-live synchronization.
use super::bounded_response;
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};
#[cfg(test)]
mod tests;
const SCOPE: &str = "user:read:follows";
pub(super) struct Runtime {
    auth: String,
    api: String,
    start: tokio::sync::Mutex<()>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            auth: "https://id.twitch.tv/oauth2".into(),
            api: "https://api.twitch.tv/helix".into(),
            start: tokio::sync::Mutex::new(()),
        }
    }
}
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/online/twitch", get(account).delete(disconnect))
        .route("/api/v1/online/twitch/connect", post(start))
        .route(
            "/api/v1/online/twitch/data",
            axum::routing::delete(delete_data),
        )
        .route("/api/v1/online/twitch/feed", get(feed))
        .route(
            "/api/v1/admin/online/twitch",
            get(configuration).put(configure),
        )
}
async fn client(state: &AppState) -> Result<String> {
    let bytes = state
        .secrets
        .get(&state.db, "provider.twitch_client_id")
        .await?
        .ok_or_else(|| {
            ApiError::bad("An administrator must configure a Twitch public application client ID")
        })?;
    let value: String = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("Invalid Twitch application configuration"))?;
    if !(8..=128).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(ApiError::bad("Invalid Twitch client ID"));
    }
    Ok(value)
}
async fn configuration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(json!({"configured":client(&state).await.is_ok()})))
}
#[derive(Deserialize)]
struct Configuration {
    client_id: String,
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Configuration>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !(8..=128).contains(&input.client_id.len())
        || !input.client_id.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(ApiError::bad("Enter a Twitch public application client ID"));
    }
    let _lock = state.online.twitch.start.lock().await;
    let changed = client(&state).await.ok().as_ref() != Some(&input.client_id);
    let encrypted = state.secrets.encrypt(
        "provider.twitch_client_id",
        &serde_json::to_vec(&input.client_id).expect("string serialization"),
    )?;
    state.db.call(move|db|{let tx=db.transaction()?;
        tx.execute("INSERT INTO secrets VALUES ('provider.twitch_client_id',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;
        if changed {
            tx.execute("UPDATE online_accounts SET credential=NULL,status=CASE WHEN status='disconnected' THEN status ELSE 'reconnect_required' END,generation=?1,expires_at=0 WHERE provider='twitch'",[id()])?;
            tx.execute("DELETE FROM twitch_attempts",[])?;tx.execute("DELETE FROM twitch_sync",[])?;
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','twitch',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(())
    }).await?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize, Serialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: i64,
    interval: i64,
}
#[derive(Deserialize, Serialize)]
struct Credential {
    access_token: String,
    refresh_token: String,
}
#[derive(Deserialize)]
struct Validation {
    client_id: String,
    user_id: String,
    login: String,
    scopes: Vec<String>,
    expires_in: i64,
}
fn provider_error() -> ApiError {
    ApiError::bad("Twitch could not complete the request; retry after the displayed delay")
}
async fn response(
    request: reqwest::RequestBuilder,
) -> Result<(axum::http::StatusCode, HeaderMap, Value)> {
    let response = request.send().await.map_err(|_| provider_error())?;
    let (status, headers, bytes) = bounded_response(response)
        .await
        .map_err(|_| provider_error())?;
    let value = serde_json::from_slice(&bytes).map_err(|_| provider_error())?;
    Ok((status, headers, value))
}
async fn account(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let data=state.db.call(move|db|{
        let account=db.query_row("SELECT status,display_name FROM online_accounts WHERE user_id=?1 AND provider='twitch'",[&p.user.id],|r|Ok(json!({"status":r.get::<_,String>(0)?,"display_name":r.get::<_,String>(1)?}))).optional()?.unwrap_or(json!({"status":"disconnected","display_name":""}));
        let attempt=db.query_row("SELECT generation,device,expires_at,error FROM twitch_attempts WHERE user_id=?1 AND session_id=?2 AND expires_at>?3",params![p.user.id,p.session_id,now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<String>>(3)?))).optional()?;
        let sync=db.query_row("SELECT last_complete,next_run,error FROM twitch_sync WHERE user_id=?1",[p.user.id],|r|Ok(json!({"last_complete":r.get::<_,Option<i64>>(0)?,"next_run":r.get::<_,i64>(1)?,"error":r.get::<_,Option<String>>(2)?}))).optional()?;
        Ok((account,attempt,sync))
    }).await?;
    let pending = if let Some((generation, encrypted, expires, error)) = data.1 {
        let device: Device = serde_json::from_slice(
            &state
                .secrets
                .decrypt(&format!("twitch-device:{generation}"), &encrypted)?,
        )
        .map_err(|_| provider_error())?;
        Some(
            json!({"user_code":device.user_code,"verification_uri":device.verification_uri,"expires_at":expires,"error":error}),
        )
    } else {
        None
    };
    Ok(Json(
        json!({"account":data.0,"pending":pending,"sync":data.2,"configured":client(&state).await.is_ok()}),
    ))
}
async fn start(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let _lock = state.online.twitch.start.lock().await;
    let client = client(&state).await?;
    let generation = id();
    let user = p.user.id.clone();
    let gen_copy = generation.clone();
    let reserved=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let recent=tx.query_row("SELECT updated_at>?1-30 FROM online_accounts WHERE user_id=?2 AND provider='twitch'",params![now(),user],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        if recent{return Ok(false)}
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES (?1,'twitch',?2,?3) ON CONFLICT(user_id,provider) DO UPDATE SET generation=excluded.generation,updated_at=excluded.updated_at",params![user,gen_copy,now()])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1",[&user])?;
        tx.execute("UPDATE twitch_sync SET generation=?1 WHERE user_id=?2",params![gen_copy,user])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !reserved {
        return Err(ApiError::conflict(
            "Wait thirty seconds before starting another Twitch connection",
        ));
    }
    let (status, _, value) = response(
        state
            .online
            .http
            .post(format!("{}/device", state.online.twitch.auth))
            .form(&[("client_id", client.as_str()), ("scopes", SCOPE)]),
    )
    .await?;
    if !status.is_success() {
        return Err(ApiError::bad(
            "Twitch rejected device sign-in. Check that the application is registered as a public client.",
        ));
    }
    let device: Device = serde_json::from_value(value).map_err(|_| provider_error())?;
    let url = url::Url::parse(&device.verification_uri).map_err(|_| provider_error())?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.twitch.tv")
        || url.path() != "/activate"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !(1..=1800).contains(&device.expires_in)
        || !(1..=60).contains(&device.interval)
        || !(1..=32).contains(&device.user_code.len())
        || !(1..=2048).contains(&device.device_code.len())
    {
        return Err(provider_error());
    }
    let expires = now() + device.expires_in;
    let interval = device.interval.max(5);
    let encrypted = state.secrets.encrypt(
        &format!("twitch-device:{generation}"),
        &serde_json::to_vec(&device).expect("device serialization"),
    )?;
    let hash = thelxinoe_auth::digest(&client);
    let stored=state.db.call(move|db|Ok(db.execute("INSERT INTO twitch_attempts(user_id,session_id,generation,client_hash,device,expires_at,next_poll,interval) SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='twitch' AND generation=?3) AND EXISTS(SELECT 1 FROM sessions WHERE id=?2 AND user_id=?1 AND expires_at>?9)",params![p.user.id,p.session_id,generation,hash,encrypted,expires,now()+interval,interval,now()])?==1)).await?;
    if !stored {
        return Err(ApiError::conflict("The connection attempt was cancelled"));
    }
    Ok(Json(
        json!({"user_code":device.user_code,"verification_uri":device.verification_uri,"expires_at":expires}),
    ))
}
async fn clear(state: &AppState, headers: &HeaderMap, delete: bool) -> Result<Json<Value>> {
    let p = security::principal(state, headers).await?;
    let stopped=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE online_accounts SET credential=NULL,status='disconnected',expires_at=0,generation=?1,updated_at=?2 WHERE user_id=?3 AND provider='twitch'",params![id(),now(),p.user.id])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1",[&p.user.id])?;tx.execute("DELETE FROM twitch_sync WHERE user_id=?1",[&p.user.id])?;
        let stopped=if delete {
            let ids=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1 AND live_media_id LIKE 'twitch:%' AND state IN ('ready','playing','paused')")?.query_map([&p.user.id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for key in &ids {tx.execute("UPDATE playback_sessions SET state='stopped' WHERE id=?1",[key])?;tx.execute("DELETE FROM playback_grants WHERE resource=?1",[format!("playback:{key}")])?;}
            tx.execute("DELETE FROM live_history WHERE user_id=?1 AND media_id LIKE 'twitch:%'",[&p.user.id])?;
            ids
        }else{vec![]};
        if delete {tx.execute("DELETE FROM twitch_streams WHERE user_id=?1",[&p.user.id])?;tx.execute("DELETE FROM online_accounts WHERE user_id=?1 AND provider='twitch'",[&p.user.id])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,'twitch',?3)",params![p.user.id,if delete{"online.delete_data"}else{"online.disconnect"},now()])?;
        tx.commit()?;Ok(stopped)
    }).await?;
    for key in stopped {
        state.playback.stop(&key).await;
    }
    Ok(Json(json!({"disconnected":true})))
}
async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    clear(&state, &headers, false).await
}
async fn delete_data(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    clear(&state, &headers, true).await
}
async fn feed(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let rows=state.db.call(move|db|Ok(db.prepare("SELECT channel_id,login,display_name,title,category,viewers,started_at,thumbnail_url,profile_image_url FROM twitch_streams WHERE user_id=?1 AND active=1 ORDER BY viewers DESC,login LIMIT 1000")?.query_map([p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"login":r.get::<_,String>(1)?,"display_name":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"category":r.get::<_,String>(4)?,"viewers":r.get::<_,i64>(5)?,"started_at":r.get::<_,String>(6)?,"thumbnail_url":r.get::<_,Option<String>>(7)?,"profile_image_url":r.get::<_,Option<String>>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
struct Attempt {
    user: String,
    session: String,
    generation: String,
    hash: String,
    device: Vec<u8>,
    interval: i64,
}
async fn claim_attempt(state: &AppState) -> anyhow::Result<Option<Attempt>> {
    state.db.call(|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM twitch_attempts WHERE expires_at<=?1 OR NOT EXISTS(SELECT 1 FROM sessions s WHERE s.id=twitch_attempts.session_id AND s.expires_at>?1)",[now()])?;
        let a=tx.query_row("SELECT user_id,session_id,generation,client_hash,device,interval FROM twitch_attempts WHERE next_poll<=?1 ORDER BY next_poll LIMIT 1",[now()],|r|Ok(Attempt{user:r.get(0)?,session:r.get(1)?,generation:r.get(2)?,hash:r.get(3)?,device:r.get(4)?,interval:r.get(5)?})).optional()?;
        if let Some(a)=&a {tx.execute("UPDATE twitch_attempts SET next_poll=?1 WHERE user_id=?2 AND generation=?3",params![now()+90,a.user,a.generation])?;}
        tx.commit()?;Ok(a)
    }).await
}
async fn validate(state: &AppState, client: &str, access: &str) -> Result<Validation> {
    let (status, _, value) = response(
        state
            .online
            .http
            .get(format!("{}/validate", state.online.twitch.auth))
            .header("Authorization", format!("OAuth {access}")),
    )
    .await?;
    if status == axum::http::StatusCode::UNAUTHORIZED {
        return Err(ApiError::unauthorized());
    }
    if !status.is_success() {
        return Err(provider_error());
    }
    let v: Validation = serde_json::from_value(value).map_err(|_| provider_error())?;
    if v.client_id != client
        || !v.scopes.iter().any(|s| s == SCOPE)
        || v.user_id.is_empty()
        || v.user_id.len() > 64
        || v.login.len() > 100
        || v.expires_in <= 0
    {
        return Err(ApiError::unauthorized());
    }
    Ok(v)
}
fn tokens(value: Value) -> Result<Credential> {
    let c: Credential = serde_json::from_value(value).map_err(|_| provider_error())?;
    if c.access_token.is_empty()
        || c.refresh_token.is_empty()
        || c.access_token.len() > 4096
        || c.refresh_token.len() > 4096
    {
        return Err(provider_error());
    }
    Ok(c)
}
async fn poll(state: &AppState, a: &Attempt) -> Result<()> {
    let client = client(state).await?;
    if thelxinoe_auth::digest(&client) != a.hash {
        return Err(ApiError::unauthorized());
    }
    let device: Device = serde_json::from_slice(
        &state
            .secrets
            .decrypt(&format!("twitch-device:{}", a.generation), &a.device)?,
    )
    .map_err(|_| provider_error())?;
    let (status, _, value) = response(
        state
            .online
            .http
            .post(format!("{}/token", state.online.twitch.auth))
            .form(&[
                ("client_id", client.as_str()),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]),
    )
    .await?;
    if !status.is_success() {
        let message = value["message"]
            .as_str()
            .or(value["error"].as_str())
            .unwrap_or("");
        let (delay, terminal) = match message {
            "authorization_pending" => (a.interval, false),
            "slow_down" => (a.interval + 5, false),
            "access_denied" | "expired_token" | "invalid device code" => (0, true),
            _ => (a.interval.max(30), false),
        };
        let user = a.user.clone();
        let generation = a.generation.clone();
        state.db.call(move|db|{if terminal{db.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND generation=?2",params![user,generation])?;}else{db.execute("UPDATE twitch_attempts SET next_poll=?1,interval=?2 WHERE user_id=?3 AND generation=?4",params![now()+delay,delay.min(300),user,generation])?;}Ok(())}).await?;
        return Ok(());
    }
    let credential = tokens(value)?;
    let v = validate(state, &client, &credential.access_token).await?;
    let encrypted = state.secrets.encrypt(
        &format!("twitch:{}", a.user),
        &serde_json::to_vec(&credential).expect("credential serialization"),
    )?;
    let user = a.user.clone();
    let session = a.session.clone();
    let generation = a.generation.clone();
    let connected=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT EXISTS(SELECT 1 FROM twitch_attempts t JOIN sessions s ON s.id=t.session_id JOIN online_accounts a ON a.user_id=t.user_id AND a.provider='twitch' AND a.generation=t.generation WHERE t.user_id=?1 AND t.generation=?2 AND t.session_id=?3 AND s.expires_at>?4 AND t.expires_at>?4)",params![user,generation,session,now()],|r|r.get::<_,bool>(0))?;
        if !valid{return Ok(false)}
        tx.execute("UPDATE online_accounts SET credential=?1,status='connected',external_id=?2,display_name=?3,expires_at=?4,updated_at=?5 WHERE user_id=?6 AND provider='twitch' AND generation=?7",params![encrypted,v.user_id,v.login,now()+v.expires_in.min(86400*60),now(),user,generation])?;
        tx.execute("INSERT INTO twitch_sync(user_id,generation,validated_at) VALUES (?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET generation=excluded.generation,validated_at=excluded.validated_at,next_run=0,cursor='',snapshot='',pages=0,error=NULL,failures=0",params![user,generation,now()])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND generation=?2",params![user,generation])?;tx.commit()?;Ok(true)
    }).await?;
    if connected {
        state
            .emit(
                Some(a.user.clone()),
                "online.account.changed",
                json!({"provider":"twitch"}),
            )
            .await?;
    }
    Ok(())
}
pub(super) async fn run(state: AppState) -> anyhow::Result<()> {
    // Force validation on server startup in addition to the hourly check.
    state
        .db
        .call(|db| {
            db.execute("UPDATE twitch_sync SET validated_at=0", [])?;
            Ok(())
        })
        .await?;
    loop {
        if let Some(a) = claim_attempt(&state).await?
            && let Err(error) = poll(&state, &a).await
        {
            state.db.call(move|db|{db.execute("UPDATE twitch_attempts SET error=?1,next_poll=?2 WHERE user_id=?3 AND generation=?4",params![error.2,now()+30,a.user,a.generation])?;Ok(())}).await?;
        }
        sync_tick(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
struct Turn {
    user: String,
    generation: String,
    external: String,
    credential: Vec<u8>,
    expires: i64,
    validated: i64,
    cursor: String,
    snapshot: String,
    pages: u32,
    failures: u32,
}
async fn sync_tick(state: &AppState) -> anyhow::Result<()> {
    let turn=state.db.call(|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let turn=tx.query_row("SELECT t.user_id,t.generation,a.external_id,a.credential,a.expires_at,t.validated_at,t.cursor,t.snapshot,t.pages,t.failures FROM twitch_sync t JOIN online_accounts a ON a.user_id=t.user_id AND a.provider='twitch' AND a.generation=t.generation AND a.status='connected' WHERE t.next_run<=?1 ORDER BY t.last_turn,t.user_id LIMIT 1",[now()],|r|Ok(Turn{user:r.get(0)?,generation:r.get(1)?,external:r.get(2)?,credential:r.get(3)?,expires:r.get(4)?,validated:r.get(5)?,cursor:r.get(6)?,snapshot:r.get(7)?,pages:r.get(8)?,failures:r.get(9)?})).optional()?;
        if let Some(t)=&turn {tx.execute("UPDATE twitch_sync SET next_run=?1,last_turn=(SELECT COALESCE(MAX(last_turn),0)+1 FROM twitch_sync) WHERE user_id=?2",params![now()+90,t.user])?;}
        tx.commit()?;Ok(turn)
    }).await?;
    let Some(t) = turn else { return Ok(()) };
    if let Err(error) = sync_step(state, &t).await {
        state.db.call(move|db|{let tx=db.transaction()?;
            if error.0==axum::http::StatusCode::UNAUTHORIZED {tx.execute("UPDATE online_accounts SET status='reconnect_required',credential=NULL,expires_at=0 WHERE user_id=?1 AND provider='twitch' AND generation=?2",params![t.user,t.generation])?;}
            let delay=30i64.saturating_mul(1i64<<t.failures.min(7)).min(3600);
            tx.execute("UPDATE twitch_sync SET error=?1,failures=failures+1,next_run=MAX(next_run,?2) WHERE user_id=?3 AND generation=?4",params![if error.0==axum::http::StatusCode::UNAUTHORIZED{"Reconnect Twitch to continue synchronization".to_owned()}else{error.2},now()+delay,t.user,t.generation])?;tx.commit()?;Ok(())
        }).await?;
    }
    Ok(())
}
async fn sync_step(state: &AppState, t: &Turn) -> Result<()> {
    let client = client(state).await?;
    let mut credential: Credential = serde_json::from_slice(
        &state
            .secrets
            .decrypt(&format!("twitch:{}", t.user), &t.credential)?,
    )
    .map_err(|_| provider_error())?;
    let refresh = t.expires <= now() + 120;
    if refresh {
        let (status, _, value) = response(
            state
                .online
                .http
                .post(format!("{}/token", state.online.twitch.auth))
                .form(&[
                    ("client_id", client.as_str()),
                    ("refresh_token", credential.refresh_token.as_str()),
                    ("grant_type", "refresh_token"),
                ]),
        )
        .await?;
        if status == axum::http::StatusCode::BAD_REQUEST
            || status == axum::http::StatusCode::UNAUTHORIZED
        {
            return Err(ApiError::unauthorized());
        }
        if !status.is_success() {
            return Err(provider_error());
        }
        credential = tokens(value)?;
        // Public-client refresh tokens are single-use. Persist the replacement before another request.
        let encrypted = state.secrets.encrypt(
            &format!("twitch:{}", t.user),
            &serde_json::to_vec(&credential).expect("credential serialization"),
        )?;
        let user = t.user.clone();
        let generation = t.generation.clone();
        let changed=state.db.call(move|db|Ok(db.execute("UPDATE online_accounts SET credential=?1,expires_at=?2 WHERE user_id=?3 AND provider='twitch' AND generation=?4 AND status='connected'",params![encrypted,now()+300,user,generation])?==1)).await?;
        if !changed {
            return Ok(());
        }
    }
    if refresh || t.validated <= now() - 3500 {
        let v = validate(state, &client, &credential.access_token).await?;
        if v.user_id != t.external {
            return Err(ApiError::unauthorized());
        }
        let user = t.user.clone();
        let generation = t.generation.clone();
        state.db.call(move|db|{let tx=db.transaction()?;tx.execute("UPDATE online_accounts SET expires_at=?1 WHERE user_id=?2 AND provider='twitch' AND generation=?3 AND status='connected'",params![now()+v.expires_in.min(86400*60),user,generation])?;tx.execute("UPDATE twitch_sync SET validated_at=?1 WHERE user_id=?2 AND generation=?3",params![now(),user,generation])?;tx.commit()?;Ok(())}).await?;
    }
    let (status, headers, value) = response(
        state
            .online
            .http
            .get(format!("{}/streams/followed", state.online.twitch.api))
            .bearer_auth(&credential.access_token)
            .header("Client-Id", &client)
            .query(&[
                ("user_id", t.external.as_str()),
                ("first", "100"),
                ("after", t.cursor.as_str()),
            ]),
    )
    .await?;
    let mut reset = headers
        .get("ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(now() + 60)
        .clamp(now() + 1, now() + 3600);
    let mut limited = status == axum::http::StatusCode::TOO_MANY_REQUESTS
        || headers.get("ratelimit-remaining").is_some_and(|v| v == "0");
    if limited {
        let user = t.user.clone();
        let generation = t.generation.clone();
        state
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE twitch_sync SET next_run=?1 WHERE user_id=?2 AND generation=?3",
                    params![reset, user, generation],
                )?;
                Ok(())
            })
            .await?;
    }
    if status == axum::http::StatusCode::UNAUTHORIZED {
        return Err(ApiError::unauthorized());
    }
    if !status.is_success() {
        return Err(provider_error());
    }
    let rows = value["data"]
        .as_array()
        .filter(|a| a.len() <= 100)
        .ok_or_else(provider_error)?
        .clone();
    let next = value["pagination"]["cursor"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    if next.len() > 2048 || (!next.is_empty() && (next == t.cursor || t.pages >= 99)) {
        return Err(ApiError::bad(
            "Twitch returned an invalid or excessive page sequence",
        ));
    }
    for r in &rows {
        if !r["user_id"].as_str().is_some_and(|v| {
            !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_digit())
        }) || !r["user_login"].as_str().is_some_and(|v| {
            !v.is_empty()
                && v.len() <= 100
                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }) || !r["user_name"].is_string()
            || !r["title"].is_string()
            || !r["game_name"].is_string()
            || !r["started_at"].is_string()
            || !r["viewer_count"].as_i64().is_some_and(|v| v >= 0)
        {
            return Err(provider_error());
        }
    }
    let mut profiles = std::collections::HashMap::<String, String>::new();
    if !limited && !rows.is_empty() {
        let ids = rows
            .iter()
            .filter_map(|r| r["user_id"].as_str().map(|id| ("id", id)))
            .collect::<Vec<_>>();
        if let Ok((profile_status, profile_headers, users)) = response(
            state
                .online
                .http
                .get(format!("{}/users", state.online.twitch.api))
                .bearer_auth(&credential.access_token)
                .header("Client-Id", &client)
                .query(&ids),
        )
        .await
        {
            if profile_status == axum::http::StatusCode::TOO_MANY_REQUESTS
                || profile_headers
                    .get("ratelimit-remaining")
                    .is_some_and(|v| v == "0")
            {
                limited = true;
                reset = profile_headers
                    .get("ratelimit-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(now() + 60)
                    .clamp(now() + 1, now() + 3600);
            }
            if profile_status.is_success()
                && let Some(users) = users["data"].as_array()
            {
                for user in users.iter().take(100) {
                    if let (Some(id), Some(image)) = (
                        user["id"].as_str(),
                        super::public_image(user["profile_image_url"].as_str()),
                    ) {
                        profiles.insert(id.to_owned(), image);
                    }
                }
            }
        }
    }
    let user = t.user.clone();
    let generation = t.generation.clone();
    let snapshot = if t.snapshot.is_empty() {
        id()
    } else {
        t.snapshot.clone()
    };
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='twitch' AND generation=?2 AND status='connected')",params![user,generation],|r|r.get::<_,bool>(0))?;
        if !valid{return Ok(())}
        for r in rows {tx.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,thumbnail_url,profile_image_url) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(user_id,channel_id) DO UPDATE SET login=excluded.login,display_name=excluded.display_name,title=excluded.title,category=excluded.category,viewers=excluded.viewers,started_at=excluded.started_at,snapshot=excluded.snapshot,thumbnail_url=excluded.thumbnail_url,profile_image_url=COALESCE(excluded.profile_image_url,twitch_streams.profile_image_url)",params![user,r["user_id"].as_str(),r["user_login"].as_str(),r["user_name"].as_str(),r["title"].as_str(),r["game_name"].as_str(),r["viewer_count"].as_i64(),r["started_at"].as_str(),snapshot,super::public_image(r["thumbnail_url"].as_str()).map(|url|url.replace("{width}","640").replace("{height}","360")),profiles.get(r["user_id"].as_str().unwrap_or(""))])?;}
        if next.is_empty(){
            tx.execute("DELETE FROM twitch_streams WHERE user_id=?1 AND snapshot<>?2",params![user,snapshot])?;
            tx.execute("UPDATE twitch_streams SET active=1 WHERE user_id=?1",[&user])?;
            tx.execute("UPDATE twitch_sync SET cursor='',snapshot='',pages=0,next_run=?1,last_complete=?2,failures=0,error=NULL WHERE user_id=?3 AND generation=?4",params![if limited{reset.max(now()+60)}else{now()+60},now(),user,generation])?;
        }else{tx.execute("UPDATE twitch_sync SET cursor=?1,snapshot=?2,pages=pages+1,next_run=?3,failures=0,error=NULL WHERE user_id=?4 AND generation=?5",params![next,snapshot,if limited{reset}else{now()+1},user,generation])?;}
        tx.commit()?;Ok(())
    }).await?;
    Ok(())
}
