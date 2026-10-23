use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    JpegEncoder::new(&mut bytes)
        .encode_image(&image::RgbImage::new(width, height))
        .unwrap();
    bytes
}

#[tokio::test]
async fn simultaneous_sizes_and_restart_reuse_one_download() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::default();
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(jpeg(1280, 720))
    };
    let (small, medium, large) = tokio::join!(
        cache.get(dir.path(), "abcdefghijk", 320, fetch),
        cache.get(dir.path(), "abcdefghijk", 640, fetch),
        cache.get(dir.path(), "abcdefghijk", 1280, fetch),
    );
    for (bytes, size) in [
        (small.unwrap(), (320, 180)),
        (medium.unwrap(), (640, 360)),
        (large.unwrap(), (1280, 720)),
    ] {
        let image = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg)
            .decode()
            .unwrap();
        assert_eq!((image.width(), image.height()), size);
    }
    let restarted = Cache::default();
    restarted
        .get(dir.path(), "abcdefghijk", 640, fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let path = dir.path().join("abcdefghijk-640.jpg");
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - TTL - Duration::from_secs(1))
        .unwrap();
    restarted
        .get(dir.path(), "abcdefghijk", 640, fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn fallback_is_not_upscaled_and_malformed_images_are_rejected() {
    for (_, bytes) in resize(jpeg(320, 180)).unwrap() {
        let image = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg)
            .decode()
            .unwrap();
        assert_eq!((image.width(), image.height()), (320, 180));
    }
    assert!(resize(b"not an image".to_vec()).is_err());
    assert!(resize(jpeg(4097, 1)).is_err());
}

#[tokio::test]
async fn invalid_requests_and_failed_fetches_do_not_pollute_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::default();
    for (video, width) in [("../abcdefgh", 320), ("abcdefghijk", 42)] {
        assert!(
            cache
                .get(dir.path(), video, width, || async {
                    panic!("must not fetch")
                })
                .await
                .is_err()
        );
    }
    assert!(
        cache
            .get(dir.path(), "abcdefghijk", 320, || async {
                Err(ApiError::not_found())
            })
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    cache
        .get(dir.path(), "abcdefghijk", 320, || async {
            Ok(jpeg(320, 180))
        })
        .await
        .unwrap();
}

#[test]
fn eviction_removes_expired_images_then_oldest_to_fit_budget() {
    let dir = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    for (name, age) in [
        ("expired", TTL),
        ("old", Duration::from_secs(10)),
        ("new", Duration::from_secs(1)),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, [0; 10]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
    }
    prune(dir.path(), 10, 20, now).unwrap();
    assert!(!dir.path().join("expired").exists());
    assert!(!dir.path().join("old").exists());
    assert!(dir.path().join("new").exists());
}

#[tokio::test]
async fn cached_artwork_still_checks_the_session_owner_and_privacy() {
    use crate::{
        grants, security,
        test_support::{call, fixture},
    };
    use axum::http::{HeaderMap, StatusCode, header};
    use serde_json::Value;
    let (_temp, state, cookie) = fixture().await;
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse().unwrap());
    let p = security::principal(&state, &headers).await.unwrap();
    state.db.write("test.thumbnail", |db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('alice','abcdefghijk','Video','public'),('bob','bbbbbbbbbbb','Other video','public')", [])?;
        Ok(())
    }).await.unwrap();
    let dir = state.config.cache.join("youtube-thumbnails");
    store(&dir, "abcdefghijk", resize(jpeg(1280, 720)).unwrap()).unwrap();
    let grant = grants::youtube_artwork(&state, &p).await.unwrap();
    let url = format!("/api/v1/online/youtube/videos/abcdefghijk/artwork?grant={grant}&width=320");
    let response = call(&state, &url, "GET", Value::Null, "").await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1[header::CACHE_CONTROL], "private, max-age=86400");
    assert_eq!(response.1[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(
        call(
            &state,
            &url.replace("width=320", "width=42"),
            "GET",
            Value::Null,
            ""
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &url.replace("abcdefghijk", "bbbbbbbbbbb"),
            "GET",
            Value::Null,
            ""
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    state
        .db
        .write("test.private", |db| {
            db.execute(
                "UPDATE youtube_videos SET privacy='private' WHERE video_id='abcdefghijk'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(&state, &url, "GET", Value::Null, "").await.0,
        StatusCode::NOT_FOUND
    );
    state
        .db
        .write("test.revoke", |db| {
            db.execute("DELETE FROM sessions WHERE user_id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(&state, &url, "GET", Value::Null, "").await.0,
        StatusCode::UNAUTHORIZED
    );
}
