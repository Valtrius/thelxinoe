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

/// A photo-like image: gradients, curves and hard edges.
fn photo(width: u32, height: u32, gray: bool) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
        let ring = ((fx - 0.5).hypot(fy - 0.5) * 40.0).sin() * 60.0;
        image::Rgb([
            (fx * 200.0 + ring).clamp(0.0, 255.0) as u8,
            (fy * 200.0 - ring).clamp(0.0, 255.0) as u8,
            if (x / 40 + y / 40) % 2 == 0 { 40 } else { 210 },
        ])
    });
    let image = if gray {
        DynamicImage::ImageLuma8(DynamicImage::ImageRgb8(image).to_luma8())
    } else {
        DynamicImage::ImageRgb8(image)
    };
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Jpeg).unwrap();
    bytes.into_inner()
}

/// A black JPEG with blue rows.
fn lined(width: u32, height: u32, rows: &[u32]) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |_, y| {
        image::Rgb(if rows.contains(&y) {
            [0, 0, 255]
        } else {
            [0, 0, 0]
        })
    });
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Jpeg).unwrap();
    bytes.into_inner()
}

/// The JPEG with an Adobe segment naming how its colours are stored.
fn adobe(bytes: &[u8], transform: u8) -> Vec<u8> {
    let mut segment = vec![0xFF, 0xEE, 0, 14];
    segment.extend_from_slice(b"Adobe");
    segment.extend_from_slice(&[0, 100, 0, 0, 0, 0, transform]);
    [&bytes[..2], &segment, &bytes[2..]].concat()
}

/// The card a whole decode makes, as before JPEGs were decoded scaled.
fn whole(bytes: &[u8], width: u32) -> DynamicImage {
    let decoded = image::load_from_memory(bytes).unwrap();
    decoded.resize(
        width.min(decoded.width()),
        decoded.height(),
        image::imageops::FilterType::Triangle,
    )
}

fn psnr(a: &DynamicImage, b: &DynamicImage) -> f64 {
    let (a, b) = (a.to_rgb8(), b.to_rgb8());
    let error = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum::<f64>()
        / a.as_raw().len() as f64;
    10.0 * (255.0f64.powi(2) / error).log10()
}

#[test]
fn large_jpegs_decode_scaled_to_the_size_and_look_of_a_whole_decode() {
    for (width, height, card, gray) in [
        (2000, 3000, 500, false),
        (2000, 3000, 1000, false),
        (3840, 2160, 500, false),
        (1440, 2160, 500, true),
        (1000, 1500, 500, false),
        (3996, 1080, 1000, false),
    ] {
        let bytes = photo(width, height, gray);
        let scaled = scaled_jpeg(&bytes, card).unwrap();
        let before = whole(&bytes, card);
        assert_eq!(scaled.dimensions(), before.dimensions(), "{width}x{height}");
        assert_eq!(scaled.color(), before.color(), "{width}x{height}");
        // The two downscale differently, most at hard edges such as these squares. A
        // swapped channel, a shift or a wrong scale lands far below this.
        let quality = psnr(&scaled, &before);
        assert!(quality > 20.0, "{width}x{height}: {quality} dB");
    }
    let bytes = adobe(&photo(2000, 3000, false), 1);
    let quality = psnr(&scaled_jpeg(&bytes, 500).unwrap(), &whole(&bytes, 500));
    assert!(quality > 20.0, "Adobe YCbCr: {quality} dB");
}

#[test]
fn jpegs_scaling_would_not_shrink_or_beyond_limits_decode_whole() {
    // Less than twice as wide as the card.
    assert!(scaled_jpeg(&photo(999, 1500, false), 500).is_none());
    // Wider than `image` decodes.
    assert!(scaled_jpeg(&photo(8200, 100, false), 500).is_none());
    assert!(scaled_jpeg(b"invalid image", 500).is_none());
    // Lossless JPEGs have no IDCT to scale.
    assert!(scaled_jpeg(include_bytes!("lossless.jpg"), 32).is_none());
    // `image` reads these colours as stored, where this decoder would convert them.
    assert!(scaled_jpeg(&adobe(&photo(2000, 3000, false), 0), 500).is_none());
    // No scale covers the card with whole MCUs.
    for (width, height, card) in [
        (1999, 3001, 500),
        (1160, 2175, 500),
        (2320, 2349, 1000),
        (4000, 9, 500),
        (1000, 1, 500),
    ] {
        assert!(
            scaled_jpeg(&photo(width, height, false), card).is_none(),
            "{width}x{height}"
        );
    }
}

#[test]
fn a_cards_edge_is_read_like_the_rest_of_it() {
    // The same line at the same place in its MCU, inside the image and at its edge.
    let card = scaled_jpeg(&lined(2000, 3000, &[1495, 2999]), 500)
        .unwrap()
        .to_rgb8();
    let blue = |row: u32| {
        (0..card.width())
            .map(|x| f64::from(card.get_pixel(x, row)[2]))
            .sum::<f64>()
            / f64::from(card.width())
    };
    let (inside, edge) = (blue(1495 / 4), blue(2999 / 4));
    assert!(
        inside > 32.0 && (edge - inside).abs() < 4.0,
        "{edge} against {inside}"
    );
    // At the edge of a partial MCU, a scaled row would mostly be the encoder's padding.
    assert!(scaled_jpeg(&lined(2000, 3001, &[3000]), 500).is_none());
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
