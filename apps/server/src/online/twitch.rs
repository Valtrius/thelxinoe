//! Server-owned Twitch device authorization and bounded followed-live synchronization.

#[path = "../storage/online/twitch.rs"]
mod storage;

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
    storage::configure(&state.db, p, changed, encrypted).await?;
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
    let data = storage::account(&state.db, p).await?;
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
    let reserved = storage::start_write_online_accounts(&state.db, user, gen_copy).await?;
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
    let stored = storage::start_write_twitch_attempts(
        &state.db, p, generation, expires, interval, encrypted, hash,
    )
    .await?;
    if !stored {
        return Err(ApiError::conflict("The connection attempt was cancelled"));
    }
    Ok(Json(
        json!({"user_code":device.user_code,"verification_uri":device.verification_uri,"expires_at":expires}),
    ))
}
async fn clear(state: &AppState, headers: &HeaderMap, delete: bool) -> Result<Json<Value>> {
    let p = security::principal(state, headers).await?;
    let stopped = storage::clear(p, &state.db, delete).await?;
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
    let rows = storage::feed(&state.db, p).await?;
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
    storage::claim_attempt(&state.db).await
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
        storage::poll_write_twitch_attempts(delay, terminal, user, generation, &state.db).await?;
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
    let connected =
        storage::complete_authorization(v, encrypted, user, session, generation, &state.db).await?;
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
    storage::run_write_twitch_sync(&state.db).await?;
    loop {
        if let Some(a) = claim_attempt(&state).await?
            && let Err(error) = poll(&state, &a).await
        {
            storage::run_write_twitch_attempts(a, error, &state.db).await?;
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
    let turn = storage::sync_tick_write_twitch_sync(&state.db).await?;
    let Some(t) = turn else { return Ok(()) };
    if let Err(error) = sync_step(state, &t).await {
        storage::sync_tick_write_online_accounts(t, error, &state.db).await?;
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
        let changed =
            storage::sync_step_write_online_accounts(encrypted, user, generation, &state.db)
                .await?;
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
        storage::record_validation(v, user, generation, &state.db).await?;
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
        storage::sync_step_write_twitch_sync(reset, user, generation, &state.db).await?;
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
            || r["viewer_count"].as_i64().is_none_or(|v| v < 0)
        {
            return Err(provider_error());
        }
    }
    let mut profiles = std::collections::HashMap::<String, String>::new();
    let owner = t.user.clone();
    let profile_due = storage::sync_step_read_online_accounts(owner, &state.db).await?;
    let mut own_profile = None;
    if !limited {
        let mut ids = rows
            .iter()
            .filter_map(|r| r["user_id"].as_str().map(|id| ("id", id)))
            .collect::<Vec<_>>();
        if profile_due && !ids.iter().any(|(_, id)| *id == t.external) {
            ids.push(("id", t.external.as_str()));
        }
        for ids in ids.chunks(100) {
            if let Ok((profile_status, profile_headers, users)) = response(
                state
                    .online
                    .http
                    .get(format!("{}/users", state.online.twitch.api))
                    .bearer_auth(&credential.access_token)
                    .header("Client-Id", &client)
                    .query(ids),
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
                        if user["id"].as_str() == Some(t.external.as_str()) {
                            own_profile = Some((
                                user["display_name"]
                                    .as_str()
                                    .map(|name| name.chars().take(200).collect::<String>()),
                                super::public_image(user["profile_image_url"].as_str()),
                            ));
                        }
                        if let (Some(id), Some(image)) = (
                            user["id"].as_str(),
                            super::public_image(user["profile_image_url"].as_str()),
                        ) {
                            profiles.insert(id.to_owned(), image);
                        }
                    }
                }
            }
            if limited {
                break;
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
    let profile_changed = storage::save_sync_page(
        &state.db,
        storage::SyncPage {
            reset,
            limited,
            rows,
            next,
            profiles,
            own_profile,
            user,
            generation,
            snapshot,
        },
    )
    .await?;
    if profile_changed {
        state
            .emit(
                Some(t.user.clone()),
                "online.account.changed",
                json!({"provider":"twitch"}),
            )
            .await?;
    }
    Ok(())
}
