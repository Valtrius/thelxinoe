use anyhow::{Context, ensure};
use image::{
    DynamicImage, GrayImage, ImageFormat, ImageReader, Limits, RgbImage, codecs::jpeg::JpegEncoder,
    imageops::FilterType,
};
use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tokio::sync::{Mutex, Semaphore};

pub(crate) fn validate_width(width: Option<u32>) -> crate::error::Result<()> {
    if width.is_some_and(|width| ![500, 1000].contains(&width)) {
        return Err(crate::error::ApiError::bad(
            "Choose an artwork width of 500 or 1000 pixels",
        ));
    }
    Ok(())
}

pub(crate) async fn card(source: PathBuf, width: Option<u32>) -> PathBuf {
    let Some(width) = width else {
        return source;
    };
    if let Some(path) = cached(&source, width).await {
        return path;
    }
    static LOCKS: [Mutex<()>; 64] = [const { Mutex::const_new(()) }; 64];
    static SLOTS: Semaphore = Semaphore::const_new(4);
    let stripe = source
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(*byte as usize)
        })
        % LOCKS.len();
    let lock = LOCKS[stripe].lock().await;
    let Ok(slot) = SLOTS.acquire().await else {
        return source;
    };
    let original = source.clone();
    match tokio::task::spawn_blocking(move || {
        let (_lock, _slot) = (lock, slot);
        generate(&source, width)
    })
    .await
    {
        Ok(Ok(path)) => path,
        result => {
            tracing::debug!(
                ?result,
                "Serving original artwork because its card variant is unavailable"
            );
            original
        }
    }
}

fn stamp(metadata: &std::fs::Metadata, width: u32) -> std::io::Result<String> {
    Ok(format!(
        "v1:{width}:{}:{:?}",
        metadata.len(),
        metadata.modified()?
    ))
}

async fn cached(source: &Path, width: u32) -> Option<PathBuf> {
    let metadata = tokio::fs::metadata(source).await.ok()?;
    let stamp = stamp(&metadata, width).ok()?;
    let name = source.file_name()?.to_string_lossy();
    let dir = source.parent()?.join("cards");
    let target = dir.join(format!("{name}.{width}"));
    let version = tokio::fs::read_to_string(dir.join(format!("{name}.{width}.source")))
        .await
        .ok()?;
    (version == stamp && tokio::fs::try_exists(&target).await.ok()?).then_some(target)
}

fn generate(source: &Path, width: u32) -> anyhow::Result<PathBuf> {
    let mut file = std::fs::File::open(source)?;
    let metadata = file.metadata()?;
    let stamp = stamp(&metadata, width)?;
    let name = source
        .file_name()
        .context("Artwork has no name")?
        .to_string_lossy();
    let dir = source
        .parent()
        .context("Artwork has no parent")?
        .join("cards");
    let target = dir.join(format!("{name}.{width}"));
    let version = dir.join(format!("{name}.{width}.source"));
    if target.is_file() && std::fs::read_to_string(&version).is_ok_and(|value| value == stamp) {
        return Ok(target);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Artwork exceeds size limit"
    );
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    reader.limits(limits);
    let scaled = if reader.format() == Some(ImageFormat::Jpeg) {
        scaled_jpeg(&bytes, width)
    } else {
        None
    };
    let resized = match scaled {
        Some(resized) => resized,
        None => {
            let decoded = reader.decode()?;
            decoded.resize(
                width.min(decoded.width()),
                decoded.height(),
                FilterType::Triangle,
            )
        }
    };
    let mut output = Vec::new();
    if resized.color().has_alpha() {
        resized.write_to(&mut Cursor::new(&mut output), image::ImageFormat::Png)?;
    } else {
        JpegEncoder::new_with_quality(&mut output, 85).encode_image(&resized.to_rgb8())?;
    }
    std::fs::create_dir_all(&dir)?;
    ensure!(
        fs2::available_space(&dir)? >= 512 * 1024 * 1024,
        "Insufficient artwork cache space"
    );
    atomic_write(&target, &output)?;
    // The stamp describes the opened original, so a concurrent replacement is
    // detected on the next request even if it arrived during this conversion.
    atomic_write(&version, stamp.as_bytes())?;
    Ok(target)
}

const MAX_SIDE: u32 = 8192;
const MAX_ALLOC: u64 = 128 * 1024 * 1024;

/// A JPEG at least twice as wide as its card, decoded at the smallest IDCT scale that
/// still covers the card, so most of its pixels are never made. The card has the size
/// and colours a whole decode would give it. What this decoder can't read as `image`
/// would is left to `image`.
fn scaled_jpeg(bytes: &[u8], width: u32) -> Option<DynamicImage> {
    use jpeg_decoder::{CodingProcess, ColorTransform, PixelFormat};
    use zune_jpeg::{
        SampleRatios,
        zune_core::{colorspace::ColorSpace, options::DecoderOptions},
    };
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let (original_width, original_height) = (u32::from(info.width), u32::from(info.height));
    let pixel_bytes = info.pixel_format.pixel_bytes() as u64;
    if !matches!(
        info.coding_process,
        CodingProcess::DctSequential | CodingProcess::DctProgressive
    ) || original_width < width * 2
        || original_width > MAX_SIDE
        || original_height > MAX_SIDE
        || u64::from(original_width) * u64::from(original_height) * pixel_bytes > MAX_ALLOC
    {
        return None;
    }
    // The colours `image` reads it as, from the headers its decoder reads the same way.
    // The two decoders can disagree, as on Adobe RGB files numbering their components
    // like YCbCr.
    let options = DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX);
    let mut headers = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), options);
    headers.decode_headers().ok()?;
    decoder.set_color_transform(match (headers.input_colorspace()?, info.pixel_format) {
        (ColorSpace::YCbCr, PixelFormat::RGB24) => ColorTransform::YCbCr,
        (ColorSpace::Luma, PixelFormat::L8) => ColorTransform::Grayscale,
        _ => return None,
    });
    // The size `DynamicImage::resize` gives it, computed the same way.
    let ratio = f64::from(width) / f64::from(original_width);
    let side = |length: u32| ((f64::from(length) * ratio).round() as u32).max(1);
    let (width, height) = (side(original_width), side(original_height));
    // The smallest IDCT scale, in eighths, that covers the card with whole MCUs. A partial
    // MCU would give its blocks' edge padding the area of real pixels.
    let (h_max, v_max) = match headers.info()?.sample_ratio {
        SampleRatios::None => (1, 1),
        SampleRatios::H => (2, 1),
        SampleRatios::V => (1, 2),
        SampleRatios::HV => (2, 2),
        SampleRatios::Generic(h_max, v_max) => (h_max as u32, v_max as u32),
    };
    let eighths = [1, 2, 4].into_iter().find(|eighths| {
        original_width * eighths % (8 * h_max) == 0
            && original_height * eighths % (8 * v_max) == 0
            && original_width * eighths / 8 >= width
            && original_height * eighths / 8 >= height
    })?;
    let (scaled_width, scaled_height) =
        (original_width * eighths / 8, original_height * eighths / 8);
    let scaled = decoder
        .scale(scaled_width as u16, scaled_height as u16)
        .ok()?;
    if scaled != (scaled_width as u16, scaled_height as u16) {
        return None;
    }
    let pixels = decoder.decode().ok()?;
    let image = match decoder.info()?.pixel_format {
        PixelFormat::RGB24 => {
            DynamicImage::ImageRgb8(RgbImage::from_raw(scaled_width, scaled_height, pixels)?)
        }
        PixelFormat::L8 => {
            DynamicImage::ImageLuma8(GrayImage::from_raw(scaled_width, scaled_height, pixels)?)
        }
        _ => return None,
    };
    Some(if (scaled_width, scaled_height) == (width, height) {
        image
    } else {
        image.resize_exact(width, height, FilterType::Triangle)
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing cache directory")?)?;
    temp.write_all(bytes)?;
    temp.persist(path)?;
    Ok(())
}

pub(crate) async fn prune(cache: &Path) -> anyhow::Result<()> {
    let cache = cache.to_owned();
    let originals = cache.join("artwork");
    tokio::task::spawn_blocking(move || {
        remove_files(&originals.join("cards"), |name, old| {
            let base = name.strip_suffix(".source").unwrap_or(name);
            let source = base
                .rsplit_once('.')
                .filter(|(_, size)| matches!(*size, "500" | "1000"))
                .map(|(name, _)| originals.join(name));
            old || source.is_some_and(|source| !source.is_file())
        })?;
        // Video stills are taken again when next shown, so old ones of removed media go.
        remove_files(&originals.join("stills"), |_, old| old)?;
        // So is music discovery artwork.
        let music = cache.join("music");
        remove_files(&music.join("cards"), |_, old| old)?;
        remove_files(&music, |_, old| old)
    })
    .await?
}

/// Delete the files of a cache directory chosen by name and by being 30 days old.
fn remove_files(dir: &Path, unwanted: impl Fn(&str, bool) -> bool) -> anyhow::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let old = SystemTime::now()
            .duration_since(entry.metadata()?.modified()?)
            .unwrap_or_default()
            >= Duration::from_secs(30 * 86400);
        if unwanted(&entry.file_name().to_string_lossy(), old) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
