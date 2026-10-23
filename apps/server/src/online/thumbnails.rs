use super::{artwork_bytes, sync};
use crate::{
    AppState,
    error::{ApiError, Result},
};
use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use std::{
    io::{Cursor, Write},
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::{Mutex, Semaphore};

const WIDTHS: [u32; 3] = [320, 640, 1280];
const TTL: Duration = Duration::from_secs(86400);
const CAPACITY: u64 = 256 * 1024 * 1024;

pub(super) struct Cache {
    // Fixed stripes bound lock memory while coalescing requests for the same video.
    videos: [Mutex<()>; 64],
    writers: Arc<Mutex<()>>,
    decoders: Arc<Semaphore>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            videos: std::array::from_fn(|_| Mutex::new(())),
            writers: Arc::new(Mutex::new(())),
            decoders: Arc::new(Semaphore::new(4)),
        }
    }
}

pub(super) async fn load(
    state: &AppState,
    video: &str,
    width: u32,
) -> Result<(&'static str, Vec<u8>)> {
    state
        .online
        .thumbnails
        .get(
            &state.config.cache.join("youtube-thumbnails"),
            video,
            width,
            || async {
                // The medium fallback is wide; hqdefault and sddefault contain baked-in bars.
                for variant in ["maxresdefault", "mqdefault"] {
                    if let Ok((_, bytes)) = artwork_bytes(
                        state,
                        &format!("https://i.ytimg.com/vi/{video}/{variant}.jpg"),
                    )
                    .await
                    {
                        return Ok(bytes);
                    }
                }
                Err(ApiError::not_found())
            },
        )
        .await
        .map(|bytes| ("image/jpeg", bytes))
}

impl Cache {
    async fn get<F, Fut>(&self, dir: &Path, video: &str, width: u32, fetch: F) -> Result<Vec<u8>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>>>,
    {
        if !sync::identifier(video, 11) {
            return Err(ApiError::not_found());
        }
        if !WIDTHS.contains(&width) {
            return Err(ApiError::bad(
                "Choose a thumbnail width of 320, 640 or 1280 pixels",
            ));
        }
        let stripe = video.bytes().fold(0usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(byte as usize)
        }) % self.videos.len();
        let _video = self.videos[stripe].lock().await;
        let path = dir.join(format!("{video}-{width}.jpg"));
        if let Ok(meta) = tokio::fs::metadata(&path).await
            && meta
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age < TTL)
            && let Ok(bytes) = tokio::fs::read(&path).await
        {
            return Ok(bytes);
        }
        let bytes = fetch().await?;
        let permit = self
            .decoders
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::not_found())?;
        let images = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            resize(bytes)
        })
        .await
        .map_err(anyhow::Error::from)?
        .map_err(|_| ApiError::not_found())?;
        let selected = images
            .iter()
            .find(|(size, _)| *size == width)
            .unwrap()
            .1
            .clone();
        let writer = self.writers.clone().lock_owned().await;
        let dir = dir.to_owned();
        let video = video.to_owned();
        // Cache failures must not turn an otherwise available thumbnail into a broken image.
        if let Err(error) = tokio::task::spawn_blocking(move || {
            let _writer = writer;
            store(&dir, &video, images)
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|r| r)
        {
            tracing::warn!(%error, "YouTube thumbnail cache could not be written");
        }
        Ok(selected)
    }
}

fn resize(bytes: Vec<u8>) -> anyhow::Result<Vec<(u32, Vec<u8>)>> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    reader.limits(limits);
    let source = reader.decode()?;
    WIDTHS
        .into_iter()
        .map(|width| {
            // Never upscale the 320 px fallback or alter the source aspect ratio.
            let image = source.resize(
                width.min(source.width()),
                source.height(),
                image::imageops::FilterType::Triangle,
            );
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, 85).encode_image(&image.to_rgb8())?;
            Ok((width, bytes))
        })
        .collect()
}

fn prune(dir: &Path, incoming: u64, capacity: u64, now: SystemTime) -> anyhow::Result<()> {
    let mut files = Vec::new();
    let mut total = incoming;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified()?;
        if now.duration_since(modified).unwrap_or_default() >= TTL {
            std::fs::remove_file(entry.path())?;
        } else {
            total += meta.len();
            files.push((modified, meta.len(), entry.path()));
        }
    }
    files.sort_by_key(|file| file.0);
    for (_, len, path) in files {
        if total <= capacity {
            break;
        }
        std::fs::remove_file(path)?;
        total -= len;
    }
    Ok(())
}

fn store(dir: &Path, video: &str, images: Vec<(u32, Vec<u8>)>) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    anyhow::ensure!(
        fs2::available_space(dir)? >= 512 * 1024 * 1024,
        "Insufficient thumbnail cache space"
    );
    prune(
        dir,
        images.iter().map(|(_, bytes)| bytes.len() as u64).sum(),
        CAPACITY,
        SystemTime::now(),
    )?;
    for (width, bytes) in images {
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&bytes)?;
        temp.persist(dir.join(format!("{video}-{width}.jpg")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
