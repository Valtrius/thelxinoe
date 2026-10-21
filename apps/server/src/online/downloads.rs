#[path = "../storage/online/downloads.rs"]
mod storage;

use super::{extract, network, process, sync, tools};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use anyhow::ensure;
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant, UNIX_EPOCH},
};
use thelxinoe_core::{Capability, Principal, now};
use thelxinoe_playback::Source;
#[cfg(test)]
mod tests;

pub(crate) async fn authorize(state: &AppState, p: &Principal, video: &str) -> Result<()> {
    super::availability::require_enabled(state, "youtube").await?;
    if !sync::identifier(video, 11) {
        return Err(ApiError::not_found());
    }
    let user = p.user.id.clone();
    let video = video.to_owned();
    let allowed = storage::authorize(user, video, &state.db).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    Ok(())
}
pub async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(json!({"enabled":enabled(&state).await?})))
}
pub(super) async fn enabled(state: &AppState) -> anyhow::Result<bool> {
    Ok(
        super::availability::enabled(state, "youtube").await?
            && storage::enabled(&state.db).await?,
    )
}

pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p, &video).await?;
    let root = state.config.cache.join("youtube");
    let user = p.user.id.clone();
    let removed = storage::remove(&state.db, video, root, user).await?;
    if !removed {
        return Err(ApiError::conflict(
            "This shared download is retained by another user or is playing. Remove it after those uses finish.",
        ));
    }
    state
        .emit(None, "online.download.changed", json!({}))
        .await?;
    Ok(Json(json!({"deleted":true})))
}

pub(super) async fn run_watchlists(state: AppState) -> anyhow::Result<()> {
    loop {
        if maintain_watchlists(&state).await.is_err() {
            tracing::warn!("Watchlist maintenance will retry");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn maintain_watchlists(state: &AppState) -> anyhow::Result<()> {
    if !super::availability::enabled(state, "youtube").await? {
        return Ok(());
    }
    let users = storage::maintain_watchlists_write_youtube_watchlist_items(&state.db).await?;
    for user in users {
        state.emit(Some(user), "youtube.changed", json!({})).await?;
    }
    if !enabled(state).await? {
        return Ok(());
    }
    let candidate = storage::maintain_watchlists_read_youtube_watchlist_items(&state.db).await?;
    if let Some(video) = candidate {
        let Ok(bundle) = tools::ready(state).await else {
            return Ok(());
        };
        storage::maintain_watchlists_write_youtube_media(bundle, video, &state.db).await?;
    }
    Ok(())
}
#[derive(serde::Deserialize)]
pub struct Configuration {
    enabled: bool,
}
pub async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Configuration>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    storage::configure(&state.db, input.enabled, p).await?;
    Ok(Json(json!({"enabled":input.enabled})))
}
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p, &video).await?;
    let enabled = enabled(&state).await?;
    let download = storage::status(&state.db, video).await?;
    Ok(Json(json!({"enabled":enabled,"download":download})))
}
pub async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    request_for(&state, &p, video).await
}
pub(crate) async fn request_for(
    state: &AppState,
    p: &Principal,
    video: String,
) -> Result<Json<Value>> {
    authorize(state, p, &video).await?;
    if !enabled(state).await? {
        return Err(ApiError::conflict(
            "YouTube downloads are disabled by the administrator",
        ));
    }
    let bundle = tools::ready(state).await?;
    let p = p.clone();
    storage::request_for(bundle, p, &state.db, video).await?;
    Ok(Json(json!({"queued":true})))
}

pub(crate) async fn source(state: &AppState, video: &str) -> Result<Source> {
    let root = state.config.cache.join("youtube");
    let source = storage::source(video.to_owned(), root, &state.db)
        .await?
        .ok_or_else(|| ApiError::conflict("The public download is not ready"))?;
    // A copy that is missing or changed on disk can't play; the video streams instead and
    // can be downloaded again.
    if let Err(error) = source.validate().await {
        tracing::warn!(video, %error, "Downloaded copy is unusable");
        storage::invalidate(video.to_owned(), source.generation.clone(), &state.db).await?;
        state
            .emit(None, "online.download.changed", json!({"video_id":video}))
            .await?;
        return Err(ApiError::conflict("The public download is not ready"));
    }
    Ok(source)
}

pub(crate) use storage::record_state;

pub async fn run(state: AppState) -> anyhow::Result<()> {
    // The exclusive server state lock means a prior downloading owner is gone.
    storage::recover_downloads(&state.db).await?;
    state.tools.wait_ready().await;
    let mut parked = Parked::new();
    loop {
        if enabled(&state).await? {
            next(&state, &mut parked).await?;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Free disk space a download needs to start.
const START_SPACE: u64 = 3 * 1024 * 1024 * 1024;
/// A download is bounded by its progress, not its length, so slow links still finish:
/// yt-dlp reports progress every second while bytes arrive, and reports its retries.
const STALLED: Duration = Duration::from_secs(300);
/// A safety bound only: a 2 GB download at about 400 kbit/s.
const LONGEST: Duration = Duration::from_secs(12 * 3600);
/// A download waiting for space is tried again after this long even when its capacity looks
/// unchanged, for changes `capacity` can't see.
const RECHECK: Duration = Duration::from_secs(300);
/// Downloads waiting for space, by video: the capacity they last waited at, and when to try
/// them again regardless.
type Parked = HashMap<String, (Capacity, Instant)>;
type Capacity = ([i64; 4], bool);

async fn capacity(state: &AppState) -> anyhow::Result<Capacity> {
    Ok((
        crate::auto_delete::capacity(&state.db).await?,
        fs2::available_space(&state.config.cache).is_ok_and(|free| free > START_SPACE),
    ))
}

/// Attempts the next download. One waiting for space is only tried again once the capacity
/// it waits on changes, and a retry that still waits announces nothing.
async fn next(state: &AppState, parked: &mut Parked) -> anyhow::Result<()> {
    if !parked.is_empty() {
        let current = capacity(state).await?;
        parked.retain(|_, (seen, until)| *seen == current && Instant::now() < *until);
    }
    let job = storage::claim_download(&state.db, parked.keys().cloned().collect()).await?;
    let Some((video, generation, bundle, waiting)) = job else {
        return Ok(());
    };
    let admitted = crate::auto_delete::admit(state, &video, &generation).await;
    let status = match admitted {
        Ok(true) => download(state, &video, &generation, &bundle)
            .await
            .unwrap_or("failed"),
        Ok(false) => "waiting_for_space",
        Err(_) => "failed",
    };
    if status != "ready" {
        // No playback can reference a generation until it is ready.
        let path = state
            .config
            .cache
            .join("youtube")
            .join(&video)
            .join(&generation);
        if let Ok(root) = state.config.cache.join("youtube").canonicalize() {
            let expected = root.join(&video).join(&generation);
            if path.canonicalize().ok().as_ref() == Some(&expected) {
                let _ = tokio::fs::remove_dir_all(&expected).await;
            }
        }
    }
    storage::finish_download(status, video.clone(), generation, &state.db).await?;
    if status == "waiting_for_space" {
        parked.insert(
            video.clone(),
            (capacity(state).await?, Instant::now() + RECHECK),
        );
        // A retry that still can't start changed nothing clients see.
        if waiting && matches!(admitted, Ok(false)) {
            return Ok(());
        }
    }
    publish_progress(state, &video).await?;
    state
        .emit(None, "online.download.changed", json!({}))
        .await?;
    Ok(())
}

fn parse_progress(line: &str) -> Option<(i64, Option<i64>, Option<i64>, &'static str)> {
    let parts = line
        .strip_prefix("THELXINOE_PROGRESS:")?
        .trim()
        .split('\t')
        .collect::<Vec<_>>();
    let number = |index: usize| {
        parts
            .get(index)?
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 64.0 * 1024.0 * 1024.0 * 1024.0)
            .map(|n| n as i64)
    };
    Some((
        number(0)?,
        number(1).or_else(|| number(2)),
        number(3),
        if parts.get(4) == Some(&"none") {
            "audio"
        } else {
            "video"
        },
    ))
}

async fn publish_progress(state: &AppState, video: &str) -> anyhow::Result<()> {
    let id = video.to_owned();
    let (progress, users) = storage::publish_progress(id, &state.db).await?;
    if let Some(progress) = progress {
        for user in users {
            state
                .emit(Some(user), "online.download.progress", progress.clone())
                .await?;
        }
    }
    Ok(())
}

async fn download(
    state: &AppState,
    video: &str,
    generation: &str,
    bundle: &str,
) -> anyhow::Result<&'static str> {
    state
        .emit(None, "online.download.changed", json!({}))
        .await?;
    let budget = crate::auto_delete::budget(&state.db, video.to_owned()).await?;
    let mut bundle: tools::OnlineSnapshot = serde_json::from_str(bundle)?;
    state.tools.runtime.restore_online(&mut bundle)?;
    tools::verify(&bundle.ffmpeg).await?;
    tools::verify(&bundle.ffprobe).await?;
    tools::verify(&bundle.yt_dlp).await?;
    tools::verify(&bundle.deno).await?;
    let directory = state
        .config
        .cache
        .join("youtube")
        .join(video)
        .join(generation);
    tokio::fs::create_dir_all(&directory).await?;
    let directory = directory.canonicalize()?;
    if fs2::available_space(&directory)? <= START_SPACE {
        return Ok("waiting_for_space");
    }
    let target = directory.join("media.mp4");
    let families = network::families(state)
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    let mut attempts = families.into_iter().peekable();
    let output = loop {
        let family = attempts.next().expect("at least one family is attempted");
        let mut args = extract::arguments(&bundle, video, family);
        args.truncate(args.len() - 4); // replace metadata output flags and canonical URL
        args.extend(
            [
                "--progress",
                "--newline",
                "--progress-delta",
                "1",
                "--progress-template",
                "download:THELXINOE_PROGRESS:%(progress.downloaded_bytes)s\t%(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t%(progress.eta)s\t%(info.vcodec)s\t%(info.acodec)s",
                "--no-warnings",
                "--no-simulate",
                "--match-filter",
                "!is_live & duration <= 21600",
                "-f",
                "bv*[height<=1080][ext=mp4]+ba[ext=m4a]/b[ext=mp4]",
                "--merge-output-format",
                "mp4",
                "--ffmpeg-location",
            ]
            .map(Into::into),
        );
        args.push(bundle.ffmpeg.path.clone().into_os_string());
        args.extend(["--max-filesize".into(), budget.to_string().into()]);
        args.push("-o".into());
        args.push(target.clone().into_os_string());
        args.extend([
            "--".into(),
            format!("https://www.youtube.com/watch?v={video}").into(),
        ]);
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
        let execution = process::run_progress(
            state.tools.temporary()?,
            &bundle.yt_dlp.path,
            &args,
            STALLED,
            LONGEST,
            1024 * 1024,
            Some(progress_tx),
        );
        tokio::pin!(execution);
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        let output = loop {
            tokio::select! {
                result=&mut execution => break result?,
                Some(line)=progress_rx.recv()=> {
                    if let Some(progress) = parse_progress(&line) {
                        let id=video.to_owned(); let version=generation.to_owned();
                        let changed=storage::save_progress(&state.db, id, version, progress).await?;
                        if changed==1 {publish_progress(state, video).await?;}
                    }
                }
                _=interval.tick()=> {
                    let video=video.to_owned();
                    let generation=generation.to_owned();
                    let interested=storage::has_interest(&state.db, video, generation).await?;
                    ensure!(interested&&enabled(state).await?,"Download cancelled after its interest or permission changed");
                    if fs2::available_space(&directory)?<=1024*1024*1024 {return Ok("waiting_for_space");}
                    let mut entries=tokio::fs::read_dir(&directory).await?;let mut bytes=0u64;
                    while let Some(entry)=entries.next_entry().await? {let meta=entry.metadata().await?;ensure!(meta.is_file(),"Unexpected download output");bytes=bytes.saturating_add(meta.len());}
                    ensure!(bytes<=6*1024*1024*1024,"Download temporary files exceed limit");
                }
            }
        };
        // Only YouTube's network block is retried, over the next family.
        if output.success
            || attempts.peek().is_none()
            || extract::failure(&output.stderr) != "network_blocked"
        {
            if output.success {
                network::worked(state, family);
            }
            break output;
        }
    };
    if !output.success {
        return Ok(match extract::failure(&output.stderr) {
            "extraction_failed" => "failed",
            state => state,
        });
    }
    let metadata = tokio::fs::metadata(&target).await?;
    ensure!(
        metadata.len() <= 2 * 1024 * 1024 * 1024,
        "Download exceeds size limit"
    );
    let probe = process::run(
        state.tools.temporary()?,
        &bundle.ffprobe.path,
        &[
            "-v".into(),
            "error".into(),
            "-show_format".into(),
            "-show_streams".into(),
            "-of".into(),
            "json".into(),
            target.clone().into_os_string(),
        ],
        Duration::from_secs(30),
        1024 * 1024,
    )
    .await?;
    ensure!(probe.success, "Downloaded media probe failed");
    let value: Value = serde_json::from_slice(&probe.stdout)?;
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    ensure!(
        duration.is_finite() && duration > 0.0,
        "Download has no playable duration"
    );
    let video = video.to_owned();
    let generation = generation.to_owned();
    Ok(
        if storage::publish_download(target, metadata, value, video, generation, &state.db).await? {
            "ready"
        } else {
            "waiting_for_space"
        },
    )
}
