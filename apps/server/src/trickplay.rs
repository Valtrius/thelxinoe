//! Seek-bar previews of library videos: thumbnail sheets in Jellyfin's trickplay layout,
//! taken from keyframes by a background worker and kept per file version in the cache. Each
//! sheet is taken on its own and kept, so a file interrupted by playback, a scan or a restart
//! continues from its last sheet.

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
use tokio::io::AsyncBufReadExt;

/// Jellyfin's defaults: thumbnails 320 pixels wide, one every 10 seconds, in sheets of 10×10.
const WIDTH: u32 = 320;
const INTERVAL: u32 = 10;
const COLUMNS: u32 = 10;
const ROWS: u32 = 10;
/// Seconds of video in one sheet.
const WINDOW: u32 = COLUMNS * ROWS * INTERVAL;
const MANIFEST: &str = "trickplay.json";
const PROGRESS: &str = "progress.json";
/// The sheet FFmpeg is writing, and its record of each thumbnail: the last sheet is padded,
/// so the record alone gives the count.
const WINDOW_SHEET: &str = "window.jpg";
const WINDOW_FRAMES: &str = "window.frames";
const UNUSABLE: &str = "unusable";
const FAILED: &str = "failed";
/// The sheet whose FFmpeg failure, repeated by the next attempt, marks the version unusable.
const STRIKE: &str = "strike";
/// A file that could not be read is attempted again after this long.
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
/// The sheets' format. Changing it takes every version again, and maintenance removes the
/// sheets of the previous format.
const GENERATOR: u32 = 1;
fn name(file: &str, generation: &str) -> String {
    thelxinoe_auth::digest(&format!("{file}:{generation}:{GENERATOR}"))
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

/// Whether this file version has sheets, or has none for good or for the day.
async fn settled(dir: &FilePath) -> Option<bool> {
    if tokio::fs::try_exists(dir.join(MANIFEST)).await.ok()? {
        return Some(true);
    }
    if tokio::fs::try_exists(dir.join(UNUSABLE)).await.ok()? {
        return Some(false);
    }
    let failed = tokio::fs::metadata(dir.join(FAILED)).await.ok()?;
    let age = failed.modified().ok()?.elapsed().unwrap_or_default();
    (age < DAY).then_some(false)
}

/// Why a file version will never have sheets, such as a video FFmpeg can't decode. The
/// version is not read again.
#[derive(Debug)]
struct Unusable(String);
impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Unusable {}

/// The thumbnails of each sheet taken so far, and the seconds spent on them.
#[derive(Default, Serialize, Deserialize)]
struct Progress {
    sheets: Vec<u32>,
    seconds: f64,
}
/// The sheets a crash or restart left, up to the first one that is missing.
async fn resume(dir: &FilePath) -> Progress {
    let mut progress: Progress = match tokio::fs::read(dir.join(PROGRESS)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Progress::default(),
    };
    for index in 0..progress.sheets.len() {
        let sheet = sheets(dir).join(format!("{index}.jpg"));
        if !tokio::fs::try_exists(sheet).await.unwrap_or(false) {
            progress.sheets.truncate(index);
            break;
        }
    }
    progress
}
async fn remove(dir: &FilePath, name: &str) -> std::io::Result<()> {
    match tokio::fs::remove_file(dir.join(name)).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
/// Writes `bytes` to `name` in `dir` whole, so a crash never leaves a part of it.
async fn replace(dir: &FilePath, name: &str, bytes: Vec<u8>) -> std::io::Result<()> {
    let temporary = dir.join(format!("{name}.tmp"));
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, dir.join(name)).await
}

/// Takes the next sheet of a file version, and completes the version after its last one.
/// Playback stops it and returns false: the sheet in progress is dropped, the others kept.
async fn step(state: &AppState, source: &Source) -> anyhow::Result<bool> {
    let tools = state.tools.runtime.media()?;
    tools.ffmpeg.verify_unchanged().await?;
    let root = root(state);
    let dir = root.join(name(&source.id, &source.generation));
    tokio::fs::create_dir_all(sheets(&dir)).await?;
    anyhow::ensure!(
        fs2::available_space(&root)? >= SPACE,
        "Insufficient trickplay cache space"
    );
    let result = advance(state, source, &tools, &dir).await;
    if let Err(error) = &result {
        // Unusable versions are not read again; others are retried after a day, from the
        // sheets they already have. Sheets are never discarded.
        let marker = if error.downcast_ref::<Unusable>().is_some() {
            UNUSABLE
        } else {
            FAILED
        };
        if let Err(marker) = tokio::fs::write(dir.join(marker), format!("{error:#}")).await {
            tracing::warn!("Failed trickplay could not be recorded: {marker:#}");
        }
    }
    result
}

async fn advance(
    state: &AppState,
    source: &Source,
    tools: &thelxinoe_tools::MediaTools,
    dir: &FilePath,
) -> anyhow::Result<bool> {
    let video = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or_else(|| Unusable("The file has no video".into()))?;
    if crate::jellyfin::video_range(video).1 == "DOVI" {
        // Without a base layer, its colors need Dolby Vision processing FFmpeg can't do here.
        return Err(Unusable("Dolby Vision video without a base layer".into()).into());
    }
    let duration = source.duration();
    if !duration.is_finite() || duration <= 0.0 {
        return Err(Unusable("The file has no duration".into()).into());
    }
    let total = (duration / f64::from(INTERVAL)).ceil().max(1.0) as u32;
    let mut progress = resume(dir).await;
    let index = progress.sheets.len() as u32;
    let wanted = (COLUMNS * ROWS).min(total.saturating_sub(index * COLUMNS * ROWS));
    if wanted > 0 {
        let started = std::time::Instant::now();
        let Some(taken) = render(state, source, tools, dir, video, index, wanted).await? else {
            return Ok(false);
        };
        if taken == 0 && index == 0 {
            return Err(Unusable("FFmpeg found no keyframes".into()).into());
        }
        if taken > 0 {
            tokio::fs::rename(
                dir.join(WINDOW_SHEET),
                sheets(dir).join(format!("{index}.jpg")),
            )
            .await?;
            progress.sheets.push(taken);
            progress.seconds += started.elapsed().as_secs_f64();
            replace(dir, PROGRESS, serde_json::to_vec(&progress)?).await?;
            remove(dir, STRIKE).await?;
        }
        // The version is complete after its last sheet, or once a window has no video: a
        // video that ends before its file repeats its last frame to the end of that sheet.
        if taken == wanted && (index + 1) * COLUMNS * ROWS < total {
            return Ok(true);
        }
    }
    let (width, height) = {
        let first = sheets(dir).join("0.jpg");
        tokio::task::spawn_blocking(move || image::image_dimensions(first)).await??
    };
    if width != WIDTH * COLUMNS || height % ROWS != 0 {
        return Err(Unusable(format!("Unexpected trickplay sheet size {width}×{height}")).into());
    }
    let mut info = Info {
        width: WIDTH,
        height: height / ROWS,
        tile_width: COLUMNS,
        tile_height: ROWS,
        thumbnail_count: progress.sheets.iter().sum(),
        interval: INTERVAL * 1000,
        bandwidth: 0,
    };
    for index in 0..progress.sheets.len() {
        let size = tokio::fs::metadata(sheets(dir).join(format!("{index}.jpg")))
            .await?
            .len();
        info.bandwidth = info.bandwidth.max((size * 8).div_ceil(u64::from(WINDOW)));
    }
    replace(dir, MANIFEST, serde_json::to_vec(&info)?).await?;
    for leftover in [
        PROGRESS,
        FAILED,
        STRIKE,
        WINDOW_SHEET,
        WINDOW_FRAMES,
        &format!("{PROGRESS}.tmp"),
        &format!("{MANIFEST}.tmp"),
    ] {
        remove(dir, leftover).await?;
    }
    tracing::info!(
        file = %source.id,
        codec = video["codec_name"].as_str().unwrap_or_default(),
        height = video["height"].as_i64().unwrap_or_default(),
        video_seconds = duration.round(),
        seconds = (progress.seconds * 10.0).round() / 10.0,
        thumbnails = info.thumbnail_count,
        "Took trickplay sheets"
    );
    Ok(true)
}

/// The filters from the video stream to `thumbnails` thumbnails `WIDTH` wide at its display
/// proportions, one per interval from the start of the window:
/// - `fps` starts at 0 s, so a video that starts after its audio keeps its thumbnails in
///   place, and keeps the last keyframe, so short videos have one;
/// - `tpad` repeats the last thumbnail over a gap at the end of the window, and `trim` stops
///   at the sheet's count, so every full sheet holds exactly one thumbnail per cell;
/// - HDR is tone mapped after scaling, so only thumbnails pay for it.
fn filter(video: &Value, thumbnails: u32) -> String {
    let mut filter = format!(
        "fps=fps=1/{INTERVAL}:start_time=0:eof_action=pass,tpad=stop_mode=clone:stop_duration={WINDOW},trim=end_frame={thumbnails},scale=w={WIDTH}:h='max(2,trunc(ow/dar/2)*2)',setsar=1"
    );
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
/// Three times the window's playing time, or the time to read its share of the file at
/// 2 MiB/s, plus ten minutes: FFmpeg at low priority is slow while other work runs.
fn limit(source: &Source) -> Duration {
    let share = (source.size as f64 * f64::from(WINDOW) / source.duration()) as u64;
    Duration::from_secs(600 + u64::from(3 * WINDOW).max(share / (2 * 1024 * 1024)))
}

/// Writes sheet `index` of `source` to the window files in `dir`, from keyframes only, and
/// returns how many thumbnails it holds. Returns `None` when it stopped for playback; library
/// work waits for the next window, so scans can't stop every window of a long file.
async fn render(
    state: &AppState,
    source: &Source,
    tools: &thelxinoe_tools::MediaTools,
    dir: &FilePath,
    video: &Value,
    index: u32,
    thumbnails: u32,
) -> anyhow::Result<Option<u32>> {
    let stream = video["index"]
        .as_i64()
        .ok_or_else(|| Unusable("Video stream has no index".into()))?;
    source.validate().await?;
    let _read = crate::subtitles::READS.acquire().await?;
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
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        // Only keyframes are decoded, by one thread, from the window's start.
        .args(["-threads", "1", "-skip_frame", "nokey"])
        .args([
            "-ss",
            &(index * WINDOW).to_string(),
            "-t",
            &WINDOW.to_string(),
        ])
        .arg("-i")
        .arg(&source.path)
        .args([
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            &format!(
                "[0:{stream}]{},split[thumbnails][frames];[thumbnails]tile={COLUMNS}x{ROWS}[sheet]",
                filter(video, thumbnails)
            ),
            "-map",
            "[sheet]",
            "-threads",
            "1",
            "-frames:v",
            "1",
            "-update",
            "1",
            "-q:v",
            "5",
        ])
        .arg(dir.join(WINDOW_SHEET))
        .args(["-map", "[frames]", "-f", "framecrc"])
        .arg(dir.join(WINDOW_FRAMES))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    // The last lines FFmpeg reports explain a failure.
    let errors = child.stderr.take().context("Missing FFmpeg errors")?;
    let errors = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(errors).lines();
        let mut last = std::collections::VecDeque::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if last.len() == 5 {
                last.pop_front();
            }
            last.push_back(line.chars().take(300).collect::<String>());
        }
        Vec::from(last).join("\n")
    });
    let deadline = tokio::time::sleep(limit(source));
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(Duration::from_secs(2));
    // Whether FFmpeg failed on the file, rather than being stopped or starved.
    let failure = loop {
        tokio::select! {
            status = child.wait() => break match status?.code() {
                Some(0) => None,
                // Signals, as when the system stops FFmpeg.
                Some(255) | None => Some((false, "FFmpeg was stopped")),
                Some(_) => Some((true, "FFmpeg could not take the thumbnails")),
            },
            _ = &mut deadline => {
                child.kill().await?;
                break Some((false, "FFmpeg took too long to take the thumbnails"));
            }
            _ = check.tick() => {
                if crate::playback::playing(state).await? {
                    return Ok(None);
                }
            }
        }
    };
    if let Some((decoding, failure)) = failure {
        let report = format!("{failure}\n{}", errors.await.unwrap_or_default());
        let report = report.trim_end();
        anyhow::ensure!(decoding, "{report}");
        // A file that moved or a full cache fails FFmpeg too.
        source.validate().await?;
        anyhow::ensure!(
            fs2::available_space(dir)? >= SPACE,
            "Insufficient trickplay cache space"
        );
        // The failure is the file's own when the next attempt, a day later, repeats it.
        let strike = dir.join(STRIKE);
        if tokio::fs::read_to_string(&strike).await.ok() == Some(index.to_string()) {
            return Err(Unusable(report.to_owned()).into());
        }
        tokio::fs::write(&strike, index.to_string()).await?;
        anyhow::bail!("{report}");
    }
    // A file replaced while it was read gives a new version; this one is never used.
    source.validate().await?;
    let frames = tokio::fs::read_to_string(dir.join(WINDOW_FRAMES)).await?;
    Ok(Some(
        frames
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .count() as u32,
    ))
}

/// Takes the sheets of library videos, the most recently added or changed first, one sheet at
/// a time while nothing plays.
pub async fn run(state: AppState) -> anyhow::Result<()> {
    state.tools.wait_ready().await;
    loop {
        let more = next(&state).await?;
        tokio::time::sleep(Duration::from_secs(if more { 5 } else { 60 })).await;
    }
}

/// Takes the next sheet of the first library video that has none yet. Returns false when no
/// file is left, or none can be read for now.
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
        return match step(state, &source).await {
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

/// Removes, after a day, the sheets of file versions that are gone or of an earlier format.
/// A current version's unfinished sheets stay, to be continued.
pub async fn prune(state: &AppState) -> anyhow::Result<()> {
    let keep = storage::current(&state.db)
        .await?
        .into_iter()
        .map(|(file, generation)| name(&file, &generation))
        .collect();
    crate::subtitles::sweep(&root(state), &keep, DAY).await
}
