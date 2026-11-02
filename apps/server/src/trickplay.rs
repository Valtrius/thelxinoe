//! Seek-bar previews of library videos: thumbnail sheets in Jellyfin's trickplay layout,
//! taken from keyframes by a background worker and kept per file version in the cache.

#[path = "storage/trickplay.rs"]
mod storage;
#[cfg(test)]
#[path = "trickplay_tests.rs"]
mod tests;

use crate::{
    AppState,
    error::{ApiError, Result},
};
use anyhow::Context;
use axum::{
    Router,
    extract::{Path, Query, Request, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::{Path as FilePath, PathBuf},
    process::Stdio,
    time::Duration,
};
use thelxinoe_core::Principal;
use thelxinoe_playback::Source;

/// Jellyfin's defaults: thumbnails 320 pixels wide, one every 10 seconds, in sheets of 10×10.
pub(crate) const WIDTH: u32 = 320;
const INTERVAL: u32 = 10;
const COLUMNS: u32 = 10;
const ROWS: u32 = 10;
const MANIFEST: &str = "trickplay.json";
const FAILED: &str = "failed";
/// FFmpeg's record of each thumbnail; the last sheet is padded, so it alone gives the count.
const FRAMES: &str = "frames";
const PARTIAL: &str = ".partial-";
/// A failed file is attempted again after this long.
const DAY: Duration = Duration::from_secs(86400);
const SPACE: u64 = 512 * 1024 * 1024;

/// A file version's sheets, as Jellyfin describes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Info {
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub thumbnail_count: u32,
    /// Milliseconds between thumbnails.
    pub interval: u32,
    /// The peak bit rate of the sheets when each thumbnail shows for one interval.
    pub bandwidth: u64,
}
impl Info {
    pub fn sheets(&self) -> u32 {
        self.thumbnail_count
            .div_ceil(self.tile_width * self.tile_height)
    }
}

fn root(state: &AppState) -> PathBuf {
    state.config.cache.join("trickplay")
}
fn name(file: &str, generation: &str) -> String {
    thelxinoe_auth::digest(&format!("{file}:{generation}"))
}
/// Jellyfin's folder name for one width's sheets.
fn sheets(dir: &FilePath) -> PathBuf {
    dir.join(format!("{WIDTH} - {COLUMNS}x{ROWS}"))
}
async fn read(dir: &FilePath) -> Option<Info> {
    serde_json::from_slice(&tokio::fs::read(dir.join(MANIFEST)).await.ok()?).ok()
}
/// The sheets of a file version, once they exist.
pub(crate) async fn info(state: &AppState, file: &str, generation: &str) -> Option<Info> {
    read(&root(state).join(name(file, generation))).await
}
/// Sheet `index` of a file version's sheets `width` pixels wide.
pub(crate) async fn sheet(
    state: &AppState,
    source: &Source,
    width: u32,
    index: u32,
) -> Option<PathBuf> {
    let dir = root(state).join(name(&source.id, &source.generation));
    let info = read(&dir).await?;
    (width == info.width && index < info.sheets())
        .then(|| sheets(&dir).join(format!("{index}.jpg")))
}

/// Jellyfin's `Trickplay` of each media item that has sheets: by media source, then width.
pub(crate) async fn jellyfin(
    state: &AppState,
    media: Vec<String>,
) -> anyhow::Result<HashMap<String, Value>> {
    let mut items = HashMap::<String, Map<String, Value>>::new();
    for (media, file, generation) in storage::files(&state.db, media).await? {
        if let Some(info) = info(state, &file, &generation).await {
            let widths = Map::from_iter([(info.width.to_string(), serde_json::to_value(info)?)]);
            items
                .entry(media)
                .or_default()
                .insert(file, Value::Object(widths));
        }
    }
    Ok(items
        .into_iter()
        .map(|(media, files)| (media, Value::Object(files)))
        .collect())
}

/// The web player's previews of a file version, or null before it has any.
pub(crate) async fn web(state: &AppState, p: &Principal, source: &Source) -> Result<Value> {
    let version = name(&source.id, &source.generation);
    let Some(info) = read(&root(state).join(&version)).await else {
        return Ok(Value::Null);
    };
    let grant = crate::grants::artwork(state, p).await?;
    Ok(json!({
        "width": info.width,
        "height": info.height,
        "columns": info.tile_width,
        "rows": info.tile_height,
        "interval": f64::from(info.interval) / 1000.0,
        "count": info.thumbnail_count,
        "sheets": (0..info.sheets())
            .map(|index| format!("/api/v1/trickplay/{version}/{index}?grant={grant}"))
            .collect::<Vec<_>>(),
    }))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/v1/trickplay/{version}/{index}", get(serve))
}
#[derive(Deserialize)]
struct Grant {
    grant: String,
}
/// A sheet for the web player, authorized like artwork.
async fn serve(
    State(state): State<AppState>,
    Path((version, index)): Path<(String, u32)>,
    Query(query): Query<Grant>,
    request: Request,
) -> Result<Response> {
    crate::grants::resolve(&state, &query.grant, "artwork", false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if version.len() != 64 || !version.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::not_found());
    }
    let dir = root(&state).join(version);
    let info = read(&dir).await.ok_or_else(ApiError::not_found)?;
    if index >= info.sheets() {
        return Err(ApiError::not_found());
    }
    let mut response = file(sheets(&dir).join(format!("{index}.jpg")), request).await?;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "private, max-age=86400".parse().unwrap(),
    );
    Ok(response)
}
/// Serves a sheet, byte ranges included.
pub(crate) async fn file(path: PathBuf, request: Request) -> Result<Response> {
    Ok(tower_http::services::ServeFile::new(path)
        .try_call(request)
        .await
        .map_err(anyhow::Error::from)?
        .into_response())
}

/// Whether this file version has sheets, or failed within a day.
async fn settled(dir: &FilePath) -> Option<bool> {
    if tokio::fs::try_exists(dir.join(MANIFEST)).await.ok()? {
        return Some(true);
    }
    let failed = tokio::fs::metadata(dir.join(FAILED)).await.ok()?;
    let age = failed.modified().ok()?.elapsed().unwrap_or_default();
    (age < DAY).then_some(false)
}

/// Takes the sheets of a file version. Playback stops it: it keeps nothing and returns false.
async fn generate(state: &AppState, source: &Source) -> anyhow::Result<bool> {
    let tools = state.tools.runtime.media()?;
    let root = root(state);
    tokio::fs::create_dir_all(&root).await?;
    anyhow::ensure!(
        fs2::available_space(&root)? >= SPACE,
        "Insufficient trickplay cache space"
    );
    let dir = root.join(name(&source.id, &source.generation));
    let partial = tempfile::Builder::new().prefix(PARTIAL).tempdir_in(&root)?;
    let info = match render(state, source, &tools, partial.path()).await {
        Ok(Some(info)) => info,
        Ok(None) => return Ok(false),
        Err(error) => {
            // A failure is recorded so the file isn't read again before it is retried.
            let marker = async {
                if tokio::fs::try_exists(&dir).await? {
                    tokio::fs::remove_dir_all(&dir).await?;
                }
                tokio::fs::create_dir_all(&dir).await?;
                tokio::fs::write(dir.join(FAILED), format!("{error:#}")).await
            };
            if let Err(marker) = marker.await {
                tracing::warn!("Failed trickplay could not be recorded: {marker:#}");
            }
            return Err(error);
        }
    };
    tokio::fs::write(partial.path().join(MANIFEST), serde_json::to_vec(&info)?).await?;
    if tokio::fs::try_exists(&dir).await? {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    tokio::fs::rename(partial.keep(), &dir).await?;
    Ok(true)
}

/// The scaling and color conversion from the video stream to thumbnails `WIDTH` wide at its
/// display proportions. HDR is tone mapped after scaling, so only thumbnails pay for it.
fn filter(video: &Value) -> String {
    let mut filter =
        format!("fps=1/{INTERVAL},scale=w={WIDTH}:h='max(2,trunc(ow/dar/2)*2)',setsar=1");
    let transfer = video["color_transfer"].as_str().unwrap_or_default();
    if ["smpte2084", "arib-std-b67"].contains(&transfer) {
        let primaries = video["color_primaries"]
            .as_str()
            .filter(|p| ["bt2020", "bt709"].contains(p))
            .unwrap_or("bt2020");
        let matrix = video["color_space"]
            .as_str()
            .filter(|m| ["bt2020nc", "bt709"].contains(m))
            .unwrap_or("bt2020nc");
        filter.push_str(&format!(",zscale=tin={transfer}:pin={primaries}:min={matrix}:t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=full"));
    }
    filter.push_str(",format=yuvj420p");
    filter
}
/// At least the time to play the file, or to read it at 4 MiB/s, plus a minute.
fn limit(source: &Source) -> Duration {
    let duration = source.duration();
    let playing = if duration.is_finite() && duration > 0.0 {
        duration as u64
    } else {
        0
    };
    Duration::from_secs(60 + playing.max(source.size / (4 * 1024 * 1024)))
}

/// Writes the sheets of `source` and FFmpeg's record of its thumbnails to `dir`, from
/// keyframes only. Returns `None` when it stopped for playback.
async fn render(
    state: &AppState,
    source: &Source,
    tools: &thelxinoe_tools::MediaTools,
    dir: &FilePath,
) -> anyhow::Result<Option<Info>> {
    let video = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .context("The file has no video")?;
    let index = video["index"]
        .as_i64()
        .context("Video stream has no index")?;
    source.validate().await?;
    tools.ffmpeg.verify_unchanged().await?;
    let output = sheets(dir);
    tokio::fs::create_dir_all(&output).await?;
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
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        // Only keyframes are decoded, by one thread.
        .args(["-threads", "1", "-skip_frame", "nokey", "-i"])
        .arg(&source.path)
        .args([
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            &format!(
                "[0:{index}]{},split[thumbnails][frames];[thumbnails]tile={COLUMNS}x{ROWS}[sheets]",
                filter(video)
            ),
            "-map",
            "[sheets]",
            "-threads",
            "1",
            "-q:v",
            "5",
            "-start_number",
            "0",
        ])
        .arg(output.join("%d.jpg"))
        .args(["-map", "[frames]", "-f", "framecrc"])
        .arg(dir.join(FRAMES))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let deadline = tokio::time::sleep(limit(source));
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            status = child.wait() => {
                anyhow::ensure!(status?.success(), "FFmpeg could not take the thumbnails");
                break;
            }
            _ = &mut deadline => anyhow::bail!("Trickplay generation timed out"),
            _ = check.tick() => {
                if !crate::playback::idle(state).await? {
                    return Ok(None);
                }
            }
        }
    }
    // A file replaced while it was read gives a new version; this one is never used.
    source.validate().await?;
    let frames = tokio::fs::read_to_string(dir.join(FRAMES)).await?;
    tokio::fs::remove_file(dir.join(FRAMES)).await?;
    let count = frames
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count() as u32;
    anyhow::ensure!(count > 0, "The video has no keyframes");
    let first = output.join("0.jpg");
    let (width, height) =
        tokio::task::spawn_blocking(move || image::image_dimensions(first)).await??;
    anyhow::ensure!(
        width == WIDTH * COLUMNS && height % ROWS == 0,
        "Unexpected trickplay sheet size {width}×{height}"
    );
    let mut info = Info {
        width: WIDTH,
        height: height / ROWS,
        tile_width: COLUMNS,
        tile_height: ROWS,
        thumbnail_count: count,
        interval: INTERVAL * 1000,
        bandwidth: 0,
    };
    for index in 0..info.sheets() {
        let size = tokio::fs::metadata(output.join(format!("{index}.jpg")))
            .await?
            .len();
        info.bandwidth = info
            .bandwidth
            .max((size * 8).div_ceil(u64::from(COLUMNS * ROWS * INTERVAL)));
    }
    Ok(Some(info))
}

/// Takes the sheets of library videos, the most recently added or changed first, one file at
/// a time while nothing plays.
pub async fn run(state: AppState) -> anyhow::Result<()> {
    state.tools.wait_ready().await;
    loop {
        let more = next(&state).await?;
        tokio::time::sleep(Duration::from_secs(if more { 5 } else { 60 })).await;
    }
}

/// Takes the sheets of the first library video that has none yet. Returns false when no file
/// is left, or none can be read for now.
async fn next(state: &AppState) -> anyhow::Result<bool> {
    if !crate::playback::idle(state).await? {
        return Ok(true);
    }
    let root = root(state);
    for (media, file, generation) in storage::candidates(&state.db).await? {
        let dir = root.join(name(&file, &generation));
        if settled(&dir).await.is_some() {
            continue;
        }
        let source = match crate::playback::source(state, &media, Some(&file)).await {
            Ok(source) if source.generation == generation => source,
            // Changed since it was listed; the next turn lists it again.
            _ => return Ok(true),
        };
        return match generate(state, &source).await {
            Ok(_) => Ok(true),
            Err(error) => {
                tracing::warn!(%file, "Trickplay generation failed: {error:#}");
                // A failure that wasn't recorded, such as missing space or tools, would stop
                // at the same file again; wait before the next attempt.
                Ok(settled(&dir).await.is_some())
            }
        };
    }
    Ok(false)
}

/// Removes, after a day, the sheets of file versions that are gone and those a crash left
/// half written.
pub async fn prune(state: &AppState) -> anyhow::Result<()> {
    let keep = storage::current(&state.db)
        .await?
        .into_iter()
        .map(|(file, generation)| name(&file, &generation))
        .collect();
    crate::subtitles::sweep(&root(state), &keep, DAY).await
}
