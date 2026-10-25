use super::*;
use image::{DynamicImage, GenericImageView, ImageFormat};

fn save(path: &Path, width: u32, height: u32, png: bool) {
    let image = if png {
        DynamicImage::new_rgba8(width, height)
    } else {
        DynamicImage::new_rgb8(width, height)
    };
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(
            &mut bytes,
            if png {
                ImageFormat::Png
            } else {
                ImageFormat::Jpeg
            },
        )
        .unwrap();
    std::fs::write(path, bytes.into_inner()).unwrap();
}

#[tokio::test]
async fn variants_preserve_originals_and_alpha_and_invalidate_when_the_source_changes() {
    let dir = tempfile::tempdir().unwrap();
    for png in [false, true] {
        let source = dir.path().join(if png { "alpha" } else { "poster" });
        save(&source, 1600, 2400, png);
        let original = std::fs::read(&source).unwrap();
        let (small, concurrent) = tokio::join!(
            card(source.clone(), Some(500)),
            card(source.clone(), Some(500))
        );
        assert_eq!(small, concurrent);
        let image = read(&small).unwrap();
        assert_eq!(image.dimensions(), (500, 750));
        assert_eq!(image.color().has_alpha(), png);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let modified = std::fs::metadata(&small).unwrap().modified().unwrap();
        assert_eq!(generate(&source, 500).unwrap(), small);
        assert_eq!(
            std::fs::metadata(&small).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(
            read(card(source.clone(), Some(1000)).await)
                .unwrap()
                .dimensions(),
            (1000, 1500)
        );
        save(&source, 600, 600, png);
        assert_eq!(
            read(card(source.clone(), Some(500)).await)
                .unwrap()
                .dimensions(),
            (500, 500)
        );
        assert_eq!(
            read(card(source.clone(), Some(1000)).await)
                .unwrap()
                .dimensions(),
            (600, 600)
        );
        assert_eq!(card(source.clone(), None).await, source);
    }
}

#[tokio::test]
async fn invalid_or_unwritable_cache_falls_back_and_orphans_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let originals = dir.path().join("artwork");
    std::fs::create_dir_all(&originals).unwrap();
    let source = originals.join("poster");
    std::fs::write(&source, b"invalid image").unwrap();
    assert_eq!(card(source.clone(), Some(500)).await, source);
    save(&source, 600, 900, false);
    std::fs::write(originals.join("cards"), b"not a directory").unwrap();
    assert_eq!(card(source.clone(), Some(500)).await, source);
    std::fs::remove_file(originals.join("cards")).unwrap();
    let variant = card(source.clone(), Some(500)).await;
    prune(dir.path()).await.unwrap();
    assert!(variant.exists());
    std::fs::remove_file(source).unwrap();
    prune(dir.path()).await.unwrap();
    assert_eq!(
        std::fs::read_dir(originals.join("cards")).unwrap().count(),
        0
    );
    // Video stills go after 30 days, to be taken again if still shown.
    let stills = originals.join("stills");
    std::fs::create_dir_all(&stills).unwrap();
    for (name, age) in [("recent", 29), ("old", 31)] {
        let file = std::fs::File::create(stills.join(name)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age * 86400))
            .unwrap();
    }
    prune(dir.path()).await.unwrap();
    assert!(stills.join("recent").exists());
    assert!(!stills.join("old").exists());
}

#[tokio::test]
async fn card_requests_keep_authentication_cache_headers_and_original_access() {
    use axum::http::{HeaderMap, StatusCode, header};
    use serde_json::Value;
    let (_temp, state, cookie) = crate::test_support::fixture().await;
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, cookie.parse().unwrap());
    let p = crate::security::principal(&state, &headers).await.unwrap();
    let grant = crate::grants::artwork(&state, &p).await.unwrap();
    let id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let dir = state.config.cache.join("artwork");
    std::fs::create_dir_all(&dir).unwrap();
    save(&dir.join(id), 1600, 2400, false);
    let url = format!("/api/v1/catalog/{id}/artwork?grant={grant}");
    for suffix in ["", "&width=500", "&width=1000"] {
        let result =
            crate::test_support::call(&state, &format!("{url}{suffix}"), "GET", Value::Null, "")
                .await;
        assert_eq!(result.0, StatusCode::OK);
        assert_eq!(result.1[header::CACHE_CONTROL], "private, max-age=86400");
        assert_eq!(result.1[header::CONTENT_TYPE], "image/jpeg");
    }
    let invalid =
        crate::test_support::call(&state, &format!("{url}&width=9999"), "GET", Value::Null, "")
            .await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
    assert_eq!(read(dir.join(id)).unwrap().dimensions(), (1600, 2400));
    let session = p.session_id;
    state
        .db
        .write("test.revoke", move |db| {
            db.execute("DELETE FROM sessions WHERE id=?1", [session])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        crate::test_support::call(&state, &format!("{url}&width=500"), "GET", Value::Null, "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

fn read(path: impl AsRef<Path>) -> anyhow::Result<DynamicImage> {
    Ok(ImageReader::open(path)?.with_guessed_format()?.decode()?)
}
