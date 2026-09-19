//! Browser-bound authorization. Viewer credentials never reach an extractor.
use super::{Google, bounded_response, google, quota, redirect_uri};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::now;
pub(super) const SCOPE: &str = "https://www.googleapis.com/auth/youtube.readonly";
const COOKIE: &str = "thelxinoe_youtube_oauth";
#[cfg(test)]
pub(super) mod tests;
type Client = BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;
pub(super) fn client(state: &AppState, google: Google, redirect: String) -> Result<Client> {
    Ok(BasicClient::new(ClientId::new(google.client_id))
        .set_client_secret(ClientSecret::new(google.client_secret))
        .set_auth_type(oauth2::AuthType::RequestBody)
        .set_auth_uri(
            AuthUrl::new(state.online.authorize.clone())
                .map_err(|_| ApiError::bad("Invalid authorization endpoint"))?,
        )
        .set_token_uri(
            TokenUrl::new(state.online.token.clone())
                .map_err(|_| ApiError::bad("Invalid token endpoint"))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect).map_err(|_| ApiError::bad("Invalid callback URL"))?,
        ))
}
pub async fn start(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let p = security::principal(&state, &headers).await?;
    if p.transport != "web" {
        return Err(ApiError::bad(
            "Open Thelxinoe in your browser to link your YouTube account",
        ));
    }
    let redirect = redirect_uri(&state)?;
    let google = google(&state).await?;
    let client_hash = google.hash();
    let client = client(&state, google, redirect.clone())?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf) = client
        .authorize_url(|| CsrfToken::new(thelxinoe_auth::token()))
        .add_scope(Scope::new(SCOPE.into()))
        .set_pkce_challenge(challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent select_account")
        .url();
    let hash = thelxinoe_auth::digest(csrf.secret());
    let encrypted = state
        .secrets
        .encrypt(&format!("oauth:{hash}"), verifier.secret().as_bytes())?;
    let browser = thelxinoe_auth::token();
    let browser_hash = thelxinoe_auth::digest(&browser);
    let generation = thelxinoe_core::id();
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM oauth_attempts WHERE expires_at<=?1 OR (user_id=?2 AND provider='youtube')",params![now(),p.user.id])?;
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES (?1,'youtube',?2,?3) ON CONFLICT(user_id,provider) DO UPDATE SET generation=excluded.generation",params![p.user.id,generation,now()])?;
        tx.execute("UPDATE youtube_sync SET generation=?1 WHERE user_id=?2",params![generation,p.user.id])?;
        tx.execute("INSERT INTO oauth_attempts VALUES (?1,?2,?3,'youtube',?4,?5,?6,?7,?8,?9)",params![hash,p.user.id,p.session_id,generation,browser_hash,encrypted,redirect,client_hash,now()+600])?;
        tx.commit()?;Ok(())
    }).await?;
    let mut response = Json(json!({"url":url.as_str()})).into_response();
    response.headers_mut().insert(header::SET_COOKIE,format!("{COOKIE}={browser}; HttpOnly; SameSite=Lax; Path=/api/v1/online/youtube/callback; Max-Age=600{}",if state.config.public_url.as_ref().is_some_and(|u|u.scheme()=="https"){"; Secure"}else{""}).parse().unwrap());
    Ok(response)
}
#[derive(Deserialize)]
pub struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}
struct Attempt {
    user: String,
    session: String,
    generation: String,
    verifier: Vec<u8>,
    redirect: String,
    client_hash: String,
}
async fn consume(state: &AppState, headers: &HeaderMap, csrf: &str) -> Result<Attempt> {
    if !(32..=256).contains(&csrf.len()) {
        return Err(ApiError::bad("Authorization attempt is invalid or expired"));
    }
    let browser = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .map(str::trim)
                .find_map(|v| v.strip_prefix(&format!("{COOKIE}=")))
        })
        .filter(|v| v.len() == 64)
        .ok_or_else(|| {
            ApiError::bad("Continue account linking in the browser where you started")
        })?;
    let browser_hash = thelxinoe_auth::digest(browser);
    let hash = thelxinoe_auth::digest(csrf);
    let attempt=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result=tx.query_row("SELECT a.user_id,a.session_id,a.generation,a.verifier,a.redirect_uri,a.client_hash FROM oauth_attempts a JOIN sessions s ON s.id=a.session_id JOIN online_accounts o ON o.user_id=a.user_id AND o.provider=a.provider AND o.generation=a.generation WHERE a.state_hash=?1 AND a.browser_hash=?2 AND a.expires_at>?3 AND s.expires_at>?3 AND s.user_id=a.user_id AND s.transport='web'",params![hash,browser_hash,now()],|r|Ok(Attempt{user:r.get(0)?,session:r.get(1)?,generation:r.get(2)?,verifier:r.get(3)?,redirect:r.get(4)?,client_hash:r.get(5)?})).optional()?;
        if result.is_some(){tx.execute("DELETE FROM oauth_attempts WHERE state_hash=?1",[hash])?;}
        tx.commit()?;Ok(result)
    }).await?;
    attempt
        .ok_or_else(|| ApiError::bad("Authorization attempt is invalid, expired or already used"))
}
#[derive(Serialize, Deserialize)]
pub(super) struct Credential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub scope: String,
}
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> Result<Response> {
    let result = complete(&state, &headers, input).await;
    let status = match result {
        Ok(()) => "connected",
        Err(error) => {
            // Never log provider bodies, codes, URLs or cookies.
            tracing::info!(
                status = error.0.as_u16(),
                "YouTube account linking did not complete"
            );
            "failed"
        }
    };
    let mut response = Redirect::to(&format!("/?youtube_link={status}")).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{COOKIE}=; HttpOnly; SameSite=Lax; Path=/api/v1/online/youtube/callback; Max-Age=0"
        )
        .parse()
        .unwrap(),
    );
    Ok(response)
}
async fn complete(state: &AppState, headers: &HeaderMap, input: Callback) -> Result<()> {
    let csrf = input
        .state
        .as_deref()
        .ok_or_else(|| ApiError::bad("Missing authorization state"))?;
    let attempt = consume(state, headers, csrf).await?;
    if input.error.is_some() {
        return Err(ApiError::bad("Google authorization was declined"));
    }
    let code = input
        .code
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or_else(|| ApiError::bad("Missing authorization code"))?;
    let _slot = state
        .online
        .slots
        .try_acquire()
        .map_err(|_| ApiError::conflict("Account linking is busy; try again"))?;
    let config = google(state).await?;
    if config.hash() != attempt.client_hash {
        return Err(ApiError::conflict(
            "Google application settings changed; start again",
        ));
    }
    let verifier = state.secrets.decrypt(
        &format!("oauth:{}", thelxinoe_auth::digest(csrf)),
        &attempt.verifier,
    )?;
    let verifier = String::from_utf8(verifier)
        .map_err(|_| ApiError::bad("Invalid stored authorization attempt"))?;
    let client = client(state, config, attempt.redirect.clone())?;
    let transport = |request| send(state.online.http.clone(), request);
    let token=client.exchange_code(AuthorizationCode::new(code)).set_pkce_verifier(PkceCodeVerifier::new(verifier)).request_async(&transport).await.map_err(|_|ApiError::bad("Google could not complete account linking; verify the Web application client and callback URL"))?;
    let scope = token
        .scopes()
        .map(|s| s.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| SCOPE.into());
    if !scope.split_whitespace().any(|v| v == SCOPE) {
        return Err(ApiError::bad(
            "Grant read access to YouTube to complete account linking",
        ));
    }
    let credential = Credential {
        access_token: token.access_token().secret().clone(),
        refresh_token: token.refresh_token().map(|v| v.secret().clone()),
        scope,
    };
    if credential.refresh_token.is_none() {
        return Err(ApiError::bad(
            "Google did not provide offline access; reconnect and grant access",
        ));
    }
    let expires = token
        .expires_in()
        .map(|v| v.as_secs().min(86400) as i64)
        .unwrap_or(3600);
    quota::reserve(state, &attempt.user).await?;
    let response = state
        .online
        .http
        .get(format!("{}/channels", state.online.api))
        .bearer_auth(&credential.access_token)
        .query(&[("part", "snippet"), ("mine", "true")])
        .send()
        .await
        .map_err(|_| ApiError::bad("YouTube account lookup failed"))?;
    let (status, _, bytes) = bounded_response(response)
        .await
        .map_err(|_| ApiError::bad("YouTube account lookup failed"))?;
    if !status.is_success() {
        return Err(ApiError::bad(
            "YouTube account lookup failed; enable the YouTube Data API for the Google project",
        ));
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("YouTube returned invalid account data"))?;
    let channel = data["items"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| ApiError::bad("This Google account has no YouTube channel"))?;
    let name = channel["snippet"]["title"]
        .as_str()
        .unwrap_or("YouTube account")
        .chars()
        .take(200)
        .collect::<String>();
    let external = channel["id"]
        .as_str()
        .filter(|s| s.len() <= 128)
        .ok_or_else(|| ApiError::bad("YouTube returned an invalid channel"))?
        .to_owned();
    let encrypted = state.secrets.encrypt(
        &format!("online:youtube:{}", attempt.user),
        &serde_json::to_vec(&credential).map_err(anyhow::Error::from)?,
    )?;
    if google(state).await?.hash() != attempt.client_hash {
        return Err(ApiError::conflict(
            "Google application settings changed; start again",
        ));
    }
    let user = attempt.user.clone();
    let saved=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved=tx.execute("UPDATE online_accounts SET status='connected',credential=?1,expires_at=?2,display_name=?3,external_id=?4,updated_at=?5 WHERE user_id=?6 AND provider='youtube' AND generation=?7 AND EXISTS(SELECT 1 FROM sessions WHERE id=?8 AND user_id=?6 AND expires_at>?5)",params![encrypted,now()+expires,name,external,now(),attempt.user,attempt.generation,attempt.session])?==1;
        if saved {tx.execute("INSERT INTO youtube_sync(user_id,generation) VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET generation=excluded.generation,cursor='{}',next_run=0,failures=0,error=NULL",params![attempt.user,attempt.generation])?;}
        tx.commit()?;Ok(saved)
    }).await?;
    if !saved {
        return Err(ApiError::conflict(
            "The account or session changed; start account linking again",
        ));
    }
    state
        .emit(
            Some(user),
            "online.account.changed",
            json!({"provider":"youtube"}),
        )
        .await?;
    Ok(())
}
pub(super) async fn send(
    http: reqwest::Client,
    request: oauth2::HttpRequest,
) -> std::result::Result<oauth2::HttpResponse, std::io::Error> {
    let (parts, body) = request.into_parts();
    let response = http
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| std::io::Error::other("OAuth provider request failed"))?;
    let (status, headers, body) = bounded_response(response).await?;
    let mut response = axum::http::Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}
