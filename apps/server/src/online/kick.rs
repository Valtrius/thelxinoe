//! Per-user tracked channels; optional application credentials enrich public metadata.
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};
use tokio::sync::Mutex;
#[cfg(test)]
mod tests;
pub(super) struct Runtime {
    token_url: String,
    api: String,
    token: Mutex<Option<(String, String, i64)>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            token_url: "https://id.kick.com/oauth/token".into(),
            api: "https://api.kick.com/public/v1".into(),
            token: Mutex::new(None),
        }
    }
}
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/online/kick",
            get(configuration).put(configure),
        )
        .route("/api/v1/online/kick", get(feed).delete(disconnect))
        .route("/api/v1/online/kick/connect", post(connect))
        .route("/api/v1/online/kick/channels", post(add))
        .route(
            "/api/v1/online/kick/channels/{slug}",
            axum::routing::delete(remove),
        )
        .route(
            "/api/v1/online/kick/data",
            axum::routing::delete(delete_data),
        )
}
#[derive(Serialize, Deserialize)]
struct Client {
    client_id: String,
    client_secret: String,
}
impl Client {
    fn valid(&self) -> bool {
        !self.client_id.is_empty()
            && self.client_id.len() <= 256
            && !self.client_secret.is_empty()
            && self.client_secret.len() <= 1024
            && !self
                .client_id
                .chars()
                .chain(self.client_secret.chars())
                .any(char::is_control)
    }
}
async fn client(state: &AppState) -> Result<Client> {
    let bytes = state
        .secrets
        .get(&state.db, "provider.kick")
        .await?
        .ok_or_else(|| ApiError::bad("Kick metadata needs application credentials in Settings"))?;
    let c: Client = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("Invalid Kick application configuration"))?;
    if !c.valid() {
        return Err(ApiError::bad("Invalid Kick application configuration"));
    }
    Ok(c)
}
async fn configuration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(json!({"configured":client(&state).await.is_ok()})))
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(c): Json<Client>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !c.valid() {
        return Err(ApiError::bad(
            "Enter a Kick application client ID and secret",
        ));
    }
    let mut token = state.online.kick.token.lock().await;
    let encrypted = state.secrets.encrypt(
        "provider.kick",
        &serde_json::to_vec(&c).expect("client serialization"),
    )?;
    state.db.call(move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO secrets VALUES ('provider.kick',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;
        tx.execute("UPDATE kick_channels SET generation=?1,next_run=0,failures=0,error=NULL",[id()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','kick',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(())}).await?;
    *token = None;
    Ok(Json(json!({"saved":true})))
}
pub(super) fn slug(input: &str) -> Result<String> {
    let input = input.trim();
    let value = if input.contains("://") {
        let url = url::Url::parse(input)
            .map_err(|_| ApiError::bad("Enter a Kick channel name or URL"))?;
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("kick.com" | "www.kick.com"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::bad("Enter a public Kick channel URL"));
        }
        url.path().trim_matches('/').to_owned()
    } else {
        input.to_owned()
    }
    .to_ascii_lowercase();
    if !(1..=100).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ApiError::bad("Enter one Kick channel name"));
    }
    Ok(value)
}
#[derive(Deserialize)]
struct Add {
    channel: String,
}
async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Add>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let slug = slug(&input.channel)?;
    let saved = slug.clone();
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        anyhow::ensure!(tx.query_row("SELECT COUNT(*) FROM kick_channels WHERE user_id=?1",[&p.user.id],|r|r.get::<_,i64>(0))?<100,"Tracked Kick channel limit reached");
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,status,updated_at) VALUES (?1,'kick',?2,'connected',?3) ON CONFLICT(user_id,provider) DO UPDATE SET status='connected',updated_at=excluded.updated_at",params![p.user.id,id(),now()])?;
        tx.execute("INSERT INTO kick_channels(user_id,slug,generation) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![p.user.id,slug,id()])?;tx.commit()?;Ok(())}).await?;
    Ok(Json(json!({"slug":saved})))
}
async fn feed(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let (connected,items)=state.db.call(move|db|{let connected=db.query_row("SELECT status='connected' FROM online_accounts WHERE user_id=?1 AND provider='kick'",[&p.user.id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let items=db.prepare("SELECT slug,title,category,live,viewers,updated_at,next_run,error,thumbnail_url,started_at FROM kick_channels WHERE user_id=?1 ORDER BY COALESCE(live,0) DESC,viewers DESC,slug")?.query_map([p.user.id],|r|Ok(json!({"slug":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"live":r.get::<_,Option<bool>>(3)?,"viewers":r.get::<_,i64>(4)?,"updated_at":r.get::<_,i64>(5)?,"next_run":r.get::<_,i64>(6)?,"error":r.get::<_,Option<String>>(7)?,"thumbnail_url":r.get::<_,Option<String>>(8)?,"started_at":r.get::<_,Option<String>>(9)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok((connected,items))}).await?;
    Ok(Json(
        json!({"connected":connected,"configured":client(&state).await.is_ok(),"items":items}),
    ))
}
async fn connect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    state.db.call(move|db|{db.execute("INSERT INTO online_accounts(user_id,provider,generation,status,updated_at) VALUES (?1,'kick',?2,'connected',?3) ON CONFLICT(user_id,provider) DO UPDATE SET status='connected',generation=excluded.generation,updated_at=excluded.updated_at",params![p.user.id,id(),now()])?;Ok(())}).await?;
    Ok(Json(json!({"connected":true})))
}
async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    state.db.call(move|db|{db.execute("UPDATE online_accounts SET status='disconnected',generation=?1,updated_at=?2 WHERE user_id=?3 AND provider='kick'",params![id(),now(),p.user.id])?;Ok(())}).await?;
    Ok(Json(json!({"disconnected":true})))
}
async fn erase(
    state: &AppState,
    headers: &HeaderMap,
    channel: Option<String>,
) -> Result<Json<Value>> {
    let p = security::principal(state, headers).await?;
    let ids=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let media=channel.as_ref().map(|s|format!("kick:{s}"));
        let ids=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1 AND live_media_id LIKE 'kick:%' AND (?2 IS NULL OR live_media_id=?2) AND state IN ('ready','playing','paused')")?.query_map(params![p.user.id,media],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for key in &ids {tx.execute("UPDATE playback_sessions SET state='stopped' WHERE id=?1",[key])?;tx.execute("DELETE FROM playback_grants WHERE resource=?1",[format!("playback:{key}")])?;}
        tx.execute("DELETE FROM kick_channels WHERE user_id=?1 AND (?2 IS NULL OR slug=?2)",params![p.user.id,channel])?;
        if channel.is_none(){tx.execute("DELETE FROM live_history WHERE user_id=?1 AND media_id LIKE 'kick:%'",[&p.user.id])?;tx.execute("DELETE FROM online_accounts WHERE user_id=?1 AND provider='kick'",[&p.user.id])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.delete_data','kick',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(ids)}).await?;
    for key in ids {
        state.playback.stop(&key).await;
    }
    Ok(Json(json!({"deleted":true})))
}
async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel): Path<String>,
) -> Result<Json<Value>> {
    erase(&state, &headers, Some(slug(&channel)?)).await
}
async fn delete_data(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    erase(&state, &headers, None).await
}
fn failure() -> ApiError {
    ApiError::bad(
        "Kick metadata is unavailable; check application settings or retry after the displayed delay",
    )
}
async fn response(
    request: reqwest::RequestBuilder,
) -> Result<(axum::http::StatusCode, HeaderMap, Value)> {
    let response = request.send().await.map_err(|_| failure())?;
    let (status, headers, bytes) = super::bounded_response(response)
        .await
        .map_err(|_| failure())?;
    let value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
    Ok((status, headers, value))
}
async fn token(state: &AppState) -> Result<String> {
    let mut cache = state.online.kick.token.lock().await;
    let c = client(state).await?;
    let hash = thelxinoe_auth::digest(&format!("{}\0{}", c.client_id, c.client_secret));
    if let Some((key, value, expires)) = &*cache
        && *key == hash
        && *expires > now() + 60
    {
        return Ok(value.clone());
    }
    let (status, _, v) = response(state.online.http.post(&state.online.kick.token_url).form(&[
        ("grant_type", "client_credentials"),
        ("client_id", &c.client_id),
        ("client_secret", &c.client_secret),
    ]))
    .await?;
    if !status.is_success() {
        return Err(failure());
    }
    let token = v["access_token"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or_else(failure)?
        .to_owned();
    let expires = v["expires_in"]
        .as_i64()
        .filter(|v| *v > 0 && *v <= 86400 * 60)
        .ok_or_else(failure)?;
    *cache = Some((hash, token.clone(), now() + expires));
    Ok(token)
}
struct Turn {
    user: String,
    slug: String,
    generation: String,
    account: String,
    failures: u32,
}
async fn tick(state: &AppState) -> anyhow::Result<()> {
    let turn=state.db.call(|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let limited=tx.query_row("SELECT CAST(value AS INTEGER)>?1 FROM settings WHERE key='kick.rate_limited_until'",[now()],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);if limited{return Ok(None)}
        let t=tx.query_row("SELECT c.user_id,c.slug,c.generation,a.generation,c.failures FROM kick_channels c JOIN online_accounts a ON a.user_id=c.user_id AND a.provider='kick' AND a.status='connected' WHERE c.next_run<=?1 ORDER BY c.last_turn,c.user_id,c.slug LIMIT 1",[now()],|r|Ok(Turn{user:r.get(0)?,slug:r.get(1)?,generation:r.get(2)?,account:r.get(3)?,failures:r.get(4)?})).optional()?;
        if let Some(t)=&t{tx.execute("UPDATE kick_channels SET next_run=?1,last_turn=(SELECT COALESCE(MAX(last_turn),0)+1 FROM kick_channels) WHERE user_id=?2 AND slug=?3",params![now()+90,t.user,t.slug])?;}tx.commit()?;Ok(t)}).await?;
    let Some(t) = turn else { return Ok(()) };
    if let Err(error) = step(state, &t).await {
        let delay = 30i64.saturating_mul(1i64 << t.failures.min(7)).min(3600);
        state.db.call(move|db|{db.execute("UPDATE kick_channels SET error=?1,failures=failures+1,next_run=MAX(next_run,?2) WHERE user_id=?3 AND slug=?4 AND generation=?5",params![error.2,now()+delay,t.user,t.slug,t.generation])?;Ok(())}).await?;
    }
    Ok(())
}
async fn step(state: &AppState, t: &Turn) -> Result<()> {
    let token = token(state).await?;
    let (status, headers, value) = response(
        state
            .online
            .http
            .get(format!("{}/channels", state.online.kick.api))
            .bearer_auth(token)
            .query(&[("slug", &t.slug)]),
    )
    .await?;
    if status == axum::http::StatusCode::UNAUTHORIZED {
        *state.online.kick.token.lock().await = None;
        return Err(failure());
    }
    if status == axum::http::StatusCode::TOO_MANY_REQUESTS {
        let delay = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(60)
            .clamp(1, 3600);
        state.db.call(move|db|{db.execute("INSERT INTO settings VALUES ('kick.rate_limited_until',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[(now()+delay).to_string()])?;Ok(())}).await?;
        return Err(failure());
    }
    if !status.is_success() {
        return Err(failure());
    }
    let rows = value["data"]
        .as_array()
        .filter(|r| r.len() <= 50)
        .ok_or_else(failure)?;
    let row = rows
        .iter()
        .find(|r| {
            r["slug"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(&t.slug))
        })
        .ok_or_else(|| ApiError::bad("Kick channel was not found"))?;
    let live = row["stream"]["is_live"].as_bool().ok_or_else(failure)?;
    let viewers = row["stream"]["viewer_count"].as_i64().unwrap_or(0).max(0);
    let title = row["stream_title"]
        .as_str()
        .ok_or_else(failure)?
        .chars()
        .take(1000)
        .collect::<String>();
    let category = row["category"]["name"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    let thumbnail = super::public_image(row["stream"]["thumbnail"].as_str());
    let started_at = row["stream"]["start_time"]
        .as_str()
        .filter(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        .map(str::to_owned);
    let user = t.user.clone();
    let slug = t.slug.clone();
    let generation = t.generation.clone();
    let account = t.account.clone();
    state.db.call(move|db|{db.execute("UPDATE kick_channels SET title=?1,category=?2,live=?3,viewers=?4,updated_at=?5,next_run=?5+60,failures=0,error=NULL,thumbnail_url=?10,started_at=?11 WHERE user_id=?6 AND slug=?7 AND generation=?8 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=?6 AND a.provider='kick' AND a.status='connected' AND a.generation=?9)",params![title,category,live,viewers,now(),user,slug,generation,account,thumbnail,started_at])?;Ok(())}).await?;
    Ok(())
}
pub(super) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        tick(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
