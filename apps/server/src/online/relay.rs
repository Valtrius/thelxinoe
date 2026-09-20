use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode, header},
    response::Response,
};
use futures_util::StreamExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::{OnceCell, Semaphore};

pub(super) struct Relay {
    client: OnceCell<reqwest::Client>,
    slots: Arc<Semaphore>,
}
impl Default for Relay {
    fn default() -> Self {
        Self {
            client: OnceCell::new(),
            slots: Arc::new(Semaphore::new(16)),
        }
    }
}

// A single byte range is sufficient for MPV's ordinary file reads and seeks.
// Reject multipart and malformed ranges before contacting the provider.
fn valid_range(value: &str) -> bool {
    let Some((start, end)) = value.strip_prefix("bytes=").and_then(|v| v.split_once('-')) else {
        return false;
    };
    let number = |v: &str| {
        (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .then(|| v.parse::<u64>().ok())
            .flatten()
    };
    match (start.is_empty(), end.is_empty()) {
        (true, true) => false,
        (true, false) => number(end).is_some_and(|n| n > 0),
        (false, true) => number(start).is_some(),
        (false, false) => number(start).zip(number(end)).is_some_and(|(a, b)| a <= b),
    }
}

pub(crate) async fn stream(
    state: &AppState,
    id: &str,
    track: &str,
    request: Request,
) -> Result<Response> {
    let source = super::streams::remote_source(state, id).await?;
    source.validate().map_err(|_| ApiError::not_found())?;
    let address = match track {
        "video" => &source.video,
        "audio" => source.audio.as_ref().ok_or_else(ApiError::not_found)?,
        _ => return Err(ApiError::not_found()),
    };
    let range = request.headers().get(header::RANGE);
    if range.is_some_and(|v| !v.to_str().is_ok_and(valid_range)) {
        return Err(ApiError::bad("Use a single valid byte range"));
    }
    let relay = &state.online.streams.relay;
    let permit = relay.slots.clone().try_acquire_owned().map_err(|_| {
        ApiError::conflict("All online file connections are busy; try again shortly")
    })?;
    let client = relay
        .client
        .get_or_try_init(|| async {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| ApiError::conflict("Could not initialize online playback"))
        })
        .await?;
    // Never forward viewer credentials, cookies or arbitrary headers. Redirects
    // cannot escape the CDN allowlist or expose the signed address to a client.
    let mut upstream = client
        .get(address)
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some(range) = range {
        upstream = upstream.header(header::RANGE, range);
    }
    let upstream = upstream
        .send()
        .await
        .map_err(|_| ApiError::conflict("The public video connection was interrupted"))?;
    if !matches!(
        upstream.status(),
        StatusCode::OK | StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE
    ) {
        return Err(ApiError::conflict(
            "The public video address expired or is unavailable; start playback again",
        ));
    }
    let mut response = Response::builder()
        .status(upstream.status())
        .header(header::CACHE_CONTROL, "private, no-store");
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
    ] {
        if name == header::CONTENT_LENGTH && upstream.status() == StatusCode::RANGE_NOT_SATISFIABLE
        {
            continue;
        }
        if let Some(value) = upstream.headers().get(&name) {
            response = response.header(name, value);
        }
    }
    if upstream.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        response = response.header(header::CONTENT_LENGTH, "0");
    }
    let body = if request.method() == Method::HEAD
        || upstream.status() == StatusCode::RANGE_NOT_SATISFIABLE
    {
        Body::empty()
    } else {
        // Keep the slot until the reader drops its body. Backpressure prevents
        // buffering a whole video in server memory; errors never contain URLs.
        Body::from_stream(futures_util::stream::unfold(
            (upstream.bytes_stream(), permit),
            |(mut stream, permit)| async move {
                stream.next().await.map(|chunk| {
                    (
                        chunk.map_err(|_| {
                            std::io::Error::other("The public video connection was interrupted")
                        }),
                        (stream, permit),
                    )
                })
            },
        ))
    };
    response
        .body(body)
        .map_err(|_| ApiError::conflict("Invalid public video response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_support_seek_and_reject_multipart_and_overflow() {
        for range in ["bytes=0-", "bytes=100-199", "bytes=-4096", "bytes=0-0"] {
            assert!(valid_range(range), "{range}");
        }
        for range in [
            "bytes=-",
            "bytes=-0",
            "bytes=2-1",
            "bytes=0-1,4-5",
            "bytes=+1-2",
            "bytes= 1-2",
            "bytes=1--2",
            "items=0-1",
            "bytes=18446744073709551616-",
        ] {
            assert!(!valid_range(range), "{range}");
        }
    }
}
