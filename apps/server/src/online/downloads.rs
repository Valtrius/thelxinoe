#[path = "../storage/online/downloads.rs"]
mod storage;

use super::{extract, process, sync, tools};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use anyhow::{Context, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};
use thelxinoe_core::{Capability, Principal, now};
use thelxinoe_playback::Source;
#[cfg(test)]
mod tests;

pub(crate) async fn authorize(state: &AppState, p: &Principal, video: &str) -> Result<()> {
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
    storage::enabled(&state.db).await
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
    let result = storage::request_for(bundle, p, &state.db, video).await?;
    if !result {
        return Err(ApiError::conflict(
            "Add this video to your watchlist or pin it before downloading",
        ));
    }
    Ok(Json(json!({"queued":true})))
}

pub(crate) async fn source(state: &AppState, video: &str) -> Result<Source> {
    let video = video.to_owned();
    let root = state.config.cache.join("youtube");
    storage::source(video, root, &state.db)
        .await?
        .ok_or_else(|| ApiError::conflict("The public download is not ready"))
}

pub(crate) use storage::record_state;

pub async fn run(state: AppState) -> anyhow::Result<()> {
    // The exclusive server state lock means a prior downloading owner is gone.
    storage::recover_downloads(&state.db).await?;
    loop {
        if cleanup(&state).await.is_err() {
            tracing::warn!("Online download cleanup could not complete; it will retry");
        }
        if enabled(&state).await? {
            let job = storage::claim_download(&state.db).await?;
            if let Some((video, generation, bundle)) = job {
                state
                    .emit(None, "online.download.changed", json!({}))
                    .await?;
                let outcome = download(&state, &video, &generation, &bundle).await;
                let status = outcome.unwrap_or("failed");
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
                let completed_video = video.clone();
                storage::finish_download(status, video, generation, &state.db).await?;
                publish_progress(&state, &completed_video).await?;
                state
                    .emit(None, "online.download.changed", json!({}))
                    .await?;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn cleanup(state: &AppState) -> anyhow::Result<()> {
    let root = state.config.cache.join("youtube");
    if !root.exists() {
        return Ok(());
    }
    let root = root.canonicalize()?;
    storage::cleanup(root, &state.db).await
}

fn system_tool(name: &str) -> anyhow::Result<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join(&file))
        .find(|p| p.is_file())
        .context("Required FFmpeg tools are unavailable")?
        .canonicalize()
        .map_err(Into::into)
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
    let bundle: tools::Bundle = serde_json::from_str(bundle)?;
    tools::verify(&bundle.yt_dlp).await?;
    tools::verify(&bundle.deno).await?;
    let retained = storage::cache_size(&state.db).await?;
    ensure!(
        retained < 48 * 1024 * 1024 * 1024,
        "Public download cache reached its 50 GiB limit"
    );
    let directory = state
        .config
        .cache
        .join("youtube")
        .join(video)
        .join(generation);
    tokio::fs::create_dir_all(&directory).await?;
    let directory = directory.canonicalize()?;
    ensure!(
        fs2::available_space(&directory)? > 3 * 1024 * 1024 * 1024,
        "Insufficient download space"
    );
    let target = directory.join("media.mp4");
    let mut args = extract::arguments(&bundle, video);
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
            "--max-filesize",
            "2G",
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
    args.push(system_tool("ffmpeg")?.into_os_string());
    args.push("-o".into());
    args.push(target.clone().into_os_string());
    args.extend([
        "--".into(),
        format!("https://www.youtube.com/watch?v={video}").into(),
    ]);
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
    let execution = process::run_progress(
        &bundle.yt_dlp.path,
        &args,
        Duration::from_secs(1800),
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
                ensure!(fs2::available_space(&directory)?>1024*1024*1024,"Download stopped to preserve free space");
                let mut entries=tokio::fs::read_dir(&directory).await?;let mut bytes=0u64;
                while let Some(entry)=entries.next_entry().await? {let meta=entry.metadata().await?;ensure!(meta.is_file(),"Unexpected download output");bytes=bytes.saturating_add(meta.len());}
                ensure!(bytes<=6*1024*1024*1024,"Download temporary files exceed limit");
            }
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
        &system_tool("ffprobe")?,
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
    storage::publish_download(target, metadata, value, video, generation, &state.db).await?;
    Ok("ready")
}
