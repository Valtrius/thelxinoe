use anyhow::{Context, ensure};
use image::{ImageReader, Limits, codecs::jpeg::JpegEncoder};
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
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    let decoded = reader.decode()?;
    let resized = decoded.resize(
        width.min(decoded.width()),
        decoded.height(),
        image::imageops::FilterType::Triangle,
    );
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

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing cache directory")?)?;
    temp.write_all(bytes)?;
    temp.persist(path)?;
    Ok(())
}

pub(crate) async fn prune(cache: &Path) -> anyhow::Result<()> {
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
        remove_files(&originals.join("stills"), |_, old| old)
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
