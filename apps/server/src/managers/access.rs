//! Same-origin native UI access to registered media-service containers.
#[path = "access_proxy.rs"]
mod proxy;
#[path = "../storage/managers/access.rs"]
mod storage;

use super::*;
use axum::{
    Extension,
    extract::Request,
    http::{Method, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::any,
};
use thelxinoe_core::Principal;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub(crate) struct NativeResponse;

#[derive(Clone)]
struct Destination {
    id: String,
    kind: String,
    container: String,
    port: u16,
    url_base: String,
    access_revision: String,
    media_source: String,
}

async fn destination(state: &AppState, id: &str) -> Result<Destination> {
    storage::load(state, id.to_owned())
        .await?
        .ok_or_else(ApiError::not_found)
}

async fn connection<'a>(
    state: &'a AppState,
    s: &Destination,
) -> Result<(Connection<'a>, Option<support::Credentials>)> {
    if matches!(s.kind.as_str(), "prowlarr" | "bazarr" | "nzbget") {
        let mut service = support::load(state, &s.id).await?;
        if service.container != s.container || service.access_revision != s.access_revision {
            return Err(ApiError::conflict(
                "Service changed; open it again from Thelxinoe",
            ));
        }
        service.url_base = s.url_base.clone();
        let connection = support::connect(state, &service).await?;
        Ok((
            connection,
            (s.kind == "nzbget").then_some(service.credentials),
        ))
    } else {
        let mut service = service(state, &s.id).await?;
        if service.container != s.container || service.access_revision != s.access_revision {
            return Err(ApiError::conflict(
                "Service changed; open it again from Thelxinoe",
            ));
        }
        service.url_base = s.url_base.clone();
        Ok((Connection::open(state, &service).await?, None))
    }
}

pub(super) fn supported(kind: &str) -> bool {
    kind == "nzbget" || !thelxinoe_core::service_url_base(kind).is_empty()
}

// NZBGet serves at its private root. Its public mount belongs to the gateway.
fn mount<'a>(kind: &str, base: &'a str) -> &'a str {
    if kind == "nzbget" {
        "/services/nzbget"
    } else {
        base
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/managers/{id}/access-ticket", post(ticket))
        .route("/service-access/bootstrap.js", get(bootstrap_script))
        .route("/service-access/{id}", get(bootstrap).post(exchange))
        .route("/services/{kind}", any(native))
        .route("/services/{kind}/", any(native))
        .route("/services/{kind}/{*path}", any(native))
}

pub(super) fn launch_urls(
    db: &rusqlite::Connection,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    storage::launch_urls(db)
}

fn can_mount(kind: &str, base: &str) -> bool {
    if kind == "nzbget" {
        return base.is_empty();
    }
    if !supported(kind) || base.is_empty() || validate_base(kind, base).is_err() {
        return false;
    }
    if base == thelxinoe_core::service_url_base(kind) {
        return true;
    }
    // Attached services may use application paths privately, but must not
    // publish a native UI over an application route or another service's mount.
    ![
        "api",
        "assets",
        "service-access",
        "seerr-bootstrap",
        "services",
        "system",
        "users",
        "userviews",
        "useritems",
        "userfavoriteitems",
        "userplayeditems",
        "items",
        "shows",
        "artists",
        "albumartists",
        "genres",
        "musicgenres",
        "persons",
        "sessions",
        "audio",
        "videos",
        "displaypreferences",
        "quickconnect",
        "branding",
        "mediasegments",
        "playlists",
        "web",
        "emby",
        "jellyfin",
    ]
    .contains(
        &base
            .split('/')
            .nth(1)
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
    )
}

pub(super) fn validate_base(kind: &str, base: &str) -> Result<()> {
    if base.is_empty() {
        return Ok(());
    }
    if thelxinoe_core::service_url_base(kind).is_empty() {
        return Err(ApiError::bad("This service does not support a URL Base"));
    }
    let segments = base
        .strip_prefix('/')
        .unwrap_or_default()
        .split('/')
        .collect::<Vec<_>>();
    if base.len() > 160
        || !base.starts_with('/')
        || segments.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        return Err(ApiError::bad(
            "Enter the service's existing URL Base, starting with / and without a trailing slash",
        ));
    }
    Ok(())
}

fn within(path: &str, base: &str) -> bool {
    !base.is_empty()
        && (path == base
            || path
                .strip_prefix(base)
                .is_some_and(|tail| tail.starts_with('/')))
}

pub(super) fn check_reported_base(kind: &str, base: &str, status: &Value) -> Result<()> {
    if matches!(kind, "radarr" | "sonarr" | "lidarr" | "prowlarr")
        && !status["appName"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(kind))
    {
        return Err(ApiError::conflict(
            "The service API identity does not match this connection",
        ));
    }
    if matches!(kind, "radarr" | "sonarr" | "lidarr" | "prowlarr")
        && status["urlBase"].as_str().unwrap_or("") != base
    {
        return Err(ApiError::conflict(format!(
            "The service's URL Base differs from its connection. Owned services must use /services/{kind}; reconnect external services using their existing prefix.",
        )));
    }
    Ok(())
}

pub(super) async fn check_connection(c: &Connection<'_>) -> Result<()> {
    let status = c.get("system/status").await?;
    if c.kind != "bazarr" {
        return check_reported_base(&c.kind, &c.url_base, &status);
    }
    if !status["data"]["bazarr_version"]
        .as_str()
        .is_some_and(|v| !v.is_empty())
    {
        return Err(ApiError::conflict(
            "The service API identity does not match this connection",
        ));
    }
    let settings = c.get("system/settings").await?;
    if settings["general"]["base_url"]
        .as_str()
        .map(|v| v.trim_end_matches('/'))
        != Some(c.url_base.as_str())
    {
        return Err(ApiError::conflict(
            "Bazarr's URL Base differs from its connection. Owned Bazarr must use /services/bazarr; reconnect external Bazarr using its existing prefix.",
        ));
    }
    Ok(())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned());
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn resource(s: &Destination, purpose: &str) -> String {
    format!(
        "service-{purpose}:{}:{}:{}",
        s.id, s.access_revision, s.container
    )
}
fn grant_cookie(s: &Destination) -> String {
    format!("thelxinoe_service_{}", s.id)
}
fn upstream_cookie_prefix(s: &Destination) -> String {
    format!("thelxinoe_native_{}_", s.id)
}

#[derive(Clone)]
enum Authorization {
    Session(String),
    Grant(String),
}

impl Authorization {
    async fn principal(&self, state: &AppState, service: &Destination) -> Result<Principal> {
        let p = match self {
            Self::Session(raw) => thelxinoe_auth::resolve(&state.db, raw, "web").await?,
            Self::Grant(raw) => {
                crate::grants::resolve(state, raw, &resource(service, "browser"), false).await?
            }
        }
        .ok_or_else(ApiError::unauthorized)?;
        if !p.user.role.allows(Capability::ManageServer) {
            return Err(ApiError::forbidden());
        }
        Ok(p)
    }
}

async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    s: &Destination,
) -> Result<Authorization> {
    // A native API key or Authorization header never authenticates the gateway.
    let candidates = [
        cookie(headers, "thelxinoe_session").map(Authorization::Session),
        cookie(headers, &grant_cookie(s)).map(Authorization::Grant),
    ];
    let mut denied = ApiError::unauthorized();
    for auth in candidates.into_iter().flatten() {
        match auth.principal(state, s).await {
            Ok(_) => return Ok(auth),
            Err(error) => denied = error,
        }
    }
    Err(denied)
}

fn check_origin(
    headers: &HeaderMap,
    context: &security::RequestContext,
    required: bool,
) -> Result<()> {
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| matches!(v, "cross-site" | "same-site"))
    {
        return Err(ApiError::forbidden());
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin.is_some_and(|o| o != context.origin)
        || required && origin != Some(context.origin.as_str())
    {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

async fn ready(state: &AppState, s: &Destination) -> Result<()> {
    let (connection, credentials) = connection(state, s).await?;
    if let Some(credentials) = credentials {
        support::version(&connection, &credentials).await?;
        Ok(())
    } else {
        check_connection(&connection).await
    }
}

pub(crate) async fn fallback(State(state): State<AppState>, request: Request) -> Response {
    match storage::routes(&state).await {
        Ok(routes)
            if routes
                .iter()
                .any(|(_, _, base)| within(request.uri().path(), base)) =>
        {
            return native(State(state), request).await;
        }
        Err(error) => return ApiError::from(error).into_response(),
        _ => {}
    }
    if request.uri().path().starts_with("/services/")
        || request.uri().path().starts_with("/service-access/")
    {
        ApiError::not_found().into_response()
    } else {
        web_files(&state.config.web, request).await
    }
}

async fn web_files(root: &std::path::Path, mut request: Request) -> Response {
    let unversioned = !request.uri().path().starts_with("/assets/");
    if unversioned {
        // A rollback can install an older index.html. Its modification time
        // must not validate cached HTML that names a newer release's assets.
        request.headers_mut().remove(header::IF_MODIFIED_SINCE);
        request.headers_mut().remove(header::IF_NONE_MATCH);
    }
    let mut response = ServeDir::new(root)
        .not_found_service(ServeFile::new(root.join("index.html")))
        .oneshot(request)
        .await
        .unwrap()
        .into_response();
    if unversioned {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
}

async fn native(State(state): State<AppState>, request: Request) -> Response {
    let document = request.method() == Method::GET
        && request
            .headers()
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"));
    let result = dispatch(state, request).await;
    let mut response = result.unwrap_or_else(|error| {
        if document {
            let message = error.2.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
            (error.0, Html(format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Service unavailable</title><h1>Service unavailable</h1><p>{message}</p><a href=\"/?section=Settings\">Open Thelxinoe settings</a></html>"))).into_response()
        } else { error.into_response() }
    });
    response.extensions_mut().insert(NativeResponse);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn dispatch(state: AppState, request: Request) -> Result<Response> {
    let path = request.uri().path();
    let routes = storage::routes(&state).await?;
    let (id, kind, _) = routes
        .iter()
        .find(|(_, kind, base)| path == format!("/services/{kind}") || within(path, base))
        .ok_or_else(ApiError::not_found)?;
    let s = destination(&state, id).await?;
    let context = request
        .extensions()
        .get::<security::RequestContext>()
        .cloned()
        .ok_or_else(ApiError::forbidden)?;
    let is_document = request.method() == Method::GET
        && request
            .headers()
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"));
    let auth = match authorize(&state, request.headers(), &s).await {
        Err(e) if e.0 == StatusCode::UNAUTHORIZED && is_document => {
            return Ok(Redirect::to(&format!("/?service={kind}")).into_response());
        }
        result => result?,
    };
    let websocket = request
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    check_origin(
        request.headers(),
        &context,
        websocket
            || !matches!(
                *request.method(),
                Method::GET | Method::HEAD | Method::OPTIONS
            ),
    )?;
    if s.kind == "nzbget"
        && path
            .split('/')
            .any(|part| matches!(part, "jsonrpc" | "xmlrpc" | "jsonprpc"))
        && matches!(*request.method(), Method::GET | Method::HEAD)
        && request
            .headers()
            .get("sec-fetch-site")
            .and_then(|h| h.to_str().ok())
            != Some("same-origin")
    {
        // NZBGet exposes RPC over GET too. Its own UI sends same-origin fetch
        // metadata; non-browser callers must supply the validated Origin.
        check_origin(request.headers(), &context, true)?;
    }
    if is_document || path == format!("/services/{kind}") {
        ready(&state, &s).await?;
    }
    let base = mount(&s.kind, &s.url_base);
    if path == format!("/services/{kind}") || path == base {
        return Ok(Redirect::temporary(&format!("{base}/")).into_response());
    }
    proxy::forward(state, s, auth, context, request, websocket).await
}

async fn ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if p.transport != "device" {
        return Err(ApiError::forbidden());
    }
    let s = destination(&state, &id).await?;
    if !supported(&s.kind) {
        return Err(ApiError::not_found());
    }
    ready(&state, &s).await?;
    let raw = crate::grants::issue(&state, &p, &resource(&s, "launch"), 30).await?;
    Ok(Json(
        json!({"ticket":raw,"path":format!("/service-access/{id}"),"public_origin":state.config.public_url.as_ref().map(|url|url.origin().ascii_serialization())}),
    ))
}

async fn bootstrap() -> Response {
    let mut response = Html("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Open service</title><p id=\"status\">Opening service…</p><script src=\"/service-access/bootstrap.js\" defer></script></html>").into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn bootstrap_script() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript"), (header::CACHE_CONTROL, "no-store")], r#"const ticket = location.hash.slice(1);
history.replaceState(null, '', location.pathname);
if (!ticket) document.getElementById('status').textContent = 'Open this service again from Thelxinoe.';
else fetch(location.pathname, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({ticket})})
 .then(async r => {if (!r.ok) throw Error(); const v=await r.json(); if (!v.path.startsWith('/') || v.path.startsWith('//')) throw Error(); location.replace(v.path);})
 .catch(() => {document.getElementById('status').textContent='This service link expired or is unavailable. Open it again from Thelxinoe.';});"#).into_response()
}

#[derive(Deserialize)]
struct Exchange {
    ticket: String,
}

async fn exchange(
    State(state): State<AppState>,
    Extension(context): Extension<security::RequestContext>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Exchange>,
) -> Result<Response> {
    check_origin(&headers, &context, true)?;
    let s = destination(&state, &id).await?;
    let p = crate::grants::resolve(&state, &input.ticket, &resource(&s, "launch"), true)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if p.transport != "device" || !p.user.role.allows(Capability::ManageServer) {
        return Err(ApiError::forbidden());
    }
    ready(&state, &s).await?;
    let token = crate::grants::issue(&state, &p, &resource(&s, "browser"), 8 * 60 * 60).await?;
    let base = mount(&s.kind, &s.url_base);
    let mut response = Json(json!({"path":format!("{base}/")})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!(
            "{}={token}; HttpOnly; SameSite=Strict; Path={}/; Max-Age=28800{}",
            grant_cookie(&s),
            base,
            if context.secure { "; Secure" } else { "" }
        )
        .parse()
        .unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}

#[cfg(test)]
mod web_cache_tests {
    use super::*;
    use axum::body::{Body, to_bytes};

    #[tokio::test]
    async fn rollback_serves_current_html_despite_newer_browser_validators() {
        let directory = tempfile::tempdir().unwrap();
        let html = "<script src='/assets/base.js'></script>";
        std::fs::write(directory.path().join("index.html"), html).unwrap();
        for path in ["/", "/?section=Settings", "/index.html"] {
            let request = Request::builder()
                .uri(path)
                .header(header::IF_MODIFIED_SINCE, "Thu, 01 Jan 2099 00:00:00 GMT")
                .header(header::IF_NONE_MATCH, "\"newer-release\"")
                .body(Body::empty())
                .unwrap();
            let response = web_files(directory.path(), request).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
                html.as_bytes()
            );
        }
    }

    #[tokio::test]
    async fn versioned_assets_still_support_conditional_requests() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("assets")).unwrap();
        std::fs::write(directory.path().join("assets/base.js"), "// base").unwrap();
        let request = Request::builder()
            .uri("/assets/base.js")
            .header(header::IF_MODIFIED_SINCE, "Thu, 01 Jan 2099 00:00:00 GMT")
            .body(Body::empty())
            .unwrap();
        let response = web_files(directory.path(), request).await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    }
}
