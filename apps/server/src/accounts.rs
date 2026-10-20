#[path = "storage/accounts.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use thelxinoe_auth::{
    SessionAuthorization, digest, issue_session, password_hash, user_row, validate_credentials,
    verify_password,
};
use thelxinoe_core::{Capability, Role, id, now};

#[derive(Deserialize)]
pub struct Credentials {
    pub(crate) username: String,
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) transport: Option<String>,
    #[serde(default)]
    pub(crate) device_name: Option<String>,
    #[serde(default)]
    pub(crate) remember_token: Option<String>,
}

pub async fn setup_status(State(state): State<AppState>) -> Result<Json<Value>> {
    let count = storage::setup_status(&state.db).await?;
    Ok(Json(
        json!({"setup_required":count==0,"version":thelxinoe_core::VERSION}),
    ))
}
/// Issues the session a completed sign-in earned and answers with it: in a cookie for the web,
/// or in the body for a device, whose native transport stores and strips it. A device
/// remembered for two-step sign-in gets its `remember` token the same way.
pub(crate) async fn respond_session(
    state: &AppState,
    authorization: SessionAuthorization,
    c: &Credentials,
    secure: bool,
    remember: Option<&str>,
) -> Result<Response> {
    let transport = c.transport.as_deref().unwrap_or("web");
    if !matches!(transport, "web" | "device") {
        return Err(ApiError::bad("Unsupported credential transport"));
    }
    let user_id = authorization.user_id().to_owned();
    let raw = issue_session(
        &state.db,
        authorization,
        transport.into(),
        c.device_name
            .as_deref()
            .unwrap_or(if transport == "web" {
                "Browser"
            } else {
                "Desktop"
            })
            .chars()
            .take(100)
            .collect(),
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    let user = storage::respond_session(&state.db, user_id).await?;
    let user = crate::avatars::profile(state, user).await?;
    let device = transport == "device";
    let mut body = json!({"user":user,"token":device.then_some(&raw)});
    if device && let Some(remember) = remember {
        body["remember_token"] = json!(remember);
    }
    let mut response = Json(body).into_response();
    if !device {
        response.headers_mut().insert(
            header::SET_COOKIE,
            security::cookie(&raw, secure).parse().unwrap(),
        );
        if let Some(remember) = remember {
            response.headers_mut().append(
                header::SET_COOKIE,
                crate::authentication::remember_cookie(remember, secure)
                    .parse()
                    .unwrap(),
            );
        }
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}
#[derive(Deserialize)]
pub struct Setup {
    #[serde(flatten)]
    credentials: Credentials,
    /// The admin browser's zone. It becomes the server default so maintenance windows
    /// such as 03:00–05:00 fall at local night; an unknown zone keeps UTC.
    #[serde(default)]
    timezone: Option<String>,
}
pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(Setup {
        credentials: c,
        timezone,
    }): Json<Setup>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    if storage::setup_status(&state.db).await? != 0 {
        return Err(ApiError::conflict("Setup is already complete"));
    }
    if !matches!(c.transport.as_deref().unwrap_or("web"), "web" | "device") {
        return Err(ApiError::bad("Unsupported transport"));
    }
    validate_credentials(&c.username, &c.password).map_err(|e| ApiError::bad(e.to_string()))?;
    allow_password_attempt(&state, format!("setup:{}", context.bucket())).await?;
    let slot = state
        .password_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Authentication is busy. Try again shortly.".into(),
            )
        })?;
    let hash = password_hash(c.password.clone(), Some(slot)).await?;
    let user_id = id();
    let uid = user_id.clone();
    let username = c.username.clone();
    let timezone = timezone
        .and_then(|zone| zone.parse::<chrono_tz::Tz>().ok())
        .map(|zone| zone.name().to_owned());
    let created = storage::setup(&state.db, hash.clone(), uid, username, timezone).await?;
    if !created {
        return Err(ApiError::conflict("Setup is already complete"));
    }
    respond_session(
        &state,
        SessionAuthorization::Password {
            user_id,
            expected_hash: hash,
        },
        &c,
        context.secure,
        None,
    )
    .await
}
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let remembered = c.remember_token.clone().or_else(|| {
        headers
            .get(header::COOKIE)?
            .to_str()
            .ok()?
            .split(';')
            .find_map(|part| part.trim().strip_prefix("thelxinoe_remember="))
            .map(str::to_owned)
    });
    let user = check_credentials(
        &state,
        context.address,
        &c.username,
        &c.password,
        remembered,
    )
    .await?;
    crate::authentication::password_login(&state, user, &c, &headers, context.secure).await
}
pub(crate) async fn check_credentials(
    state: &AppState,
    address: std::net::IpAddr,
    username: &str,
    password: &str,
    remembered: Option<String>,
) -> Result<SessionAuthorization> {
    allow_password_attempt(state, security::client_bucket(address)).await?;
    if password.len() > 256 || username.len() > 64 {
        return Err(ApiError::unauthorized());
    }
    // Keyed by name whether or not the account exists, so throttling reveals nothing. A device
    // remembered after two-step sign-in to this account isn't locked out by others' failed
    // guesses; its own failures still count, and the per-address limit still applies.
    let throttle = format!("password-account:{}", username.to_ascii_lowercase());
    let trusted = match remembered.filter(|token| token.len() == 64) {
        Some(token) => {
            storage::remembered_device(&state.db, username.to_owned(), digest(&token)).await?
        }
        None => false,
    };
    if !trusted {
        reserve_account_attempt(state, &throttle).await?;
    }
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let username = username.to_owned();
    let record = storage::check_credentials(username, &state.db).await?;
    let hash = record
        .as_ref()
        .map(|r| r.1.clone())
        .unwrap_or_else(|| state.dummy_hash.as_ref().clone());
    let valid = verify_password(password.to_owned(), hash, Some(slot)).await?;
    if !valid || record.is_none() {
        // An untrusted attempt already counts; a remembered device's failures count here.
        if trusted {
            record_account_failure(state, throttle).await?;
        }
        return Err(ApiError::unauthorized());
    }
    if !trusted {
        release_account_attempt(state, throttle).await?;
    }
    let (user_id, expected_hash) = record.unwrap();
    Ok(SessionAuthorization::Password {
        user_id,
        expected_hash,
    })
}
pub(crate) use storage::{QUICK_CONNECT_LOOKUP, QUICK_CONNECT_START, throttle};
pub(crate) async fn allow_password_attempt(state: &AppState, address: String) -> Result<()> {
    let allowed = storage::throttled(&state.db, address, storage::PER_CLIENT).await?;
    if !allowed {
        return Err(ApiError(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts. Try again in 15 minutes.".into(),
        ));
    }
    Ok(())
}
/// Counts an attempt against the account before its proof is checked, in the same write
/// that checks the limit, so concurrent guesses can't all pass it. A proof that succeeds
/// gives its attempt back with `release_account_attempt`.
pub(crate) async fn reserve_account_attempt(state: &AppState, key: &str) -> Result<()> {
    if !storage::throttled(&state.db, key.to_owned(), storage::PER_ACCOUNT).await? {
        return Err(ApiError(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many failed attempts for this account. Try again in 15 minutes.".into(),
        ));
    }
    Ok(())
}
pub(crate) async fn release_account_attempt(state: &AppState, key: String) -> Result<()> {
    storage::refunded(&state.db, key).await?;
    Ok(())
}
/// Counts a failed proof against the account without refusing it (a remembered device).
pub(crate) async fn record_account_failure(state: &AppState, key: String) -> Result<()> {
    storage::throttled(&state.db, key, storage::PER_ACCOUNT).await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    #[serde(default)]
    current_password: String,
    new_password: String,
    /// Media apps keep working through client passwords unless these are revoked too.
    #[serde(default)]
    revoke_client_passwords: bool,
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordChange>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    validate_credentials(&principal.user.username, &input.new_password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    crate::authentication::password_change_proof(&state, &principal, input.current_password)
        .await?;
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = password_hash(input.new_password, Some(slot)).await?;
    crate::authentication::set_password(
        &state,
        principal.clone(),
        hash,
        input.revoke_client_passwords,
    )
    .await?;
    state
        .emit(
            Some(principal.user.id),
            "account.password.changed",
            json!({}),
        )
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = security::principal(&state, &headers).await?.user;
    Ok(Json(
        json!({"user":crate::avatars::profile(&state, user).await?}),
    ))
}
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let p = security::principal(&state, &headers).await?;
    storage::logout(&state.db, p).await?;
    state.notify_events();
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        "thelxinoe_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(response)
}
pub async fn sessions(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let current = p.session_id.clone();
    let rows = storage::sessions(&state.db, p).await?;
    Ok(Json(json!({"items":rows,"current":current})))
}
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    crate::authentication::revoke_session(&state, p, session).await?;
    state.notify_events();
    Ok(Json(json!({"ok":true})))
}
pub async fn users(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let rows = storage::users(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    password: String,
    role: Role,
}
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut user): Json<NewUser>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    crate::authentication::require_fresh(&state, &p).await?;
    validate_credentials(&user.username, &user.password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = password_hash(std::mem::take(&mut user.password), Some(slot)).await?;
    let result = storage::create_user(&state.db, user, p, hash).await?;
    result
        .map(|id| Json(json!({"id":id})))
        .ok_or_else(|| ApiError::conflict("Username already exists"))
}
pub async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let _policy = state.transcoding.read().await;
    let value = storage::settings(&state.db).await?;
    Ok(Json(
        json!({"timezone":value.timezone,"time_format":value.time_format,"video_transcoding":value.transcoding.video_transcoding,"audio_transcoding":value.transcoding.audio_transcoding,"public_url":state.config.public_url.as_ref().map(ToString::to_string),"trusted_proxies":state.config.trusted_proxies.iter().map(ToString::to_string).collect::<Vec<_>>()}),
    ))
}
pub(crate) struct SettingsView {
    timezone: String,
    time_format: String,
    transcoding: crate::transcoding::Policy,
}
pub(crate) async fn transcoding_policy(
    db: &thelxinoe_database::Database,
) -> anyhow::Result<crate::transcoding::Policy> {
    Ok(storage::settings(db).await?.transcoding)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    timezone: Option<String>,
    time_format: Option<String>,
    video_transcoding: Option<bool>,
    audio_transcoding: Option<bool>,
}
pub async fn save_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<Settings>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if let Some(timezone) = &settings.timezone {
        timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| ApiError::bad("Unknown timezone"))?;
    }
    if let Some(format) = &settings.time_format
        && !matches!(format.as_str(), "12h" | "24h")
    {
        return Err(ApiError::bad("Unknown time format"));
    }
    // Serialize policy edits with playback preparation and lazy VOD encoding.
    let mut policy = state.transcoding.write().await;
    let value = storage::save_settings(&state.db, settings, p)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    *policy = value.transcoding;
    for run in state.playback.runs().await {
        if let Some(encoding) = state.playback.encoding(&run).await
            && policy.check(encoding).is_err()
        {
            let _lifecycle = state.playback_lifecycle.write(&[&run.session]).await;
            state.playback.stop_revision(&run).await;
            crate::playback::fail(&state, &run.session).await?;
        }
    }
    state
        .emit(
            None,
            "server.settings.changed",
            json!({"timezone":value.timezone,"time_format":value.time_format,"video_transcoding":policy.video_transcoding,"audio_transcoding":policy.audio_transcoding}),
        )
        .await?;
    Ok(Json(
        json!({"ok":true,"timezone":value.timezone,"time_format":value.time_format,"video_transcoding":policy.video_transcoding,"audio_transcoding":policy.audio_transcoding}),
    ))
}

#[cfg(test)]
mod tests {
    use super::storage::{PER_ACCOUNT, refund, throttle};

    // Attempts count in the same write that checks them; a refund gives one back, and a key's
    // window restarts once it has passed.
    #[tokio::test]
    async fn a_budget_refuses_past_its_limit_until_its_window_restarts() {
        let temp = tempfile::tempdir().unwrap();
        let db = thelxinoe_database::Database::open(temp.path().join("db")).unwrap();
        let outcomes = db
            .write("test.throttle", |c| {
                let mut outcomes = Vec::new();
                for _ in 0..10 {
                    outcomes.push(throttle(c, "account", PER_ACCOUNT)?);
                }
                outcomes.push(throttle(c, "account", PER_ACCOUNT)?);
                refund(c, "account")?;
                refund(c, "account")?;
                outcomes.push(throttle(c, "account", PER_ACCOUNT)?);
                outcomes.push(throttle(c, "other", PER_ACCOUNT)?);
                c.execute(
                    "UPDATE login_attempts SET window_start=window_start-900",
                    [],
                )?;
                outcomes.push(throttle(c, "account", PER_ACCOUNT)?);
                Ok(outcomes)
            })
            .await
            .unwrap();
        assert_eq!(&outcomes[..10], [true; 10]);
        // Refused at eleven, back under the limit after two refunds, other keys unaffected,
        // and a fresh window after fifteen minutes.
        assert_eq!(&outcomes[10..], [false, true, true, true]);
    }
}
