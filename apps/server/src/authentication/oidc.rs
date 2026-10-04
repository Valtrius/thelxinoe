use super::*;
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/auth/oidc",
            get(configuration).post(configure).delete(disable),
        )
        .route("/api/v1/auth/oidc/start", post(start))
        .route("/api/v1/auth/oidc/callback", get(callback))
}
fn http() -> Result<openidconnect::reqwest::Client> {
    openidconnect::reqwest::Client::builder()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(anyhow::Error::from)
        .map_err(Into::into)
}
fn endpoint(value: &str) -> Result<url::Url> {
    let u = url::Url::parse(value).map_err(|_| ApiError::bad("Enter an HTTP(S) URL"))?;
    let local = u.host_str().is_some_and(|h| {
        h == "localhost"
            || h.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(u.scheme() == "https" || (u.scheme() == "http" && local))
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err(ApiError::bad(
            "OIDC requires HTTPS, except for loopback development",
        ));
    }
    Ok(u)
}
async fn metadata(discovery: &str, expected: Option<&str>) -> Result<CoreProviderMetadata> {
    endpoint(discovery)?;
    let client = http()?;
    let response = client
        .get(discovery)
        .send()
        .await
        .map_err(|_| ApiError::bad("Cannot reach the OIDC discovery URL"))?;
    if !response.status().is_success() {
        return Err(ApiError::bad("OIDC discovery failed"));
    }
    let bytes = response.bytes().await.map_err(anyhow::Error::from)?;
    if bytes.len() > 1024 * 1024 {
        return Err(ApiError::bad("OIDC discovery is too large"));
    }
    let metadata: CoreProviderMetadata = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("Invalid OIDC discovery document"))?;
    if expected.is_some_and(|iss| iss != metadata.issuer().as_str()) {
        return Err(ApiError::bad(
            "OIDC issuer changed; configure the provider again",
        ));
    }
    endpoint(metadata.issuer().as_str())?;
    endpoint(metadata.authorization_endpoint().as_str())?;
    endpoint(metadata.jwks_uri().as_str())?;
    let token = metadata
        .token_endpoint()
        .ok_or_else(|| ApiError::bad("OIDC provider has no token endpoint"))?;
    endpoint(token.as_str())?;
    let response = client
        .get(metadata.jwks_uri().as_str())
        .send()
        .await
        .map_err(|_| ApiError::bad("Cannot reach the OIDC signing keys"))?;
    if !response.status().is_success() {
        return Err(ApiError::bad("OIDC signing keys are unavailable"));
    }
    let bytes = response.bytes().await.map_err(anyhow::Error::from)?;
    if bytes.len() > 1024 * 1024 {
        return Err(ApiError::bad("OIDC signing keys are too large"));
    }
    let keys: CoreJsonWebKeySet =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid OIDC signing keys"))?;
    Ok(metadata.set_jwks(keys))
}
async fn configuration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let provider = storage::provider(&state.db).await?;
    let redirect = state.config.public_url.as_ref().map(|u| {
        format!(
            "{}/api/v1/auth/oidc/callback",
            u.origin().ascii_serialization()
        )
    });
    Ok(Json(json!({"provider":provider,"redirect_uri":redirect})))
}
#[derive(Deserialize)]
struct Configuration {
    discovery_url: String,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    label: String,
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Configuration>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    require_fresh(&state, &p).await?;
    let public = state.config.public_url.as_ref().ok_or_else(|| {
        ApiError::bad("Set THELXINOE_PUBLIC_URL to the authentication origin first")
    })?;
    endpoint(public.as_str())?;
    if input.client_id.is_empty() || input.client_id.len() > 512 {
        return Err(ApiError::bad("Enter the OIDC client ID"));
    }
    let metadata = metadata(&input.discovery_url, None).await?;
    let previous = storage::provider(&state.db).await?;
    let secret = match input.client_secret {
        Some(s) if !s.is_empty() && s.len() <= 4096 => state
            .secrets
            .encrypt("auth-oidc-client-secret", s.as_bytes())?,
        None if previous.as_ref().is_some_and(|v| {
            v.client_id == input.client_id && v.issuer == metadata.issuer().as_str()
        }) =>
        {
            previous.unwrap().secret
        }
        _ => return Err(ApiError::bad("Enter the OIDC client secret")),
    };
    let provider = storage::Provider {
        discovery_url: input.discovery_url,
        issuer: metadata.issuer().as_str().into(),
        client_id: input.client_id,
        secret,
        label: label(&input.label)?,
        version: id(),
    };
    storage::provider_save(&state.db, p, Some(provider))
        .await
        .map_err(failure)?;
    Ok(Json(json!({"saved":true})))
}
async fn disable(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    storage::provider_save(&state.db, p, None)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Start {
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    return_to: String,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    purpose: String,
    return_to: String,
    nonce: String,
    pkce: String,
    provider_version: String,
    redirect: String,
}
fn return_path(value: &str) -> String {
    if value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains(['\\', '\r', '\n'])
        && value.len() < 2048
    {
        value.into()
    } else {
        "/".into()
    }
}
async fn start(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<Start>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    accounts::allow_password_attempt(&state, format!("oidc:{}", context.address)).await?;
    let purpose = if input.purpose.is_empty() {
        "login"
    } else {
        input.purpose.as_str()
    };
    if !matches!(purpose, "login" | "link" | "verify") {
        return Err(ApiError::bad("Unsupported OIDC action"));
    }
    let p = if purpose == "login" {
        None
    } else {
        Some(security::principal(&state, &headers).await?)
    };
    if purpose == "link" {
        require_fresh(&state, p.as_ref().unwrap()).await?;
    }
    let account = if let Some(p) = &p {
        storage::account(&state.db, p.user.id.clone(), false).await?
    } else {
        None
    };
    let provider = storage::provider(&state.db)
        .await?
        .ok_or_else(|| ApiError::bad("OIDC is not configured"))?;
    let metadata = metadata(&provider.discovery_url, Some(&provider.issuer)).await?;
    let redirect = format!("{}/api/v1/auth/oidc/callback", context.origin);
    endpoint(&redirect)?;
    let secret = String::from_utf8(
        state
            .secrets
            .decrypt("auth-oidc-client-secret", &provider.secret)?,
    )
    .map_err(anyhow::Error::from)?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(provider.client_id),
        Some(ClientSecret::new(secret)),
    )
    .set_redirect_uri(RedirectUrl::new(redirect.clone()).map_err(anyhow::Error::from)?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let nonce = Nonce::new_random();
    let binding = token();
    let pending = Pending {
        purpose: purpose.into(),
        return_to: return_path(&input.return_to),
        nonce: nonce.secret().clone(),
        pkce: verifier.secret().clone(),
        provider_version: provider.version,
        redirect,
    };
    let raw = save_attempt(
        &state,
        "oidc",
        account.as_ref(),
        p.as_ref(),
        Some(&binding),
        &pending,
        purpose == "link",
    )
    .await?;
    let (url, _, _) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(raw.clone()),
            move || nonce,
        )
        .set_pkce_challenge(challenge)
        .set_max_age(std::time::Duration::ZERO)
        .url();
    let mut response = Json(json!({"url":url.to_string()})).into_response();
    // Only this one-use binding crosses the identity-provider redirect; the app session remains Strict.
    response.headers_mut().append(header::SET_COOKIE,format!("thelxinoe_oidc={binding}; HttpOnly; SameSite=Lax; Path=/api/v1/auth/oidc; Max-Age=300{}",if context.secure{"; Secure"}else{""}).parse().unwrap());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}
#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn complete(
    state: &AppState,
    headers: &HeaderMap,
    input: &Callback,
    secure: bool,
) -> Result<Response> {
    let a = storage::attempt(&state.db, digest(&input.state), "oidc".into(), false)
        .await
        .map_err(failure)?;
    let binding = cookie_value(headers, "thelxinoe_oidc").ok_or_else(ApiError::unauthorized)?;
    if a.binding.as_deref() != Some(&digest(&binding)) {
        return Err(ApiError::unauthorized());
    }
    let a = storage::attempt(&state.db, digest(&input.state), "oidc".into(), true)
        .await
        .map_err(failure)?;
    if input.error.is_some() {
        return Err(ApiError::bad("Identity-provider sign-in was canceled"));
    }
    let pending: Pending = attempt_payload(state, &input.state, &a)?;
    let provider = storage::provider(&state.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if provider.version != pending.provider_version {
        return Err(ApiError::unauthorized());
    }
    let metadata = metadata(&provider.discovery_url, Some(&provider.issuer)).await?;
    let secret = String::from_utf8(
        state
            .secrets
            .decrypt("auth-oidc-client-secret", &provider.secret)?,
    )
    .map_err(anyhow::Error::from)?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(provider.client_id),
        Some(ClientSecret::new(secret)),
    )
    .set_redirect_uri(RedirectUrl::new(pending.redirect).map_err(anyhow::Error::from)?);
    let response = client
        .exchange_code(AuthorizationCode::new(
            input.code.clone().ok_or_else(ApiError::unauthorized)?,
        ))
        .map_err(|_| ApiError::bad("OIDC code exchange is unavailable"))?
        .set_pkce_verifier(PkceCodeVerifier::new(pending.pkce))
        .request_async(&http()?)
        .await
        .map_err(|_| ApiError::bad("OIDC code exchange failed"))?;
    let id_token = response
        .id_token()
        .ok_or_else(|| ApiError::bad("OIDC provider did not return an identity token"))?;
    let verifier = client.id_token_verifier();
    let claims = id_token
        .claims(&verifier, &Nonce::new(pending.nonce))
        .map_err(|_| ApiError::bad("OIDC identity verification failed"))?;
    if let Some(expected) = claims.access_token_hash() {
        let actual = AccessTokenHash::from_token(
            response.access_token(),
            id_token.signing_alg().map_err(anyhow::Error::from)?,
            id_token
                .signing_key(&verifier)
                .map_err(anyhow::Error::from)?,
        )
        .map_err(anyhow::Error::from)?;
        if &actual != expected {
            return Err(ApiError::unauthorized());
        }
    }
    let auth_time = claims
        .auth_time()
        .map(|t| t.timestamp())
        .filter(|t| *t >= now() - 300 && *t <= now() + 30)
        .ok_or_else(|| ApiError::bad("The identity provider did not confirm a fresh sign-in"))?;
    let issuer = claims.issuer().as_str().to_owned();
    let subject = claims.subject().as_str().to_owned();
    let mut result = if pending.purpose == "login" {
        let account = storage::oidc_account(&state.db, issuer, subject)
            .await?
            .ok_or_else(|| {
                ApiError::bad("This identity-provider account is not linked to a Thelxinoe account")
            })?;
        accounts::respond_session(
            state,
            verified(&account, auth_time, None),
            &LoginDetails {
                transport: None,
                device_name: None,
            }
            .credentials(),
            secure,
        )
        .await?
    } else {
        let user = a.user.ok_or_else(ApiError::unauthorized)?;
        let account = storage::account(&state.db, user, false)
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        if a.version != Some(account.version) {
            return Err(ApiError::unauthorized());
        }
        let p = Principal {
            user: thelxinoe_core::User {
                id: account.id.clone(),
                username: account.username.clone(),
                role: thelxinoe_core::Role::User,
                timezone: "UTC".into(),
            },
            session_id: a.session.ok_or_else(ApiError::unauthorized)?,
            transport: "web".into(),
        };
        if pending.purpose == "link" {
            storage::oidc_link(&state.db, p, account.version, issuer, subject)
                .await
                .map_err(failure)?;
        } else {
            let linked = storage::oidc_account(&state.db, issuer, subject)
                .await?
                .ok_or_else(ApiError::unauthorized)?;
            if linked.id != account.id {
                return Err(ApiError::bad(
                    "Use the identity-provider account linked to this user",
                ));
            }
            storage::verify(&state.db, p, account.version, None)
                .await
                .map_err(failure)?;
        }
        Json(json!({"saved":true})).into_response()
    };
    *result.status_mut() = StatusCode::SEE_OTHER;
    let mut target = url::Url::parse(&format!("https://return.invalid{}", pending.return_to))
        .map_err(anyhow::Error::from)?;
    if pending.purpose == "link" {
        target
            .query_pairs_mut()
            .append_pair("auth_notice", "oidc-linked");
    } else if pending.purpose == "verify" {
        target
            .query_pairs_mut()
            .append_pair("auth_notice", "verified");
    }
    let location = format!(
        "{}{}{}",
        target.path(),
        target.query().map(|q| format!("?{q}")).unwrap_or_default(),
        target
            .fragment()
            .map(|f| format!("#{f}"))
            .unwrap_or_default()
    );
    result.headers_mut().insert(
        header::LOCATION,
        location
            .parse()
            .map_err(|_| ApiError::bad("Invalid return path"))?,
    );
    Ok(result)
}
async fn callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> Response {
    let result = match security::request_context(&state.config, &headers, peer) {
        Ok(context) => complete(&state, &headers, &input, context.secure).await,
        Err(e) => Err(e),
    };
    let mut response = match result {
        Ok(response) => response,
        Err(error) => {
            let mut url = url::Url::parse("https://return.invalid/").unwrap();
            url.query_pairs_mut().append_pair("auth_error", &error.2);
            axum::response::Redirect::to(&format!("/?{}", url.query().unwrap())).into_response()
        }
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        "thelxinoe_oidc=; HttpOnly; SameSite=Lax; Path=/api/v1/auth/oidc; Max-Age=0"
            .parse()
            .unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}
