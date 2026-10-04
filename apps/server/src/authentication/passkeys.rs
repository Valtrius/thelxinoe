use super::*;
use webauthn_rs::prelude::*;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/passkey/start", post(start))
        .route("/api/v1/auth/passkey/finish", post(finish))
        .route(
            "/api/v1/me/auth/passkeys/register/start",
            post(register_start),
        )
        .route(
            "/api/v1/me/auth/passkeys/register/finish",
            post(register_finish),
        )
        .route("/api/v1/me/auth/passkeys/{id}", delete(remove_passkey))
}
pub(super) fn webauthn(state: &AppState, origin: &str) -> Result<Webauthn> {
    let url = state
        .config
        .public_url
        .clone()
        .unwrap_or(url::Url::parse(origin).map_err(anyhow::Error::from)?);
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::bad("Configure an authentication hostname"))?;
    if host.parse::<std::net::IpAddr>().is_ok()
        || !(url.scheme() == "https" || (url.scheme() == "http" && host == "localhost"))
    {
        return Err(ApiError::bad(
            "Passkeys require HTTPS and a hostname, or localhost",
        ));
    }
    WebauthnBuilder::new(host, &url)
        .and_then(|b| b.rp_name("Thelxinoe").build())
        .map_err(|_| ApiError::bad("Passkeys are unavailable at this address"))
}
fn check_origin(state: &AppState, headers: &HeaderMap, origin: &str) -> Result<Webauthn> {
    let webauthn = webauthn(state, origin)?;
    let expected = state
        .config
        .public_url
        .as_ref()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|| origin.into());
    if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) != Some(expected.as_str()) {
        return Err(ApiError::bad(
            "Use the authentication hostname for passkeys",
        ));
    }
    Ok(webauthn)
}
async fn credentials(state: &AppState, user: &str) -> Result<Vec<(String, String, Passkey)>> {
    storage::passkeys(&state.db, user.into())
        .await?
        .into_iter()
        .map(|(id, json)| {
            Ok((
                id,
                json.clone(),
                serde_json::from_str(&json).map_err(anyhow::Error::from)?,
            ))
        })
        .collect()
}
#[derive(Deserialize)]
struct Start {
    #[serde(default)]
    username: String,
    #[serde(default)]
    purpose: String,
}
#[derive(Serialize, Deserialize)]
struct Authentication {
    state: PasskeyAuthentication,
    purpose: String,
}
async fn start(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Start>,
) -> Result<Json<Value>> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let webauthn = check_origin(&state, &headers, &context.origin)?;
    accounts::allow_password_attempt(&state, format!("passkey:{}", context.address)).await?;
    let p = if input.purpose == "verify" {
        Some(security::principal(&state, &headers).await?)
    } else {
        None
    };
    let account = storage::account(
        &state.db,
        p.as_ref()
            .map(|p| p.user.id.clone())
            .unwrap_or(input.username),
        p.is_none(),
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    let passkeys = credentials(&state, &account.id).await?;
    let (options, authentication) = webauthn
        .start_passkey_authentication(&passkeys.into_iter().map(|(_, _, p)| p).collect::<Vec<_>>())
        .map_err(|_| ApiError::bad("No passkey is available for this account"))?;
    let attempt = save_attempt(
        &state,
        "passkey",
        Some(&account),
        p.as_ref(),
        None,
        &Authentication {
            state: authentication,
            purpose: if p.is_some() { "verify" } else { "login" }.into(),
        },
        false,
    )
    .await?;
    Ok(Json(json!({"attempt":attempt,"options":options})))
}
#[derive(Deserialize)]
struct Finish {
    attempt: String,
    credential: PublicKeyCredential,
}
async fn finish(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Finish>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let webauthn = check_origin(&state, &headers, &context.origin)?;
    let a = storage::attempt(&state.db, digest(&input.attempt), "passkey".into(), true)
        .await
        .map_err(failure)?;
    let data: Authentication = attempt_payload(&state, &input.attempt, &a)?;
    let authentication = webauthn
        .finish_passkey_authentication(&input.credential, &data.state)
        .map_err(|_| ApiError::bad("Passkey verification failed"))?;
    if !authentication.user_verified() {
        return Err(ApiError::unauthorized());
    }
    let user = a.user.ok_or_else(ApiError::unauthorized)?;
    let account = storage::account(&state.db, user.clone(), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if a.version != Some(account.version) {
        return Err(ApiError::unauthorized());
    }
    let (key, previous, mut passkey) = credentials(&state, &user)
        .await?
        .into_iter()
        .find(|(_, _, p)| p.cred_id() == authentication.cred_id())
        .ok_or_else(ApiError::unauthorized)?;
    passkey.update_credential(&authentication);
    storage::passkey_used(
        &state.db,
        user,
        account.version,
        key,
        previous,
        serde_json::to_string(&passkey).map_err(anyhow::Error::from)?,
    )
    .await
    .map_err(failure)?;
    if data.purpose == "verify" {
        let p = security::principal(&state, &headers).await?;
        if a.session.as_deref() != Some(&p.session_id) || p.user.id != account.id {
            return Err(ApiError::unauthorized());
        }
        storage::verify(&state.db, p, account.version, None)
            .await
            .map_err(failure)?;
        Ok(Json(json!({"verified":true})).into_response())
    } else {
        accounts::respond_session(
            &state,
            verified(&account, now(), None),
            &LoginDetails {
                transport: None,
                device_name: None,
            }
            .credentials(),
            context.secure,
        )
        .await
    }
}
#[derive(Deserialize)]
struct Register {
    #[serde(default)]
    name: String,
    #[serde(default)]
    recovery_token: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Registration {
    state: PasskeyRegistration,
    name: String,
    recovery: Option<String>,
}
async fn register_start(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Register>,
) -> Result<Json<Value>> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let webauthn = check_origin(&state, &headers, &context.origin)?;
    let (account, p) = if let Some(recovery) = &input.recovery_token {
        (
            storage::recovery_account(&state.db, digest(recovery))
                .await
                .map_err(failure)?,
            None,
        )
    } else {
        let p = security::principal(&state, &headers).await?;
        require_fresh(&state, &p).await?;
        (
            storage::account(&state.db, p.user.id.clone(), false)
                .await?
                .ok_or_else(ApiError::unauthorized)?,
            Some(p),
        )
    };
    let passkeys = credentials(&state, &account.id).await?;
    let (options, registration) = webauthn
        .start_passkey_registration(
            Uuid::parse_str(&account.id).map_err(anyhow::Error::from)?,
            &account.username,
            &account.username,
            Some(
                passkeys
                    .into_iter()
                    .map(|(_, _, p)| p.cred_id().clone())
                    .collect(),
            ),
        )
        .map_err(|_| ApiError::bad("Cannot create a passkey"))?;
    let name = if input.name.trim().is_empty() {
        "Passkey".into()
    } else {
        label(&input.name)?
    };
    let attempt = save_attempt(
        &state,
        "passkey-register",
        Some(&account),
        p.as_ref(),
        None,
        &Registration {
            state: registration,
            name,
            recovery: input.recovery_token,
        },
        p.is_some(),
    )
    .await?;
    Ok(Json(json!({"attempt":attempt,"options":options})))
}
#[derive(Deserialize)]
struct RegisterFinish {
    attempt: String,
    credential: RegisterPublicKeyCredential,
}
async fn register_finish(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<RegisterFinish>,
) -> Result<Json<Value>> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let webauthn = check_origin(&state, &headers, &context.origin)?;
    let a = storage::attempt(
        &state.db,
        digest(&input.attempt),
        "passkey-register".into(),
        true,
    )
    .await
    .map_err(failure)?;
    let data: Registration = attempt_payload(&state, &input.attempt, &a)?;
    let passkey = webauthn
        .finish_passkey_registration(&input.credential, &data.state)
        .map_err(|_| ApiError::bad("Passkey registration failed"))?;
    let key = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        passkey.cred_id(),
    );
    let credential = serde_json::to_string(&passkey).map_err(anyhow::Error::from)?;
    if let Some(raw) = data.recovery {
        storage::recovery_enroll(&state.db, digest(&raw), None, Some((key, credential)))
            .await
            .map_err(failure)?;
    } else {
        let p = security::principal(&state, &headers).await?;
        if a.session.as_deref() != Some(&p.session_id) || a.user.as_deref() != Some(&p.user.id) {
            return Err(ApiError::unauthorized());
        }
        storage::passkey_add(
            &state.db,
            p,
            a.version.ok_or_else(ApiError::unauthorized)?,
            key,
            data.name,
            credential,
        )
        .await
        .map_err(failure)?;
    }
    Ok(Json(json!({"saved":true})))
}
async fn remove_passkey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    remove(state, headers, "passkey", key).await
}
