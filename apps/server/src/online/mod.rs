pub(crate) mod downloads;
mod extract;
mod feed;
pub(crate) mod kick;
pub(crate) mod live;
pub(crate) mod oauth;
mod process;
mod quota;
pub(crate) mod streams;
mod sync;
pub(crate) mod tools;
mod twitch;
mod youtube;
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
pub async fn run(state: AppState) -> anyhow::Result<()> {
    tokio::try_join!(
        sync::run(state.clone()),
        twitch::run(state.clone()),
        kick::run(state)
    )?;
    Ok(())
}
use thelxinoe_core::{Capability, now};

// Provider images are loaded by the client, with no credentials or server fetch.
pub(super) fn public_image(value: Option<&str>) -> Option<String> {
    let value = value.filter(|s| s.len() <= 2048)?;
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host == "static-cdn.jtvnw.net"
                || host == "kick.com"
                || host.ends_with(".kick.com")
                || host.ends_with(".kickcdn.com")
        }))
    .then(|| value.to_owned())
}
pub struct Runtime {
    http: reqwest::Client,
    authorize: String,
    token: String,
    api: String,
    slots: tokio::sync::Semaphore,
    refresh: tokio::sync::Mutex<()>,
    extraction: tokio::sync::Semaphore,
    pub(crate) streams: streams::Runtime,
    twitch: twitch::Runtime,
    kick: kick::Runtime,
}
impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            api: "https://www.googleapis.com/youtube/v3".into(),
            slots: tokio::sync::Semaphore::new(4),
            refresh: tokio::sync::Mutex::new(()),
            extraction: tokio::sync::Semaphore::new(2),
            streams: streams::Runtime::default(),
            twitch: twitch::Runtime::default(),
            kick: kick::Runtime::default(),
        })
    }
}
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(twitch::router())
        .merge(kick::router())
        .route("/api/v1/admin/online", get(configuration).put(configure))
        .route(
            "/api/v1/admin/online/tools",
            get(tools::status).post(tools::request_install),
        )
        .route("/api/v1/online/youtube", get(account).delete(disconnect))
        .route("/api/v1/online/youtube/connect", post(oauth::start))
        .route("/api/v1/online/youtube/callback", get(oauth::callback))
        .route("/api/v1/online/youtube/sync", post(sync::request))
        .route("/api/v1/online/youtube/feed", get(feed::list))
        .route("/api/v1/online/youtube/watchlist", post(feed::add))
        .route(
            "/api/v1/admin/online/downloads",
            get(downloads::settings).put(downloads::configure),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}/download",
            get(downloads::status).post(downloads::request),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}/extract",
            post(extract::inspect),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}",
            axum::routing::put(feed::edit),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}/artwork",
            get(feed::artwork),
        )
        .route(
            "/api/v1/online/youtube/data",
            axum::routing::delete(feed::delete_data),
        )
}
#[derive(Deserialize, Serialize)]
pub(crate) struct Google {
    pub client_id: String,
    pub client_secret: String,
}
impl Google {
    fn validate(&self) -> Result<()> {
        if !self.client_id.ends_with(".apps.googleusercontent.com")
            || self.client_id.len() > 256
            || self.client_secret.is_empty()
            || self.client_secret.len() > 512
            || self
                .client_id
                .chars()
                .chain(self.client_secret.chars())
                .any(char::is_control)
        {
            return Err(ApiError::bad(
                "Enter a Google Web application client ID and client secret",
            ));
        }
        Ok(())
    }
    fn hash(&self) -> String {
        thelxinoe_auth::digest(&format!("{}\0{}", self.client_id, self.client_secret))
    }
}
async fn google(state: &AppState) -> Result<Google> {
    let bytes = state
        .secrets
        .get(&state.db, "provider.google")
        .await?
        .ok_or_else(|| {
            ApiError::bad("An administrator must configure Google application credentials")
        })?;
    let client: Google = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("Google application credentials are invalid"))?;
    client.validate()?;
    Ok(client)
}
fn redirect_uri(state: &AppState) -> Result<String> {
    let base =
        state.config.public_url.as_ref().ok_or_else(|| {
            ApiError::bad("Set THELXINOE_PUBLIC_URL before linking online accounts")
        })?;
    let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if base.scheme() != "https" && !loopback {
        return Err(ApiError::bad(
            "Online account linking requires an HTTPS public URL (HTTP localhost is allowed for development)",
        ));
    }
    Ok(format!(
        "{}/api/v1/online/youtube/callback",
        base.origin().ascii_serialization()
    ))
}
async fn configuration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let settings = state
        .db
        .call(|db| {
            let downloads = db
                .query_row(
                    "SELECT value FROM settings WHERE key='youtube_downloads'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .is_some_and(|v| v == "true");
            let budget = db
                .query_row(
                    "SELECT value FROM settings WHERE key='youtube_daily_quota'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(10000);
            Ok((downloads, budget))
        })
        .await?;
    Ok(Json(
        json!({"google_configured":google(&state).await.is_ok(),"redirect_uri":redirect_uri(&state).ok(),"youtube_downloads":settings.0,"youtube_daily_quota":settings.1,"quota":quota::status(&state).await?}),
    ))
}
#[derive(Deserialize)]
struct Settings {
    google: Option<Google>,
    youtube_downloads: bool,
    youtube_daily_quota: u32,
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Settings>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !(1..=10_000_000).contains(&input.youtube_daily_quota) {
        return Err(ApiError::bad(
            "Daily quota must be between 1 and 10,000,000 units",
        ));
    }
    let changed = if let Some(client) = &input.google {
        client.validate()?;
        google(&state)
            .await
            .ok()
            .is_none_or(|old| old.hash() != client.hash())
    } else {
        false
    };
    let encrypted = input
        .google
        .as_ref()
        .map(|g| {
            state.secrets.encrypt(
                "provider.google",
                &serde_json::to_vec(g).expect("credential serialization"),
            )
        })
        .transpose()?;
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(encrypted)=encrypted{tx.execute("INSERT INTO secrets VALUES ('provider.google',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;}
        if changed {
            tx.execute("UPDATE online_accounts SET status=CASE WHEN status='disconnected' THEN status ELSE 'reconnect_required' END,credential=NULL,expires_at=0,generation=?1 WHERE provider='youtube'",[thelxinoe_core::id()])?;
            tx.execute("DELETE FROM oauth_attempts WHERE provider='youtube'",[])?;
            tx.execute("DELETE FROM youtube_sync",[])?;
        }
        for (key,value) in [("youtube_downloads",input.youtube_downloads.to_string()),("youtube_daily_quota",input.youtube_daily_quota.to_string())]{tx.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','youtube',?2)",params![p.user.id,now()])?;
        tx.commit()?;Ok(())
    }).await?;
    state
        .emit(None, "online.configuration.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
async fn account(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let account=state.db.call(move|db|Ok(db.query_row("SELECT status,display_name,external_id,updated_at FROM online_accounts WHERE user_id=?1 AND provider='youtube'",[p.user.id],|r|Ok(json!({"status":r.get::<_,String>(0)?,"display_name":r.get::<_,String>(1)?,"external_id":r.get::<_,String>(2)?,"updated_at":r.get::<_,i64>(3)?}))).optional()?)).await?.unwrap_or(json!({"status":"disconnected","display_name":"","external_id":""}));
    Ok(Json(
        json!({"account":account,"configured":google(&state).await.is_ok(),"linking_available":redirect_uri(&state).is_ok(),"linking_url":state.config.public_url.as_ref().map(|u|format!("{}/?section=YouTube",u.origin().ascii_serialization())),"quota":quota::status(&state).await?}),
    ))
}
async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE online_accounts SET status='disconnected',credential=NULL,expires_at=0,generation=?1,updated_at=?2 WHERE user_id=?3 AND provider='youtube'",params![thelxinoe_core::id(),now(),user])?;
        tx.execute("DELETE FROM oauth_attempts WHERE user_id=?1 AND provider='youtube'",[&user])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.disconnect','youtube',?2)",params![user,now()])?;
        tx.commit()?;Ok(())
    }).await?;
    state
        .emit(
            Some(p.user.id),
            "online.account.changed",
            json!({"provider":"youtube"}),
        )
        .await?;
    Ok(Json(json!({"disconnected":true})))
}
async fn bounded_response(
    mut response: reqwest::Response,
) -> std::result::Result<(axum::http::StatusCode, HeaderMap, Vec<u8>), std::io::Error> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| std::io::Error::other("Online provider response failed"))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(std::io::Error::other(
                "Online provider response exceeds limit",
            ));
        }
        bytes.extend(chunk);
    }
    Ok((status, headers, bytes))
}
