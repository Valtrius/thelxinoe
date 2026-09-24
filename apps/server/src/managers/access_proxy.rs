use super::*;
use axum::{
    body::Body,
    extract::{
        FromRequestParts,
        ws::{Message, WebSocketUpgrade},
    },
    http::HeaderValue,
};
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::{Message as UpstreamMessage, client::IntoClientRequest};

const UPLOAD_LIMIT: u64 = 1024 * 1024 * 1024;

fn unavailable_proxy() -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "service_unavailable",
        "The service is unavailable. Check its status and URL Base in Media services.".into(),
    )
}

fn filter_headers(headers: &HeaderMap) -> HeaderMap {
    let mut output = headers.clone();
    // Connection can nominate additional hop-by-hop headers.
    for connection in headers.get_all(header::CONNECTION) {
        if let Ok(value) = connection.to_str() {
            for name in value.split(',').map(str::trim) {
                output.remove(name);
            }
        }
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "cookie",
        "set-cookie",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-forwarded-prefix",
        "x-real-ip",
        "x-original-url",
        "x-rewrite-url",
        "x-thelxinoe-client",
        "x-thelxinoe-api",
        "sec-websocket-key",
        "sec-websocket-version",
        "sec-websocket-extensions",
        "sec-websocket-accept",
        "sec-websocket-protocol",
    ] {
        output.remove(name);
    }
    output
}

fn request_headers(
    headers: &HeaderMap,
    s: &Destination,
    context: &security::RequestContext,
    private: &str,
) -> Result<HeaderMap> {
    let mut output = filter_headers(headers);
    // The UI can use its own HTTP Basic login. Device bearer tokens belong only
    // to Thelxinoe's API and must never reach a native service.
    if s.kind == "nzbget"
        || !output
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("Basic "))
    {
        output.remove(header::AUTHORIZATION);
    }
    let prefix = upstream_cookie_prefix(s);
    let cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter_map(|(name, value)| {
            name.strip_prefix(&prefix)
                .map(|name| format!("{name}={value}"))
        })
        .collect::<Vec<_>>()
        .join("; ");
    if !cookies.is_empty() {
        output.insert(
            header::COOKIE,
            cookies
                .parse()
                .map_err(|_| ApiError::bad("Invalid service cookie"))?,
        );
    }
    let public = url::Url::parse(&context.origin).map_err(|_| ApiError::forbidden())?;
    // Keep Prowlarr's Host allowlist scoped to its registered private address.
    // Its native redirects are translated back into paths below.
    let host = if s.kind == "prowlarr" {
        private
            .strip_prefix("http://")
            .ok_or_else(unavailable_proxy)?
    } else {
        &context.origin[public.scheme().len() + 3..]
    };
    output.insert(
        header::HOST,
        host.parse().map_err(|_| ApiError::forbidden())?,
    );
    output.insert(
        "x-forwarded-host",
        host.parse().map_err(|_| ApiError::forbidden())?,
    );
    output.insert("x-forwarded-proto", public.scheme().parse().unwrap());
    output.insert(
        "x-forwarded-for",
        context.address.to_string().parse().unwrap(),
    );
    Ok(output)
}

fn response_cookie(value: &HeaderValue, s: &Destination, secure: bool) -> Option<HeaderValue> {
    let mut parts = value.to_str().ok()?.split(';');
    let (name, value) = parts.next()?.trim().split_once('=')?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return None;
    }
    let mut cookie = format!(
        "{}{name}={value}; Path={}/; SameSite=Strict",
        upstream_cookie_prefix(s),
        mount(&s.kind, &s.url_base)
    );
    for attribute in parts {
        let name = attribute.trim().split('=').next()?.to_ascii_lowercase();
        if matches!(name.as_str(), "expires" | "max-age" | "httponly") {
            cookie.push_str("; ");
            cookie.push_str(attribute.trim());
        }
    }
    if secure {
        cookie.push_str("; Secure");
    }
    cookie.parse().ok()
}

fn response_headers(
    headers: &HeaderMap,
    s: &Destination,
    origin: &str,
    private: &str,
    target: &str,
    secure: bool,
) -> Result<HeaderMap> {
    let mut output = filter_headers(headers);
    if s.kind == "nzbget" {
        output.remove(header::WWW_AUTHENTICATE);
    }
    for name in [
        "access-control-allow-origin",
        "access-control-allow-credentials",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "access-control-expose-headers",
        "service-worker-allowed",
        "refresh",
        "alt-svc",
        "clear-site-data",
    ] {
        output.remove(name);
    }
    for cookie in headers.get_all(header::SET_COOKIE) {
        if let Some(cookie) = response_cookie(cookie, s, secure) {
            output.append(header::SET_COOKIE, cookie);
        }
    }
    if let Some(location) = headers.get(header::LOCATION) {
        let target = url::Url::parse(target).map_err(|_| unavailable_proxy())?;
        let mut redirect = target
            .join(location.to_str().map_err(|_| unavailable_proxy())?)
            .map_err(|_| unavailable_proxy())?;
        // Servarr may construct Location from Host while ignoring the forwarded
        // scheme. Accept that same public authority and return a relative URL.
        let public = url::Url::parse(origin).map_err(|_| unavailable_proxy())?;
        let public_authority = redirect.host_str() == public.host_str()
            && redirect.port() == public.port()
            && matches!(redirect.scheme(), "http" | "https");
        let private = url::Url::parse(private).map_err(|_| unavailable_proxy())?;
        let private_authority = redirect.host_str() == private.host_str()
            && redirect.port() == private.port()
            && matches!(redirect.scheme(), "http" | "https");
        if (!public_authority && !private_authority)
            || !redirect.username().is_empty()
            || redirect.password().is_some()
        {
            return Err(unavailable_proxy());
        }
        let base = mount(&s.kind, &s.url_base);
        if s.kind == "nzbget" {
            redirect.set_path(&format!("{base}{}", redirect.path()));
        }
        if !within(redirect.path(), base) {
            return Err(unavailable_proxy());
        }
        let local = &redirect[url::Position::BeforePath..];
        output.insert(
            header::LOCATION,
            local.parse().map_err(|_| unavailable_proxy())?,
        );
    }
    output.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(output)
}

#[derive(Clone)]
struct Gate {
    state: AppState,
    service: Destination,
    authorization: Authorization,
}

impl Gate {
    async fn check(&self) -> bool {
        let Ok(current) = destination(&self.state, &self.service.id).await else {
            return false;
        };
        if current.container != self.service.container
            || current.access_revision != self.service.access_revision
            || current.url_base != self.service.url_base
            || self
                .authorization
                .principal(&self.state, &current)
                .await
                .is_err()
        {
            return false;
        }
        support::api_evidence(&self.state, &current.container, current.port, &current.kind)
            .await
            .is_ok_and(|(_, source)| source == current.media_source)
    }
    async fn revoked(&self) {
        let mut events = self.state.events.subscribe();
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! { _ = events.recv() => {}, _ = interval.tick() => {} }
            if !self.check().await {
                return;
            }
        }
    }
}

pub(super) async fn forward(
    state: AppState,
    s: Destination,
    authorization: Authorization,
    context: security::RequestContext,
    request: Request,
    websocket: bool,
) -> Result<Response> {
    let path = request.uri().path();
    let lower = path.to_ascii_lowercase();
    let base = mount(&s.kind, &s.url_base);
    if !within(path, base)
        || path.contains(['\\', '\0'])
        || path.contains("//")
        || path.split('/').any(|part| matches!(part, "." | ".."))
        || ["%2e", "%2f", "%5c", "%25", "%00"]
            .iter()
            .any(|encoded| lower.contains(encoded))
        || request.uri().query().is_some_and(|q| q.len() > 32 * 1024)
    {
        return Err(ApiError::bad("Invalid service path"));
    }
    if request.headers().contains_key("service-worker") {
        return Err(ApiError::forbidden());
    }
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|len| len > UPLOAD_LIMIT)
    {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            "Service uploads are limited to 1 GiB".into(),
        ));
    }
    let (connection, credentials) = connection(&state, &s).await?;
    let private = connection.base;
    let path_and_query = request
        .uri()
        .path_and_query()
        .ok_or_else(|| ApiError::bad("Invalid service URL"))?
        .as_str();
    let upstream_path = if s.kind == "nzbget" {
        path_and_query
            .strip_prefix(base)
            .ok_or_else(unavailable_proxy)?
    } else {
        path_and_query
    };
    let target = format!("{private}{upstream_path}");
    let headers = request_headers(request.headers(), &s, &context, &private)?;
    if s.kind == "nzbget" && websocket {
        return Err(ApiError::bad("NZBGet does not use WebSockets"));
    }
    let gate = Gate {
        state: state.clone(),
        service: s.clone(),
        authorization,
    };
    if websocket {
        return forward_websocket(request, headers, target, gate).await;
    }
    let (parts, body) = request.into_parts();
    let stream = body
        .into_data_stream()
        .map_err(|_| std::io::Error::other("Service request interrupted"))
        .scan(0u64, |received, chunk| {
            let chunk = chunk.and_then(|chunk| {
                *received += chunk.len() as u64;
                if *received <= UPLOAD_LIMIT {
                    Ok(chunk)
                } else {
                    Err(std::io::Error::other("Service upload exceeds limit"))
                }
            });
            futures_util::future::ready(Some(chunk))
        });
    let mut sending = state
        .managers
        .proxy_http
        .request(parts.method, &target)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(stream));
    if let Some(credentials) = credentials {
        // Always replace browser credentials with the registered connection.
        sending = sending.basic_auth(credentials.username, Some(credentials.secret));
    }
    let sending = sending.send();
    let upstream = tokio::select! {
        result = sending => result.map_err(|_| unavailable_proxy())?,
        _ = gate.revoked() => return Err(ApiError::unauthorized()),
    };
    let status = upstream.status();
    if s.kind == "nzbget" && status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::conflict(
            "NZBGet rejected its saved credentials. Reconnect it in Media services.",
        ));
    }
    let headers = response_headers(
        upstream.headers(),
        &s,
        &context.origin,
        &private,
        &target,
        context.secure,
    )?;
    // Cancellation drops the upstream body as soon as authorization or Docker
    // evidence disappears. No detached forwarding task can outlive the response.
    let revoked = Box::pin(async move { gate.revoked().await });
    let stream = futures_util::stream::try_unfold(
        (upstream.bytes_stream().boxed(), revoked),
        |(mut upstream, mut revoked)| async move {
            tokio::select! {
                _ = &mut revoked => Err(std::io::Error::other("Service access ended")),
                chunk = upstream.next() => match chunk {
                    Some(Ok(bytes)) => Ok(Some((bytes, (upstream, revoked)))),
                    Some(Err(_)) => Err(std::io::Error::other("Service stream interrupted")),
                    None => Ok(None),
                }
            }
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

async fn forward_websocket(
    request: Request,
    headers: HeaderMap,
    target: String,
    gate: Gate,
) -> Result<Response> {
    let (mut parts, _) = request.into_parts();
    let upgrade = WebSocketUpgrade::from_request_parts(&mut parts, &())
        .await
        .map_err(|_| ApiError::bad("Invalid service WebSocket"))?;
    let mut upstream = target
        .replacen("http://", "ws://", 1)
        .into_client_request()
        .map_err(|_| unavailable_proxy())?;
    for (name, value) in &headers {
        upstream.headers_mut().insert(name, value.clone());
    }
    if let Some(protocols) = parts.headers.get("sec-websocket-protocol") {
        upstream
            .headers_mut()
            .insert("sec-websocket-protocol", protocols.clone());
    }
    let (socket, response) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(upstream),
    )
    .await
    .map_err(|_| unavailable_proxy())?
    .map_err(|_| unavailable_proxy())?;
    let upgrade = if let Some(protocol) = response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
    {
        upgrade.protocols([protocol.to_owned()])
    } else {
        upgrade
    };
    Ok(upgrade.on_upgrade(move |browser| async move {
        let (mut browser_write, mut browser_read) = browser.split();
        let (mut service_write, mut service_read) = socket.split();
        {
            let to_service = async {
                while let Some(Ok(message)) = browser_read.next().await {
                    let message = match message {
                        Message::Text(v) => UpstreamMessage::Text(v.to_string().into()),
                        Message::Binary(v) => UpstreamMessage::Binary(v),
                        Message::Ping(v) => UpstreamMessage::Ping(v),
                        Message::Pong(v) => UpstreamMessage::Pong(v),
                        Message::Close(_) => UpstreamMessage::Close(None),
                    };
                    if service_write.send(message).await.is_err() {
                        break;
                    }
                }
            };
            let to_browser = async {
                while let Some(Ok(message)) = service_read.next().await {
                    let message = match message {
                        UpstreamMessage::Text(v) => Message::Text(v.to_string().into()),
                        UpstreamMessage::Binary(v) => Message::Binary(v),
                        UpstreamMessage::Ping(v) => Message::Ping(v),
                        UpstreamMessage::Pong(v) => Message::Pong(v),
                        UpstreamMessage::Close(_) => Message::Close(None),
                        UpstreamMessage::Frame(_) => continue,
                    };
                    if browser_write.send(message).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! { _ = to_service => {}, _ = to_browser => {}, _ = gate.revoked() => {} }
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            browser_write.send(Message::Close(None)),
        )
        .await;
        let _ = tokio::time::timeout(Duration::from_secs(1), service_write.close()).await;
    }))
}
