use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    response::Response,
};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// YouTube throttles open-ended reads to about twice the bitrate, while it
/// serves bounded ranges up to this size at full speed.
const CHUNK: u64 = 10 * 1024 * 1024;

fn cdn_redirect(url: &reqwest::Url, previous: usize) -> bool {
    previous <= 5
        && url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
        && url
            .host_str()
            .is_some_and(|host| host == "googlevideo.com" || host.ends_with(".googlevideo.com"))
}

pub(super) struct Relay {
    /// One client per egress proxy, which pins the source's address family.
    clients: Mutex<HashMap<Option<String>, reqwest::Client>>,
    slots: Arc<Semaphore>,
}
impl Default for Relay {
    fn default() -> Self {
        Self {
            clients: Mutex::default(),
            slots: Arc::new(Semaphore::new(32)),
        }
    }
}
impl Relay {
    pub(super) fn client(&self, proxy: Option<&String>) -> Result<reqwest::Client> {
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.get(&proxy.cloned()) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if cdn_redirect(attempt.url(), attempt.previous().len()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .referer(false)
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30));
        // Product tests serve fixtures from a loopback CDN with a self-signed certificate.
        #[cfg(test)]
        {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(proxy) = proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|_| ApiError::conflict("Could not initialize online playback"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|_| ApiError::conflict("Could not initialize online playback"))?;
        clients.insert(proxy.cloned(), client.clone());
        Ok(client)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Range {
    /// First byte and optional inclusive last byte.
    From(u64, Option<u64>),
    /// The final bytes of the file.
    Suffix(u64),
}
// A single byte range is sufficient for MPV's file reads and HLS byte ranges.
// Reject multipart and malformed ranges before contacting the provider.
fn range(value: &str) -> Option<Range> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let number = |v: &str| {
        (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .then(|| v.parse::<u64>().ok())
            .flatten()
    };
    match (start.is_empty(), end.is_empty()) {
        (true, true) => None,
        (true, false) => number(end).filter(|n| *n > 0).map(Range::Suffix),
        (false, true) => Some(Range::From(number(start)?, None)),
        (false, false) => {
            let (a, b) = (number(start)?, number(end)?);
            (a <= b).then_some(Range::From(a, Some(b)))
        }
    }
}
/// The returned first byte, last byte and total size of a partial response.
fn content_range(response: &reqwest::Response) -> Option<(u64, u64, u64)> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?, total.parse().ok()?))
}

pub(crate) async fn stream(
    state: &AppState,
    id: &str,
    track: &str,
    request: Request,
) -> Result<Response> {
    if let Some(track) = track.strip_suffix("-init") {
        // HLS initialization segments were read with the index; serve them from memory.
        let (kind, bytes) = super::streams::initialization(state, id, track)
            .await
            .ok_or_else(ApiError::not_found)?;
        return Response::builder()
            .header(header::CONTENT_TYPE, kind)
            .header(header::CACHE_CONTROL, "private, no-store")
            .body(Body::from(bytes))
            .map_err(|_| ApiError::conflict("Invalid public video response"));
    }
    let (source, available_at) = super::streams::remote_source(state, id).await?;
    source.validate().map_err(|_| ApiError::not_found())?;
    let address = match track {
        "video" => &source.video,
        "audio" => source.audio.as_ref().ok_or_else(ApiError::not_found)?,
        _ => return Err(ApiError::not_found()),
    }
    .clone();
    let requested = match request.headers().get(header::RANGE) {
        Some(value) => Some(
            value
                .to_str()
                .ok()
                .and_then(range)
                .ok_or_else(|| ApiError::bad("Use a single valid byte range"))?,
        ),
        None => None,
    };
    let relay = &state.online.streams.relay;
    let permit = relay.slots.clone().try_acquire_owned().map_err(|_| {
        ApiError::conflict("All online file connections are busy; try again shortly")
    })?;
    let client = relay.client(source.proxy.as_ref())?;
    super::streams::wait_until(available_at).await;
    let head = request.method() == Method::HEAD;
    // Never forward viewer credentials, cookies or arbitrary headers. Redirects
    // stay within the CDN allowlist and never expose signed addresses to a client.
    let (first, last) = match requested {
        Some(Range::Suffix(_)) => (None, None),
        Some(Range::From(first, last)) => (Some(first), last),
        None => (Some(0), None),
    };
    let upstream_range = match (first, requested) {
        (Some(first), _) => {
            let end = if head {
                first
            } else {
                last.unwrap_or(u64::MAX)
                    .min(first.saturating_add(CHUNK - 1))
            };
            format!("bytes={first}-{end}")
        }
        (None, Some(Range::Suffix(count))) => format!("bytes=-{count}"),
        _ => unreachable!(),
    };
    let upstream = fetch(&client, &address, &upstream_range).await?;
    let status = upstream.status();
    let mut response = Response::builder().header(header::CACHE_CONTROL, "private, no-store");
    for name in [header::CONTENT_TYPE, header::ACCEPT_RANGES] {
        if let Some(value) = upstream.headers().get(&name) {
            response = response.header(name, value);
        }
    }
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        if let Some(value) = upstream.headers().get(header::CONTENT_RANGE) {
            response = response.header(header::CONTENT_RANGE, value);
        }
        return response
            .status(status)
            .header(header::CONTENT_LENGTH, "0")
            .body(Body::empty())
            .map_err(|_| ApiError::conflict("Invalid public video response"));
    }
    // The client receives its whole request, in bounded upstream chunks.
    let span = match (status, first, content_range(&upstream)) {
        (StatusCode::PARTIAL_CONTENT, Some(first), Some((from, to, total)))
            if from == first && to >= from && total > to =>
        {
            Some((first, to, last.unwrap_or(total - 1).min(total - 1), total))
        }
        _ => None,
    };
    let Some((first, received, end, total)) = span else {
        // A suffix, or a provider that answered differently: pass it through.
        for name in [header::CONTENT_LENGTH, header::CONTENT_RANGE] {
            if let Some(value) = upstream.headers().get(&name) {
                response = response.header(name, value);
            }
        }
        let body = if head {
            Body::empty()
        } else {
            body(upstream, None, permit)
        };
        return response
            .status(status)
            .body(body)
            .map_err(|_| ApiError::conflict("Invalid public video response"));
    };
    response = if requested.is_none() {
        response.status(StatusCode::OK)
    } else {
        response.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!("bytes {first}-{end}/{total}"),
        )
    };
    response = response.header(header::CONTENT_LENGTH, HeaderValue::from(end - first + 1));
    let body = if head {
        Body::empty()
    } else {
        body(
            upstream,
            Some(Remaining {
                client,
                address,
                next: received + 1,
                end,
            }),
            permit,
        )
    };
    response
        .body(body)
        .map_err(|_| ApiError::conflict("Invalid public video response"))
}
async fn fetch(client: &reqwest::Client, address: &str, range: &str) -> Result<reqwest::Response> {
    let upstream = client
        .get(address)
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, range)
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
    Ok(upstream)
}
/// The chunks that follow the first upstream response.
struct Remaining {
    client: reqwest::Client,
    address: String,
    next: u64,
    end: u64,
}
type Chunks = futures_util::stream::BoxStream<'static, reqwest::Result<axum::body::Bytes>>;
fn body(
    upstream: reqwest::Response,
    remaining: Option<Remaining>,
    permit: OwnedSemaphorePermit,
) -> Body {
    // Keep the slot until the reader drops its body. Backpressure prevents
    // buffering a whole video in server memory; errors never contain URLs.
    let interrupted = || std::io::Error::other("The public video connection was interrupted");
    let state: (Chunks, Option<Remaining>, OwnedSemaphorePermit) =
        (upstream.bytes_stream().boxed(), remaining, permit);
    Body::from_stream(futures_util::stream::unfold(
        Some(state),
        move |state| async move {
            let (mut chunks, mut remaining, permit) = state?;
            loop {
                match chunks.next().await {
                    Some(Ok(bytes)) => return Some((Ok(bytes), Some((chunks, remaining, permit)))),
                    Some(Err(_)) => return Some((Err(interrupted()), None)),
                    None => {}
                }
                let rest = remaining.as_mut().filter(|rest| rest.next <= rest.end)?;
                let last = rest.end.min(rest.next.saturating_add(CHUNK - 1));
                let next = fetch(
                    &rest.client,
                    &rest.address,
                    &format!("bytes={}-{last}", rest.next),
                )
                .await
                .ok()
                .filter(|r| {
                    r.status() == StatusCode::PARTIAL_CONTENT
                        && content_range(r)
                            .is_some_and(|(from, to, _)| from == rest.next && to == last)
                });
                let Some(next) = next else {
                    return Some((Err(interrupted()), None));
                };
                rest.next = last + 1;
                chunks = next.bytes_stream().boxed();
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirects_stay_on_the_https_youtube_cdn_and_are_bounded() {
        for address in [
            "https://googlevideo.com/videoplayback?signature=public",
            "https://rr4---sn-example.googlevideo.com/videoplayback?signature=public",
        ] {
            let url = reqwest::Url::parse(address).unwrap();
            assert!(cdn_redirect(&url, 1));
            assert!(cdn_redirect(&url, 5));
            assert!(!cdn_redirect(&url, 6));
        }
        for address in [
            "http://r1.googlevideo.com/video",
            "https://googlevideo.com.attacker.test/video",
            "https://attacker.test/googlevideo.com/video",
            "https://127.0.0.1/video",
            "https://[::1]/video",
            "https://r1.googlevideo.com:444/video",
            "https://user:password@r1.googlevideo.com/video",
            "https://r1.ttvnw.net/video",
        ] {
            assert!(!cdn_redirect(&reqwest::Url::parse(address).unwrap(), 1));
        }
    }

    #[test]
    fn ranges_support_seek_and_reject_multipart_and_overflow() {
        for (value, expected) in [
            ("bytes=0-", Range::From(0, None)),
            ("bytes=100-199", Range::From(100, Some(199))),
            ("bytes=-4096", Range::Suffix(4096)),
            ("bytes=0-0", Range::From(0, Some(0))),
        ] {
            assert_eq!(range(value), Some(expected), "{value}");
        }
        for value in [
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
            assert_eq!(range(value), None, "{value}");
        }
    }
}
