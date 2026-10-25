//! Stills taken from the video file of a movie or episode that has no artwork.

use crate::{
    AppState,
    error::{ApiError, Result},
};
use anyhow::Context;
use axum::{extract::Request, response::Response};
use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use serde_json::Value;
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use thelxinoe_playback::Source;
use tokio::{
    io::AsyncReadExt,
    sync::{Mutex, Semaphore},
};

const WIDTH: u32 = 640;
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// A page of new cards takes its stills a couple at a time, at low priority.
static SLOTS: Semaphore = Semaphore::const_new(2);
static LOCKS: [Mutex<()>; 64] = [const { Mutex::const_new(()) }; 64];

/// Serve the still of a media item's first file, extracting it once per file version.
pub(super) async fn serve(state: &AppState, media: &str, request: Request) -> Result<Response> {
    let source = crate::playback::source(state, media, None).await?;
    let video = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .cloned()
        .ok_or_else(ApiError::not_found)?;
    let dir = state.config.cache.join("artwork").join("stills");
    let path = dir.join(media);
    // The extraction completes and is cached even if the client stops waiting.
    let task = {
        let state = state.clone();
        let path = path.clone();
        tokio::spawn(async move { ensure(&state, &source, &video, &path).await })
    };
    match task.await.map_err(anyhow::Error::from)? {
        Ok(true) => super::catalog::serve(path, request).await,
        Ok(false) => Err(ApiError::not_found()),
        Err(error) => {
            tracing::debug!(%error, "Video still is unavailable");
            Err(ApiError::not_found())
        }
    }
}

/// Whether the still of this file version is on disk. A file ffmpeg could not take a
/// still from is not decoded again for a day.
async fn ensure(
    state: &AppState,
    source: &Source,
    video: &Value,
    path: &Path,
) -> anyhow::Result<bool> {
    let stamp = format!("{}:{}", source.id, source.generation);
    let marker = PathBuf::from(format!("{}.source", path.display()));
    let stripe = source.media_id.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    }) % LOCKS.len();
    let failed = format!("failed:{stamp}");
    let _lock = LOCKS[stripe].lock().await;
    let recent = tokio::fs::metadata(&marker)
        .await
        .ok()
        .and_then(|meta| meta.modified().ok()?.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(86400));
    match tokio::fs::read_to_string(&marker).await {
        Ok(current) if current == failed && recent => return Ok(false),
        Ok(current) if current == stamp && tokio::fs::try_exists(path).await? => return Ok(true),
        _ => {}
    }
    let _slot = SLOTS.acquire().await?;
    let still = extract(state, source, video).await?;
    let dir = path
        .parent()
        .context("Missing stills directory")?
        .to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        anyhow::ensure!(
            fs2::available_space(&dir)? >= 512 * 1024 * 1024,
            "Insufficient artwork cache space"
        );
        let found = still.is_some();
        match still {
            Some(bytes) => {
                write(&dir, &path, &bytes)?;
                write(&dir, &marker, stamp.as_bytes())?;
            }
            None => write(&dir, &marker, failed.as_bytes())?,
        }
        Ok(found)
    })
    .await?
}

fn write(dir: &Path, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.persist(path)?;
    Ok(())
}

/// A JPEG of the keyframe a tenth of the way in, past most cold opens and title cards;
/// `None` when ffmpeg finds no picture there.
async fn extract(
    state: &AppState,
    source: &Source,
    video: &Value,
) -> anyhow::Result<Option<Vec<u8>>> {
    source.validate().await?;
    let tools = state.tools.runtime.media()?;
    tools.ffmpeg.verify_unchanged().await?;
    let index = video["index"]
        .as_i64()
        .context("Video stream has no index")?;
    let duration = source.duration();
    let at = if duration.is_finite() && duration > 0.0 {
        duration / 10.0
    } else {
        0.0
    };
    // Display proportions, so anamorphic pictures are not squeezed.
    let mut filter = format!("scale=w='min({WIDTH},iw*sar)':h='ow/dar'");
    if ["smpte2084", "arib-std-b67"].contains(&video["color_transfer"].as_str().unwrap_or("")) {
        filter.push_str(",zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=p=bt709:t=bt709");
    }
    filter.push_str(",format=rgb24");
    #[cfg(unix)]
    let mut command = {
        let mut c = tokio::process::Command::new("nice");
        c.args(["-n", "15"]).arg(&tools.ffmpeg.path);
        c
    };
    #[cfg(not(unix))]
    let mut command = tokio::process::Command::new(&tools.ffmpeg.path);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            // Only the keyframe at or before the position is decoded.
            "-noaccurate_seek",
            "-ss",
            &format!("{at:.3}"),
            "-i",
        ])
        .arg(&source.path)
        .args([
            "-map",
            &format!("0:{index}"),
            "-frames:v",
            "1",
            "-vf",
            &filter,
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().context("Missing ffmpeg output")?;
    let (png, status) = tokio::time::timeout(Duration::from_secs(60), async {
        let mut png = Vec::new();
        (&mut stdout)
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut png)
            .await?;
        anyhow::ensure!(
            png.len() as u64 <= OUTPUT_LIMIT,
            "Video still exceeds size limit"
        );
        Ok::<_, anyhow::Error>((png, child.wait().await?))
    })
    .await
    .context("Video still extraction timed out")??;
    if !status.success() || png.is_empty() {
        return Ok(None);
    }
    Ok(tokio::task::spawn_blocking(move || jpeg(png).ok()).await?)
}

fn jpeg(png: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
    reader.limits(limits);
    let picture = reader.decode()?;
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 85).encode_image(&picture.to_rgb8())?;
    Ok(bytes)
}
