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
    SessionAuthorization, issue_session, password_hash_with_permit, user_row, validate_credentials,
    verify_password_with_permit,
};
use thelxinoe_core::{Capability, Role, id, now};

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
}

pub async fn setup_status(State(state): State<AppState>) -> Result<Json<Value>> {
    let count = storage::setup_status(&state.db).await?;
    Ok(Json(
        json!({"setup_required":count==0,"version":thelxinoe_core::VERSION}),
    ))
}
async fn respond_session(
    state: &AppState,
    authorization: SessionAuthorization,
    c: &Credentials,
    secure: bool,
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
    let mut response =
        Json(json!({"user":user,"token":if transport=="device"{Some(&raw)}else{None}}))
            .into_response();
    if transport == "web" {
        response.headers_mut().insert(
            header::SET_COOKIE,
            security::cookie(&raw, secure).parse().unwrap(),
        );
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}
pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    if storage::setup_status(&state.db).await? != 0 {
        return Err(ApiError::conflict("Setup is already complete"));
    }
    if !matches!(c.transport.as_deref().unwrap_or("web"), "web" | "device") {
        return Err(ApiError::bad("Unsupported transport"));
    }
    validate_credentials(&c.username, &c.password).map_err(|e| ApiError::bad(e.to_string()))?;
    allow_password_attempt(&state, format!("setup:{}", context.address)).await?;
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
    let hash = password_hash_with_permit(c.password.clone(), slot).await?;
    let user_id = id();
    let uid = user_id.clone();
    let username = c.username.clone();
    let created = storage::setup(&state.db, hash.clone(), uid, username).await?;
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
    let user = check_credentials(&state, context.address, &c.username, &c.password).await?;
    respond_session(&state, user, &c, context.secure).await
}
pub(crate) async fn check_credentials(
    state: &AppState,
    address: std::net::IpAddr,
    username: &str,
    password: &str,
) -> Result<SessionAuthorization> {
    allow_password_attempt(state, address.to_string()).await?;
    if password.len() > 256 || username.len() > 64 {
        return Err(ApiError::unauthorized());
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
    let valid = verify_password_with_permit(password.to_owned(), hash, slot).await?;
    if !valid || record.is_none() {
        return Err(ApiError::unauthorized());
    }
    let (user_id, expected_hash) = record.unwrap();
    Ok(SessionAuthorization::Password {
        user_id,
        expected_hash,
    })
}
async fn allow_password_attempt(state: &AppState, address: String) -> Result<()> {
    let allowed = storage::allow_password_attempt(&state.db, address).await?;
    if !allowed {
        return Err(ApiError(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts. Try again in 15 minutes.".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordChange>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    validate_credentials(&principal.user.username, &input.new_password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    if input.current_password.len() > 256 {
        return Err(ApiError::bad("Current password is incorrect"));
    }
    allow_password_attempt(&state, format!("password:{}", principal.user.id)).await?;
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let user_id = principal.user.id.clone();
    let previous_hash = storage::change_password_read_users(&state.db, user_id).await?;
    if !verify_password_with_permit(input.current_password, previous_hash.clone(), slot).await? {
        return Err(ApiError::bad("Current password is incorrect"));
    }
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = password_hash_with_permit(input.new_password, slot).await?;
    let user_id = principal.user.id.clone();
    let changed =
        storage::change_password_write_users(&state.db, &principal, previous_hash, hash, user_id)
            .await?;
    if !changed {
        return Err(ApiError::conflict(
            "Your account changed. Sign in again before changing your password",
        ));
    }
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
    storage::revoke(&state.db, session, p).await?;
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
    validate_credentials(&user.username, &user.password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = password_hash_with_permit(std::mem::take(&mut user.password), slot).await?;
    let result = storage::create_user(&state.db, user, p, hash).await?;
    result
        .map(|id| Json(json!({"id":id})))
        .ok_or_else(|| ApiError::conflict("Username already exists"))
}
pub async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (timezone, time_format) = storage::settings(&state.db).await?;
    Ok(Json(
        json!({"timezone":timezone,"time_format":time_format,"public_url":state.config.public_url.as_ref().map(ToString::to_string),"trusted_proxies":state.config.trusted_proxies.iter().map(ToString::to_string).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
pub struct Settings {
    timezone: String,
    time_format: Option<String>,
}
pub async fn save_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<Settings>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    settings
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::bad("Unknown timezone"))?;
    if let Some(format) = &settings.time_format
        && !matches!(format.as_str(), "12h" | "24h")
    {
        return Err(ApiError::bad("Unknown time format"));
    }
    let timezone = settings.timezone.clone();
    let time_format = settings.time_format.clone();
    let effective_time_format = storage::save_settings(&state.db, settings, p).await?;
    state
        .emit(
            None,
            "server.settings.changed",
            json!({"timezone":timezone,"time_format":effective_time_format}),
        )
        .await?;
    Ok(Json(
        json!({"ok":true,"timezone":timezone,"time_format":time_format.unwrap_or(effective_time_format)}),
    ))
}
