mod oidc;
mod passkeys;
#[path = "storage/authentication.rs"]
mod storage;

use crate::{
    AppState,
    accounts::{self, Credentials},
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::SocketAddr;
use thelxinoe_auth::{SessionAuthorization, digest, token};
use thelxinoe_core::{Capability, Principal, id, now};
use totp_rs::{Builder, Secret};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/methods", get(options))
        .route("/api/v1/me/auth", get(methods))
        .route("/api/v1/auth/verify", post(verify))
        .route("/api/v1/auth/totp", post(totp_login))
        .route("/api/v1/me/auth/totp/start", post(totp_start))
        .route("/api/v1/me/auth/totp/confirm", post(totp_confirm))
        .route("/api/v1/me/auth/totp", delete(totp_remove))
        .route("/api/v1/me/auth/password", delete(password_remove))
        .route("/api/v1/me/auth/oidc", delete(oidc_remove))
        .route("/api/v1/auth/devices", get(devices))
        .route("/api/v1/auth/devices/{id}", delete(forget))
        .route(
            "/api/v1/me/auth/client-passwords",
            get(client_passwords).post(client_create),
        )
        .route(
            "/api/v1/me/auth/client-passwords/{id}",
            delete(client_revoke),
        )
        .route("/api/v1/users/{id}/recovery", post(recover))
        .route("/api/v1/auth/recovery/info", post(recovery_info))
        .route("/api/v1/auth/recovery/enroll", post(recovery_enroll))
        .route("/api/v1/auth/desktop/start", post(desktop_start))
        .route("/api/v1/auth/desktop/info", get(desktop_info))
        .route("/api/v1/auth/desktop/approve", post(desktop_approve))
        .route("/api/v1/auth/desktop/exchange", post(desktop_exchange))
        .route("/api/v1/auth/desktop/cancel", post(desktop_cancel))
        .merge(passkeys::router())
        .merge(oidc::router())
}

fn failure(error: anyhow::Error) -> ApiError {
    match error.downcast_ref::<storage::Fault>() {
        Some(storage::Fault::Unauthorized) => ApiError::unauthorized(),
        Some(storage::Fault::Verify) => ApiError(
            StatusCode::FORBIDDEN,
            "verification_required",
            "Verify your identity to continue".into(),
        ),
        Some(storage::Fault::LastMethod) => {
            ApiError::conflict("Keep at least one working sign-in method")
        }
        Some(storage::Fault::Conflict) => {
            ApiError::conflict("This sign-in method is already configured or unavailable")
        }
        None => error.into(),
    }
}
pub(crate) async fn require_fresh(state: &AppState, p: &Principal) -> Result<()> {
    storage::fresh(&state.db, p.clone()).await.map_err(failure)
}
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            part.trim()
                .split_once('=')
                .filter(|(key, _)| *key == name)
                .map(|(_, value)| value.to_owned())
        })
}
fn persistent_cookie(name: &str, raw: &str, secure: bool) -> String {
    format!(
        "{name}={raw}; HttpOnly; SameSite=Strict; Path=/; Max-Age=34560000{}",
        if secure { "; Secure" } else { "" }
    )
}
pub(crate) fn session_cookie(raw: &str, secure: bool) -> String {
    persistent_cookie("thelxinoe_session", raw, secure)
}
fn label(value: &str) -> Result<String> {
    let v = value.trim();
    if v.is_empty() || v.chars().count() > 100 {
        return Err(ApiError::bad("Enter a name of 1–100 characters"));
    }
    Ok(v.to_owned())
}
fn verified(
    account: &storage::Account,
    verified_at: i64,
    remembered_device_id: Option<String>,
) -> SessionAuthorization {
    SessionAuthorization::Verified {
        user_id: account.id.clone(),
        auth_version: account.version,
        verified_at,
        remembered_device_id,
    }
}
async fn options(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let provider = storage::provider(&state.db).await?;
    Ok(Json(
        json!({"canonical_url":context.origin,"passkeys":passkeys::webauthn(&state,&context.origin).is_ok(),"oidc":provider.as_ref().map(|p|json!({"label":p.label})),"server_id":state.server_id.as_str()}),
    ))
}
async fn methods(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<storage::Methods>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(storage::methods(&state.db, p).await.map_err(failure)?))
}

#[derive(Serialize, Deserialize)]
struct LoginDetails {
    transport: Option<String>,
    device_name: Option<String>,
}
impl LoginDetails {
    fn credentials(&self) -> Credentials {
        Credentials {
            username: String::new(),
            password: String::new(),
            transport: self.transport.clone(),
            device_name: self.device_name.clone(),
            remember_token: None,
        }
    }
}
async fn save_attempt<T: Serialize>(
    state: &AppState,
    kind: &str,
    account: Option<&storage::Account>,
    p: Option<&Principal>,
    binding: Option<&str>,
    payload: &T,
    fresh: bool,
) -> Result<String> {
    let raw = token();
    let hash = digest(&raw);
    let payload = state.secrets.encrypt(
        &format!("auth-attempt:{hash}"),
        &serde_json::to_vec(payload).map_err(anyhow::Error::from)?,
    )?;
    storage::attempt_put(
        &state.db,
        hash,
        storage::Attempt {
            kind: kind.into(),
            user: account.map(|a| a.id.clone()),
            version: account.map(|a| a.version),
            session: p.map(|p| p.session_id.clone()),
            binding: binding.map(digest),
            payload,
        },
        p.cloned(),
        fresh,
    )
    .await
    .map_err(failure)?;
    Ok(raw)
}
fn attempt_payload<T: serde::de::DeserializeOwned>(
    state: &AppState,
    raw: &str,
    a: &storage::Attempt,
) -> Result<T> {
    Ok(serde_json::from_slice(
        &state
            .secrets
            .decrypt(&format!("auth-attempt:{}", digest(raw)), &a.payload)?,
    )
    .map_err(anyhow::Error::from)?)
}
pub(crate) async fn password_login(
    state: &AppState,
    authorization: SessionAuthorization,
    c: &Credentials,
    headers: &HeaderMap,
    secure: bool,
) -> Result<Response> {
    let account = storage::account(&state.db, authorization.user_id().into(), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if let SessionAuthorization::Password { expected_hash, .. } = &authorization
        && account.password.as_ref() != Some(expected_hash)
    {
        return Err(ApiError::unauthorized());
    }
    if account.totp.as_ref().is_some_and(|(_, enabled)| *enabled) {
        let trust = c
            .remember_token
            .clone()
            .or_else(|| cookie_value(headers, "thelxinoe_remember"));
        if let Some(raw) = trust.filter(|t| t.len() == 64)
            && let Some(device) =
                storage::remembered(&state.db, account.id.clone(), digest(&raw)).await?
        {
            let mut response =
                accounts::respond_session(state, verified(&account, 0, Some(device)), c, secure)
                    .await?;
            if c.transport.as_deref() != Some("device") {
                response.headers_mut().append(
                    header::SET_COOKIE,
                    persistent_cookie("thelxinoe_remember", &raw, secure)
                        .parse()
                        .unwrap(),
                );
            }
            return Ok(response);
        }
        let details = LoginDetails {
            transport: c.transport.clone(),
            device_name: c.device_name.clone(),
        };
        if !matches!(
            details.transport.as_deref().unwrap_or("web"),
            "web" | "device"
        ) {
            return Err(ApiError::bad("Unsupported transport"));
        }
        let attempt =
            save_attempt(state, "totp", Some(&account), None, None, &details, false).await?;
        let mut response = Json(json!({"totp_required":true,"attempt":attempt})).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        Ok(response)
    } else {
        accounts::respond_session(state, authorization, c, secure).await
    }
}
fn totp(state: &AppState, account: &storage::Account) -> Result<totp_rs::Totp> {
    let encrypted = &account.totp.as_ref().ok_or_else(ApiError::unauthorized)?.0;
    let secret = state
        .secrets
        .decrypt(&format!("auth-totp:{}", account.id), encrypted)?;
    Builder::new()
        .with_secret(secret)
        .with_account_name(account.username.clone())
        .with_issuer(Some("Thelxinoe"))
        .build()
        .map_err(|_| ApiError::bad("Authenticator is unavailable"))
}
fn totp_step(state: &AppState, account: &storage::Account, code: &str) -> Result<u64> {
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ApiError::bad("Enter the six-digit authentication code"));
    }
    totp(state, account)?
        .check(code, now() as u64)
        .ok_or_else(|| ApiError::bad("Authentication code is incorrect or expired"))
}
#[derive(Deserialize)]
struct TotpLogin {
    attempt: String,
    code: String,
    #[serde(default)]
    remember_device: bool,
}
async fn totp_login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<TotpLogin>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    accounts::allow_password_attempt(&state, format!("totp:{}", context.address)).await?;
    let hash = digest(&input.attempt);
    let a = storage::attempt(&state.db, hash.clone(), "totp".into(), false)
        .await
        .map_err(failure)?;
    let account = storage::account(
        &state.db,
        a.user.clone().ok_or_else(ApiError::unauthorized)?,
        false,
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    let step = match totp_step(&state, &account, &input.code) {
        Ok(step) => step,
        Err(e) => {
            storage::failed_attempt(&state.db, hash).await?;
            return Err(e);
        }
    };
    let details: LoginDetails = attempt_payload(&state, &input.attempt, &a)?;
    let remember = input.remember_device.then(token);
    let remembered = remember.as_ref().map(|raw| storage::Remember {
        id: id(),
        hash: digest(raw),
        name: details.device_name.clone().unwrap_or_else(|| {
            if details.transport.as_deref() == Some("device") {
                "Desktop"
            } else {
                "Browser"
            }
            .into()
        }),
    });
    let authorization = storage::finish_totp(
        &state.db,
        hash,
        account.id,
        account.version,
        step,
        remembered,
    )
    .await
    .map_err(failure)?;
    let mut response = accounts::respond_session(
        &state,
        authorization,
        &details.credentials(),
        context.secure,
    )
    .await?;
    if let Some(raw) = remember {
        if details.transport.as_deref() == Some("device") {
            // The native transport saves and strips this credential before returning to the renderer.
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .map_err(anyhow::Error::from)?;
            let mut value: Value = serde_json::from_slice(&bytes).map_err(anyhow::Error::from)?;
            value["remember_token"] = json!(raw);
            response = Json(value).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        } else {
            response.headers_mut().append(
                header::SET_COOKIE,
                persistent_cookie("thelxinoe_remember", &raw, context.secure)
                    .parse()
                    .unwrap(),
            );
        }
    }
    Ok(response)
}
#[derive(Deserialize)]
struct Proof {
    #[serde(default)]
    password: String,
    #[serde(default)]
    code: String,
}
async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Proof>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    accounts::allow_password_attempt(&state, format!("verify:{}", p.user.id)).await?;
    let account = storage::account(&state.db, p.user.id.clone(), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let step = if account.totp.as_ref().is_some_and(|(_, enabled)| *enabled) {
        Some(totp_step(&state, &account, &input.code)?)
    } else {
        let hash = account
            .password
            .clone()
            .ok_or_else(|| ApiError::bad("Verify with a passkey or your identity provider"))?;
        let slot = state
            .password_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(anyhow::Error::from)?;
        if input.password.len() > 256
            || !thelxinoe_auth::verify_password_with_permit(input.password, hash, slot).await?
        {
            return Err(ApiError::bad("Password is incorrect"));
        }
        None
    };
    storage::verify(&state.db, p, account.version, step)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"verified":true})))
}
async fn totp_start(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let secret = Secret::generate();
    let otp = Builder::new()
        .with_secret(secret.clone())
        .with_account_name(p.user.username.clone())
        .with_issuer(Some("Thelxinoe"))
        .build()
        .map_err(anyhow::Error::from)?;
    let encrypted = state
        .secrets
        .encrypt(&format!("auth-totp:{}", p.user.id), secret.as_ref())?;
    storage::totp_start(&state.db, p, encrypted)
        .await
        .map_err(failure)?;
    Ok(Json(
        json!({"secret":secret.to_base32(),"uri":otp.to_url().map_err(anyhow::Error::from)?}),
    ))
}
#[derive(Deserialize)]
struct Code {
    code: String,
}
async fn totp_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Code>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    accounts::allow_password_attempt(&state, format!("totp-enroll:{}", p.user.id)).await?;
    let account = storage::account(&state.db, p.user.id.clone(), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let step = totp_step(&state, &account, &input.code)?;
    storage::totp_confirm(&state.db, p, account.totp.unwrap().0, step)
        .await
        .map_err(failure)?;
    state.notify_events();
    Ok(Json(json!({"saved":true})))
}
async fn remove(
    state: AppState,
    headers: HeaderMap,
    kind: &str,
    key: String,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::remove_method(&state.db, p, kind.into(), key)
        .await
        .map_err(failure)?;
    state.notify_events();
    Ok(Json(json!({"saved":true})))
}
async fn totp_remove(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    remove(state, headers, "totp", String::new()).await
}
async fn password_remove(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    remove(state, headers, "password", String::new()).await
}
async fn oidc_remove(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    remove(state, headers, "oidc", String::new()).await
}
async fn devices(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(
        json!({"items":storage::devices(&state.db,p,false).await.map_err(failure)?}),
    ))
}
async fn client_passwords(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(
        json!({"items":storage::devices(&state.db,p,true).await.map_err(failure)?}),
    ))
}
async fn forget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::forget(&state.db, p, key, false)
        .await
        .map_err(failure)?;
    state.notify_events();
    Ok(Json(json!({"saved":true})))
}
async fn client_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::forget(&state.db, p, key, true)
        .await
        .map_err(failure)?;
    state.notify_events();
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Named {
    name: String,
}
async fn client_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Named>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let name = label(&input.name)?;
    let raw = token();
    let key = id();
    storage::client_create(&state.db, p, key.clone(), digest(&raw), name)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"id":key,"password":raw})))
}
pub(crate) async fn client_credential(
    state: &AppState,
    username: &str,
    password: &str,
) -> Result<Option<SessionAuthorization>> {
    if password.len() != 64 {
        return Ok(None);
    }
    Ok(storage::client_password(&state.db, username.into(), digest(password)).await?)
}
#[derive(Deserialize)]
struct RecoveryToken {
    token: String,
}
#[derive(Deserialize)]
struct RecoveryEnrollment {
    token: String,
    password: String,
}
async fn recover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    let raw = token();
    storage::recovery(&state.db, Some(p), user, false, digest(&raw))
        .await
        .map_err(failure)?;
    state.notify_events();
    let base = state
        .config
        .public_url
        .as_ref()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    Ok(Json(
        json!({"url":format!("{base}/#recovery={raw}"),"expires_in":600}),
    ))
}
async fn recovery_info(
    State(state): State<AppState>,
    Json(input): Json<RecoveryToken>,
) -> Result<Json<Value>> {
    let account = storage::recovery_account(&state.db, digest(&input.token))
        .await
        .map_err(failure)?;
    Ok(Json(json!({"username":account.username})))
}
async fn recovery_enroll(
    State(state): State<AppState>,
    Json(input): Json<RecoveryEnrollment>,
) -> Result<Json<Value>> {
    let account = storage::recovery_account(&state.db, digest(&input.token))
        .await
        .map_err(failure)?;
    accounts::allow_password_attempt(&state, format!("recovery:{}", account.id)).await?;
    thelxinoe_auth::validate_credentials(&account.username, &input.password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let slot = state
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = thelxinoe_auth::password_hash_with_permit(input.password, slot).await?;
    storage::recovery_enroll(&state.db, digest(&input.token), Some(hash), None)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"saved":true})))
}
pub async fn recover_from_host(
    config: crate::config::Config,
    username: String,
) -> anyhow::Result<String> {
    let base = config
        .public_url
        .as_ref()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|| format!("http://{}", config.bind));
    let db = thelxinoe_database::Database::open(config.state.join("thelxinoe.sqlite3"))?;
    let raw = token();
    storage::recovery(&db, None, username, true, digest(&raw)).await?;
    db.shutdown().await?;
    Ok(format!("{base}/#recovery={raw}"))
}
async fn desktop_start(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let context = security::request_context(&state.config, &headers, peer)?;
    accounts::allow_password_attempt(&state, format!("desktop:{}", context.address)).await?;
    let key = token();
    let raw = token();
    let name = label(input["device_name"].as_str().unwrap_or("Desktop"))?;
    let target = if input["purpose"].as_str() == Some("verify") {
        Some(security::principal(&state, &headers).await?)
    } else {
        None
    };
    storage::desktop_start(&state.db, key.clone(), digest(&raw), name, target)
        .await
        .map_err(failure)?;
    Ok(Json(
        json!({"request":key,"secret":raw,"url":format!("{}/?desktop={key}",context.origin),"server_id":state.server_id.as_str(),"expires_in":300}),
    ))
}
#[derive(Deserialize)]
struct DesktopRequest {
    request: String,
    #[serde(default)]
    secret: String,
}
async fn desktop_info(
    State(state): State<AppState>,
    Query(input): Query<DesktopRequest>,
) -> Result<Json<Value>> {
    let info = storage::desktop_info(&state.db, input.request)
        .await
        .map_err(failure)?;
    Ok(Json(
        json!({"name":info.name,"verifying":info.verifying,"server_id":state.server_id.as_str()}),
    ))
}
async fn desktop_approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DesktopRequest>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::desktop_approve(&state.db, p, input.request)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"approved":true})))
}
async fn desktop_exchange(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<DesktopRequest>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let result = storage::desktop_exchange(
        &state.db,
        input.request,
        digest(&input.secret),
        false,
        security::principal(&state, &headers).await.ok(),
    )
    .await
    .map_err(failure)?;
    let (authorization, name) = match result {
        None => return Ok((StatusCode::ACCEPTED, Json(json!({"pending":true}))).into_response()),
        Some(storage::DesktopExchange::Verified) => {
            return Ok(Json(json!({"verified":true})).into_response());
        }
        Some(storage::DesktopExchange::SignIn(auth, name)) => (auth, name),
    };
    accounts::respond_session(
        &state,
        authorization,
        &Credentials {
            username: String::new(),
            password: String::new(),
            transport: Some("device".into()),
            device_name: Some(name),
            remember_token: None,
        },
        context.secure,
    )
    .await
}
async fn desktop_cancel(
    State(state): State<AppState>,
    Json(input): Json<DesktopRequest>,
) -> Result<Json<Value>> {
    storage::desktop_exchange(&state.db, input.request, digest(&input.secret), true, None)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"canceled":true})))
}
pub(crate) async fn revoke_session(state: &AppState, p: Principal, key: String) -> Result<()> {
    storage::revoke_session(&state.db, p, key)
        .await
        .map_err(failure)
}
pub(crate) async fn set_password(state: &AppState, p: Principal, hash: String) -> Result<()> {
    storage::password(&state.db, p, hash).await.map_err(failure)
}
pub(crate) async fn password_change_proof(
    state: &AppState,
    p: &Principal,
    password: String,
) -> Result<()> {
    match storage::fresh(&state.db, p.clone()).await {
        Ok(()) => return Ok(()),
        Err(error)
            if matches!(
                error.downcast_ref::<storage::Fault>(),
                Some(storage::Fault::Verify)
            ) => {}
        Err(error) => return Err(failure(error)),
    }
    let account = storage::account(&state.db, p.user.id.clone(), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if account.totp.as_ref().is_some_and(|(_, enabled)| *enabled) {
        return require_fresh(state, p).await;
    }
    if let Some(hash) = account.password {
        accounts::allow_password_attempt(state, format!("password:{}", p.user.id)).await?;
        let slot = state
            .password_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(anyhow::Error::from)?;
        if password.len() > 256
            || !thelxinoe_auth::verify_password_with_permit(password, hash, slot).await?
        {
            return Err(ApiError::bad("Current password is incorrect"));
        }
        storage::verify(&state.db, p.clone(), account.version, None)
            .await
            .map_err(failure)?;
    }
    require_fresh(state, p).await
}
pub(crate) fn authorize_admin(c: &rusqlite::Connection, p: &Principal) -> anyhow::Result<()> {
    storage::authorize_admin(c, p)
}
