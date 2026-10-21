//! Responses from third-party services, read without trusting their size.

use axum::http::{HeaderMap, StatusCode};

/// Reads a whole response, refusing one past 1 MiB as it arrives instead of after
/// buffering it.
pub(crate) async fn response(
    mut response: reqwest::Response,
) -> std::io::Result<(StatusCode, HeaderMap, Vec<u8>)> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| std::io::Error::other("Provider response failed"))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(std::io::Error::other("Provider response exceeds limit"));
        }
        bytes.extend(chunk);
    }
    Ok((status, headers, bytes))
}

/// The OAuth transport: `http` sends, and the reply is read through [`response`].
pub(crate) async fn oauth(
    http: reqwest::Client,
    request: oauth2::HttpRequest,
) -> std::io::Result<oauth2::HttpResponse> {
    let (parts, body) = request.into_parts();
    let reply = http
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| std::io::Error::other("OAuth provider request failed"))?;
    let (status, headers, body) = response(reply).await?;
    let mut reply = axum::http::Response::new(body);
    *reply.status_mut() = status;
    *reply.headers_mut() = headers;
    Ok(reply)
}
