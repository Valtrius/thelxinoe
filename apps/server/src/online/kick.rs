//! Per-user tracked channels; optional application credentials enrich public metadata.

#[path = "../storage/online/kick.rs"]
mod storage;

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
    storage::configure(&state.db, p, encrypted).await?;
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
    storage::add(&state.db, p, slug).await?;
    Ok(Json(json!({"slug":saved})))
}
async fn feed(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let (connected, items) = storage::feed(&state.db, p).await?;
    Ok(Json(
        json!({"connected":connected,"configured":client(&state).await.is_ok(),"items":items}),
    ))
}
async fn connect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::connect(&state.db, p).await?;
    Ok(Json(json!({"connected":true})))
}
async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::disconnect(&state.db, p).await?;
    Ok(Json(json!({"disconnected":true})))
}
async fn erase(
    state: &AppState,
    headers: &HeaderMap,
    channel: Option<String>,
) -> Result<Json<Value>> {
    let p = security::principal(state, headers).await?;
    let ids = storage::erase(p, &state.db, channel).await?;
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
    let turn = storage::tick_write_settings(&state.db).await?;
    let Some(t) = turn else { return Ok(()) };
    if let Err(error) = step(state, &t).await {
        let delay = 30i64.saturating_mul(1i64 << t.failures.min(7)).min(3600);
        storage::tick_write_kick_channels(t, delay, error, &state.db).await?;
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
        storage::step_write_settings(delay, &state.db).await?;
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
    let mut display_name = None;
    let mut profile = None;
    let mut language = None;
    let mut mature = false;
    let mut tags = Vec::<String>::new();
    if let Some(id) = row["broadcaster_user_id"].as_u64()
        && let Ok(users) = presentation_metadata(state, "users", "id", id).await
    {
        if let Some(user) = users
            .as_array()
            .and_then(|rows| rows.iter().find(|r| r["user_id"].as_u64() == Some(id)))
        {
            display_name = user["name"]
                .as_str()
                .map(|s| s.chars().take(100).collect::<String>());
            profile = super::public_image(user["profile_picture"].as_str());
        }
        if live
            && let Ok(streams) =
                presentation_metadata(state, "users/livestreams", "user_id", id).await
            && let Some(stream) = streams.as_array().and_then(|rows| {
                rows.iter().find(|r| {
                    r["channel"]["slug"]
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&t.slug))
                })
            })
        {
            display_name = stream["broadcaster_user"]["username"]
                .as_str()
                .map(|s| s.chars().take(100).collect::<String>())
                .or(display_name);
            profile = super::public_image(stream["broadcaster_user"]["profile_picture"].as_str())
                .or(profile);
            language = stream["language_code"]
                .as_str()
                .map(|s| s.chars().take(20).collect::<String>());
            mature = stream["has_mature_content"].as_bool().unwrap_or(false);
            tags = stream["tags"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|v| v.as_str().map(|s| s.chars().take(100).collect::<String>()))
                        .take(20)
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    let user = t.user.clone();
    let slug = t.slug.clone();
    let generation = t.generation.clone();
    let account = t.account.clone();
    storage::step_write_kick_channels(
        &state.db,
        storage::ChannelUpdate {
            live,
            viewers,
            title,
            category,
            thumbnail,
            started_at,
            display_name,
            profile,
            language,
            mature,
            tags,
            user,
            slug,
            generation,
            account,
        },
    )
    .await?;
    Ok(())
}
pub(super) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        tick(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

async fn presentation_metadata(state: &AppState, path: &str, key: &str, id: u64) -> Result<Value> {
    let access = token(state).await?;
    let (status, headers, bytes) = super::bounded_response(
        state
            .online
            .http
            .get(format!("{}/{path}", state.online.kick.api))
            .bearer_auth(access)
            .query(&[(key, id.to_string())])
            .send()
            .await
            .map_err(|_| failure())?,
    )
    .await
    .map_err(|_| failure())?;
    if status == axum::http::StatusCode::TOO_MANY_REQUESTS {
        let delay = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(60)
            .clamp(1, 3600);
        storage::presentation_metadata(delay, &state.db).await?;
    }
    if !status.is_success() {
        return Err(failure());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
    Ok(value["data"].clone())
}
