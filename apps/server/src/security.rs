use crate::{
    AppState,
    config::Config,
    error::{ApiError, Result},
};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, SocketAddr};
use thelxinoe_core::{Capability, Principal};

#[derive(Clone)]
pub struct RequestContext {
    pub secure: bool,
    pub origin: String,
    pub address: IpAddr,
}
impl RequestContext {
    pub fn remote(&self) -> bool {
        match self.address {
            IpAddr::V4(ip) => !(ip.is_private() || ip.is_loopback() || ip.is_link_local()),
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
                || !(ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()),
                |ip| !(ip.is_private() || ip.is_loopback() || ip.is_link_local()),
            ),
        }
    }
}
pub fn request_context(
    config: &Config,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> Result<RequestContext> {
    let trusted = config
        .trusted_proxies
        .iter()
        .any(|net| net.contains(&peer.ip()));
    let forwarded = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.contains(','))
    };
    let scheme = if trusted {
        forwarded("x-forwarded-proto").unwrap_or("http")
    } else {
        "http"
    };
    if !matches!(scheme, "http" | "https") {
        return Err(ApiError::bad("Invalid forwarded scheme"));
    }
    let host = (if trusted {
        forwarded("x-forwarded-host")
    } else {
        None
    })
    .or_else(|| headers.get(header::HOST).and_then(|v| v.to_str().ok()))
    .unwrap_or("localhost");
    let origin = format!("{scheme}://{host}");
    let parsed = url::Url::parse(&origin).map_err(|_| ApiError::bad("Invalid host"))?;
    if parsed.host_str().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(ApiError::bad("Invalid host"));
    }
    let address = if trusted {
        headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|value| {
                value
                    .split(',')
                    .rev()
                    .map(str::trim)
                    .map(str::parse::<IpAddr>)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .ok()
            })
            .and_then(|ips| {
                ips.into_iter()
                    .find(|ip| !config.trusted_proxies.iter().any(|net| net.contains(ip)))
            })
            .unwrap_or(peer.ip())
    } else {
        peer.ip()
    };
    Ok(RequestContext {
        secure: scheme == "https",
        origin: config
            .public_url
            .as_ref()
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or(origin),
        address,
    })
}

pub async fn guard(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    if request.uri().path().starts_with("/api/v1/")
        && !["/api/v1/health", "/api/v1/release"].contains(&request.uri().path())
        && let Some(version) = request.headers().get("x-thelxinoe-api")
        && version.to_str().ok().and_then(|v| v.parse::<u32>().ok())
            != Some(thelxinoe_core::API_VERSION)
    {
        return ApiError(axum::http::StatusCode::UPGRADE_REQUIRED,"update_required","This client uses an incompatible API. Update the client or connect to a compatible server.".into()).into_response();
    }
    let product_command = request
        .uri()
        .path()
        .starts_with("/api/v1/admin/product-update/")
        && request.method() == Method::POST;
    let _release_gate = if product_command {
        None
    } else {
        Some(state.release_gate.read().await)
    };
    if state
        .release_quiescing
        .load(std::sync::atomic::Ordering::SeqCst)
        && !(request.method() == Method::GET
            && (request.uri().path().starts_with("/api/v1/admin/")
                || request.uri().path() == "/api/v1/health"
                || !request.uri().path().starts_with("/api/")
                    && (request.uri().path() == "/"
                        || request.uri().path().starts_with("/assets/"))))
    {
        return ApiError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "maintenance",
            "The server is preparing an update. Reconnect shortly.".into(),
        )
        .into_response();
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0)
        .unwrap_or_else(|| "127.0.0.1:0".parse().expect("literal address"));
    let context = match request_context(&state.config, request.headers(), peer) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let cors = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|origin| {
            state
                .config
                .cors_origins
                .iter()
                .any(|allowed| allowed == origin)
        })
        .map(str::to_owned);
    if request.uri().path().starts_with("/api/") {
        if let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            let desktop = matches!(
                origin,
                "http://tauri.localhost" | "https://tauri.localhost" | "tauri://localhost"
            );
            if origin != context.origin && !desktop && cors.is_none() {
                return ApiError::forbidden().into_response();
            }
            if desktop && request.headers().contains_key(header::COOKIE) {
                return ApiError::forbidden().into_response();
            }
        }
        if !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        ) && request
            .headers()
            .get("x-thelxinoe-client")
            .and_then(|v| v.to_str().ok())
            != Some("1")
        {
            return ApiError::bad("Missing client header").into_response();
        }
    }
    let api_request = request.uri().path().starts_with("/api/");
    let preflight = api_request
        && cors.is_some()
        && request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    request.extensions_mut().insert(context.clone());
    let mut response = if preflight {
        axum::http::StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    if api_request && let Some(origin) = cors {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().unwrap());
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            "true".parse().unwrap(),
        );
        response
            .headers_mut()
            .append(header::VARY, "Origin".parse().unwrap());
        if preflight {
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"
                    .parse()
                    .unwrap(),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type, Authorization, X-Thelxinoe-Client, X-Thelxinoe-API"
                    .parse()
                    .unwrap(),
            );
            response
                .headers_mut()
                .insert(header::ACCESS_CONTROL_MAX_AGE, "600".parse().unwrap());
        }
    }
    if api_request
        && response.status().is_client_error()
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| !v.starts_with("application/json"))
    {
        let status = response.status();
        response=(status,axum::Json(serde_json::json!({"error":{"code":"invalid_request","message":status.canonical_reason().unwrap_or("Invalid request")}}))).into_response();
    }
    if api_request && !response.headers().contains_key(header::CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
        .headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    response.headers_mut().insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'".parse().unwrap());
    if context.secure {
        response.headers_mut().insert(
            "strict-transport-security",
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}
pub async fn principal(state: &AppState, headers: &HeaderMap) -> Result<Principal> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .map(str::trim)
                .find_map(|s| s.strip_prefix("thelxinoe_session="))
        });
    let (raw, transport) = if let Some(token) = bearer {
        (token, "device")
    } else if let Some(token) = cookie {
        (token, "web")
    } else {
        return Err(ApiError::unauthorized());
    };
    thelxinoe_auth::resolve(&state.db, raw, transport)
        .await?
        .ok_or_else(ApiError::unauthorized)
}
pub async fn require(
    state: &AppState,
    headers: &HeaderMap,
    capability: Capability,
) -> Result<Principal> {
    let p = principal(state, headers).await?;
    if !p.user.role.allows(capability) {
        return Err(ApiError::forbidden());
    }
    Ok(p)
}
pub fn cookie(raw: &str, secure: bool) -> String {
    format!(
        "thelxinoe_session={raw}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000{}",
        if secure { "; Secure" } else { "" }
    )
}
