use crate::test_support::fixture;
use axum::{
    body::Body,
    http::{Request, Response, StatusCode, header},
};
use http_body_util::BodyExt;
use std::io::Read;
use tower::ServiceExt;

async fn get(
    state: &crate::AppState,
    path: &str,
    headers: &[(header::HeaderName, &str)],
) -> (Response<Body>, Vec<u8>) {
    let mut request = Request::builder()
        .uri(path)
        .header("host", "internal:8484")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "media.test");
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    let mut request = request.body(Body::empty()).unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = crate::router(state.clone()).oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (Response::from_parts(parts, Body::empty()), bytes)
}

fn decode(encoding: &str, bytes: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    match encoding {
        "br" => brotli::Decompressor::new(bytes, 4096)
            .read_to_end(&mut decoded)
            .unwrap(),
        "gzip" => flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut decoded)
            .unwrap(),
        other => panic!("unexpected encoding {other}"),
    };
    decoded
}

#[tokio::test]
async fn api_data_compresses_for_clients_that_accept_it() {
    let (_temp, state, cookie) = fixture().await;
    let (plain, expected) = get(
        &state,
        "/api/v1/timezones",
        &[(header::COOKIE, cookie.as_str())],
    )
    .await;
    assert_eq!(plain.status(), StatusCode::OK);
    assert!(!plain.headers().contains_key(header::CONTENT_ENCODING));
    for (accepted, encoding) in [("gzip, deflate, br", "br"), ("gzip", "gzip")] {
        let (response, bytes) = get(
            &state,
            "/api/v1/timezones",
            &[
                (header::COOKIE, cookie.as_str()),
                (header::ACCEPT_ENCODING, accepted),
            ],
        )
        .await;
        assert_eq!(response.headers()[header::CONTENT_ENCODING], encoding);
        assert_eq!(response.headers()[header::VARY], "accept-encoding");
        assert!(bytes.len() * 2 < expected.len(), "{encoding} barely shrank");
        assert_eq!(decode(encoding, &bytes), expected);
    }
    // Small answers are not worth the encoding.
    let (response, _) = get(&state, "/api/v1/health", &[(header::ACCEPT_ENCODING, "br")]).await;
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
}

#[tokio::test]
async fn media_ranges_and_images_pass_through_untouched() {
    let (_temp, state, _) = fixture().await;
    let assets = state.config.web.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    let script = "export const value = 'compressible';\n".repeat(200);
    std::fs::write(assets.join("app.js"), &script).unwrap();
    std::fs::write(assets.join("clip.mp4"), vec![7u8; 8192]).unwrap();
    std::fs::write(assets.join("poster.png"), vec![7u8; 8192]).unwrap();
    // A web build without precompressed copies is still compressed on the fly.
    let (response, bytes) = get(
        &state,
        "/assets/app.js",
        &[(header::ACCEPT_ENCODING, "gzip")],
    )
    .await;
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    assert_eq!(
        response.headers().get_all(header::VARY).iter().count(),
        1,
        "{:?}",
        response.headers()
    );
    assert_eq!(decode("gzip", &bytes), script.as_bytes());
    let (response, bytes) = get(
        &state,
        "/assets/app.js",
        &[
            (header::ACCEPT_ENCODING, "gzip"),
            (header::RANGE, "bytes=0-9"),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
    assert_eq!(bytes, &script.as_bytes()[..10]);
    for path in ["/assets/clip.mp4", "/assets/poster.png"] {
        let (response, bytes) = get(&state, path, &[(header::ACCEPT_ENCODING, "br, gzip")]).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            !response.headers().contains_key(header::CONTENT_ENCODING),
            "{path}"
        );
        assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes", "{path}");
        assert_eq!(bytes.len(), 8192, "{path}");
    }
}

#[test]
fn only_text_formats_and_owned_responses_compress() {
    let check = |kind: Option<&str>, native: bool| {
        let mut headers = axum::http::HeaderMap::new();
        if let Some(kind) = kind {
            headers.insert(header::CONTENT_TYPE, kind.parse().unwrap());
        }
        let mut extensions = axum::http::Extensions::new();
        if native {
            extensions.insert(crate::managers::access::NativeResponse);
        }
        crate::compressible(
            StatusCode::OK,
            axum::http::Version::HTTP_11,
            &headers,
            &extensions,
        )
    };
    for kind in [
        "application/json",
        "text/html; charset=utf-8",
        "text/css",
        "text/javascript",
        "text/vtt",
        "image/svg+xml",
        "application/manifest+json",
        "application/vnd.apple.mpegurl",
    ] {
        assert!(check(Some(kind), false), "{kind}");
    }
    for kind in [
        "video/mp4",
        "video/mp2t",
        "audio/mpeg",
        "image/jpeg",
        "image/webp",
        "text/event-stream",
        "application/zip",
        "application/octet-stream",
    ] {
        assert!(!check(Some(kind), false), "{kind}");
    }
    assert!(!check(None, false));
    // Service pages stream through the proxy as their service sent them.
    assert!(!check(Some("text/html"), true));
}
