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
        .map_err(|error| std::io::Error::other(ReadFailed(error)))?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(std::io::Error::other("Provider response exceeds limit"));
        }
        bytes.extend(chunk);
    }
    Ok((status, headers, bytes))
}

/// A body that stopped arriving; the cause is kept as its source for logs.
#[derive(Debug)]
struct ReadFailed(reqwest::Error);

impl std::fmt::Display for ReadFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Provider response failed")
    }
}

impl std::error::Error for ReadFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
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

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_body_cut_short_keeps_its_cause_without_showing_it() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await
                .unwrap();
        });
        let reply = reqwest::get(format!("http://{address}/")).await.unwrap();
        let error = super::response(reply).await.unwrap_err();
        assert_eq!(error.to_string(), "Provider response failed");
        let logged = format!("{:#}", anyhow::Error::from(error));
        assert!(logged.len() > "Provider response failed".len(), "{logged}");
    }
}
