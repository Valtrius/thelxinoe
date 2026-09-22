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
    let allowed = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",
                params![user, video],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
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
    state
        .db
        .call(|db| {
            Ok(db
                .query_row(
                    "SELECT value='true' FROM settings WHERE key='youtube_downloads'",
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .await
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
    let removed=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let protected=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE video_id=?1 AND user_id<>?2 AND (watchlist=1 OR pinned=1)) OR EXISTS(SELECT 1 FROM playback_sessions WHERE youtube_video_id=?1 AND state IN ('ready','playing','paused') AND updated_at>?3)",params![video,user,now()-120],|r|r.get::<_,bool>(0))?;
        if protected {return Ok(false);}
        let row=tx.query_row("SELECT generation,state FROM youtube_downloads WHERE video_id=?1",[&video],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        if let Some((generation,status))=row {
            ensure!(uuid::Uuid::parse_str(&generation).is_ok(),"Invalid download generation");
            // The worker owns a running process and removes its cancelled generation.
            if status!="downloading" && root.exists() {
                let root=root.canonicalize()?;
                let path=root.join(&video).join(&generation);
                if path.exists(){ensure!(path.canonicalize()?==path,"Download path changed");std::fs::remove_dir_all(path)?;}
            }
            tx.execute("DELETE FROM youtube_downloads WHERE video_id=?1",[&video])?;
        }
        tx.execute("INSERT INTO youtube_download_suppressed VALUES(?1) ON CONFLICT DO NOTHING",[video])?;
        tx.commit()?;Ok(true)
    }).await?;
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
    let users=state.db.call(|db|{
        let users=db.prepare("SELECT DISTINCT i.user_id FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id JOIN youtube_state s ON s.user_id=i.user_id AND s.video_id=i.video_id WHERE w.auto_remove_watched=1 AND s.watched=1 AND s.updated_at<?1 AND i.added_at<?1")?.query_map([now()-5],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        db.execute("DELETE FROM youtube_watchlist_items WHERE added_at<?1 AND EXISTS(SELECT 1 FROM youtube_watchlists w JOIN youtube_state s ON s.user_id=w.user_id WHERE w.id=youtube_watchlist_items.watchlist_id AND w.auto_remove_watched=1 AND s.video_id=youtube_watchlist_items.video_id AND s.watched=1 AND s.updated_at<?1)",[now()-5])?;
        Ok(users)
    }).await?;
    for user in users {
        state.emit(Some(user), "youtube.changed", json!({})).await?;
    }
    if !enabled(state).await? {
        return Ok(());
    }
    let candidate=state.db.call(|db|Ok(db.query_row("SELECT i.video_id FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id JOIN youtube_videos v ON v.user_id=i.user_id AND v.video_id=i.video_id WHERE w.auto_download=1 AND v.metadata_at>0 AND v.available=1 AND v.privacy='public' AND v.broadcast IN ('none','replay') AND NOT EXISTS(SELECT 1 FROM youtube_downloads d WHERE d.video_id=i.video_id) AND NOT EXISTS(SELECT 1 FROM youtube_download_suppressed b WHERE b.video_id=i.video_id) ORDER BY i.added_at LIMIT 1",[],|r|r.get::<_,String>(0)).optional()?)).await?;
    if let Some(video) = candidate {
        let Ok(bundle) = tools::ready(state).await else {
            return Ok(());
        };
        state.db.call(move|db|{
            let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO youtube_media(video_id) VALUES(?1) ON CONFLICT DO NOTHING",[&video])?;
            tx.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) SELECT ?1,?2,'queued',?3,?4,?4 WHERE EXISTS(SELECT 1 FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id WHERE i.video_id=?1 AND w.auto_download=1) AND NOT EXISTS(SELECT 1 FROM youtube_download_suppressed WHERE video_id=?1) ON CONFLICT DO NOTHING",params![video,thelxinoe_core::id(),serde_json::to_string(&bundle)?,now()])?;
            tx.commit()?;Ok(())
        }).await?;
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
    state.db.call(move |db| {let tx=db.transaction()?; tx.execute("INSERT INTO settings VALUES ('youtube_downloads',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[input.enabled.to_string()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.downloads.configure',?2,?3)",params![p.user.id,input.enabled.to_string(),now()])?;tx.commit()?;Ok(())}).await?;
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
    let download=state.db.call(move |db| Ok(db.query_row("SELECT state,size,error,downloaded_bytes,total_bytes,eta_seconds,media_kind FROM youtube_downloads WHERE video_id=?1",[video],|r|Ok(json!({"state":r.get::<_,String>(0)?,"size":r.get::<_,Option<i64>>(1)?,"error":r.get::<_,Option<String>>(2)?,"downloaded_bytes":r.get::<_,i64>(3)?,"total_bytes":r.get::<_,Option<i64>>(4)?,"eta_seconds":r.get::<_,Option<i64>>(5)?,"media_kind":r.get::<_,Option<String>>(6)?}))).optional()?)).await?;
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
    let result=state.db.call(move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let interest=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE user_id=?1 AND video_id=?2 AND (watchlist=1 OR pinned=1))",params![p.user.id,video],|r|r.get::<_,bool>(0))?;
        if !interest { return Ok(false); }
        tx.execute("INSERT INTO youtube_media(video_id) VALUES (?1) ON CONFLICT DO NOTHING",[&video])?;
        tx.execute("DELETE FROM youtube_download_suppressed WHERE video_id=?1",[&video])?;
        // A retained interest is required before acquiring shared physical media.
        tx.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES (?1,?2,'queued',?3,?4,?4) ON CONFLICT(video_id) DO UPDATE SET generation=excluded.generation,state='queued',tools=excluded.tools,error=NULL,downloaded_bytes=0,total_bytes=NULL,eta_seconds=NULL,media_kind=NULL,updated_at=excluded.updated_at WHERE youtube_downloads.state IN ('failed','unavailable','extractor_authentication_required')",params![video,thelxinoe_core::id(),serde_json::to_string(&bundle)?,now()])?;
        tx.commit()?;Ok(true)
    }).await?;
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
    state.db.call(move |db| Ok(db.query_row("SELECT generation,path,size,modified,probe FROM youtube_downloads WHERE video_id=?1 AND state='ready'",[&video],|r|Ok(Source {id:video.clone(),media_id:format!("youtube:{video}"),generation:r.get(0)?,edition:"public".into(),path:PathBuf::from(r.get::<_,String>(1)?),root,size:r.get::<_,i64>(2)? as u64,modified:r.get(3)?,probe:serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or_default()})).optional()?)).await?.ok_or_else(||ApiError::conflict("The public download is not ready"))
}

pub(crate) fn record_state(
    tx: &rusqlite::Transaction<'_>,
    user: &str,
    video: &str,
    position: f64,
    duration: f64,
) -> anyhow::Result<()> {
    tx.execute("INSERT INTO youtube_state(user_id,video_id,watched,position,updated_at) SELECT ?1,?2,?3,?4,?5 WHERE EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2) ON CONFLICT(user_id,video_id) DO UPDATE SET watched=MAX(watched,excluded.watched),position=excluded.position,updated_at=excluded.updated_at",params![user,video,duration>0.0&&position>=duration*0.9,if duration>0.0 {position}else{0.0},now()])?;
    Ok(())
}

pub async fn run(state: AppState) -> anyhow::Result<()> {
    // The exclusive server state lock means a prior downloading owner is gone.
    state
        .db
        .call(|db| {
            db.execute(
                "UPDATE youtube_downloads SET state='queued' WHERE state='downloading'",
                [],
            )?;
            Ok(())
        })
        .await?;
    loop {
        if cleanup(&state).await.is_err() {
            tracing::warn!("Online download cleanup could not complete; it will retry");
        }
        if enabled(&state).await? {
            let job=state.db.call(|db| {
                let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let row=tx.query_row("SELECT video_id,generation,tools FROM youtube_downloads d WHERE state='queued' AND EXISTS(SELECT 1 FROM youtube_video_state s WHERE s.video_id=d.video_id AND (s.watchlist=1 OR s.pinned=1)) ORDER BY updated_at LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
                if let Some((video,_,_))=&row {tx.execute("UPDATE youtube_downloads SET state='downloading',updated_at=?2 WHERE video_id=?1",params![video,now()])?;}
                tx.commit()?;Ok(row)
            }).await?;
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
                state.db.call(move |db| {db.execute("UPDATE youtube_downloads SET state=?3,error=CASE WHEN ?3='ready' THEN NULL ELSE ?3 END,updated_at=?4 WHERE video_id=?1 AND generation=?2",params![video,generation,status,now()])?;Ok(())}).await?;
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
    state.db.call(move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let protection="EXISTS(SELECT 1 FROM youtube_video_state s WHERE s.video_id=youtube_downloads.video_id AND (s.watchlist=1 OR s.pinned=1)) OR EXISTS(SELECT 1 FROM playback_sessions p WHERE p.youtube_video_id=youtube_downloads.video_id AND p.state IN ('ready','playing','paused') AND p.updated_at>".to_owned()+&(now()-120).to_string()+")";
        tx.execute(&format!("UPDATE youtube_downloads SET unprotected_at=NULL WHERE {protection}"),[])?;
        tx.execute(&format!("UPDATE youtube_downloads SET unprotected_at=?1 WHERE unprotected_at IS NULL AND NOT ({protection}) AND state!='downloading'"),[now()])?;
        let candidate=tx.query_row(&format!("SELECT video_id,generation,path,size,modified FROM youtube_downloads WHERE unprotected_at<?1 AND state!='downloading' AND NOT ({protection}) ORDER BY unprotected_at LIMIT 1"),[now()-86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,Option<String>>(4)?))).optional()?;
        if let Some((video,generation,file,size,modified))=candidate {
            ensure!(sync::identifier(&video,11)&&uuid::Uuid::parse_str(&generation).is_ok(),"Invalid download identity");
            let path=root.join(&video).join(&generation);
            if path.exists() {
                ensure!(path.canonicalize()?==path,"Download cleanup path changed");
                if let Some(file)=file {
                    let file=PathBuf::from(file);
                    let expected=path.join("media.mp4");
                    ensure!(file.canonicalize()?==expected,"Downloaded file path changed before cleanup");
                    let metadata=std::fs::metadata(&expected)?;
                    ensure!(size==Some(metadata.len() as i64)&&modified==Some(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string()),"Downloaded file generation changed before cleanup");
                }
                // The write transaction fences new interests/playback until the
                // captured generation is removed. Only owned cache files enter here.
                std::fs::remove_dir_all(path)?;
            }
            tx.execute("DELETE FROM playback_sessions WHERE youtube_video_id=?1",[&video])?;
            tx.execute("DELETE FROM youtube_downloads WHERE video_id=?1 AND generation=?2",params![video,generation])?;
        }
        tx.commit()?;Ok(())
    }).await
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
    let (progress,users)=state.db.call(move|db|{
        let progress=db.query_row("SELECT generation,state,size,downloaded_bytes,total_bytes,eta_seconds,media_kind FROM youtube_downloads WHERE video_id=?1",[&id],|r|Ok(json!({"video_id":id,"generation":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"size":r.get::<_,Option<i64>>(2)?,"downloaded_bytes":r.get::<_,i64>(3)?,"total_bytes":r.get::<_,Option<i64>>(4)?,"eta_seconds":r.get::<_,Option<i64>>(5)?,"media_kind":r.get::<_,Option<String>>(6)?}))).optional()?;
        let users=db.prepare("SELECT user_id FROM youtube_videos WHERE video_id=?1")?.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((progress,users))
    }).await?;
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
    let retained = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT COALESCE(SUM(size),0) FROM youtube_downloads WHERE state='ready'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await?;
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
                    let changed=state.db.call(move|db|Ok(db.execute("UPDATE youtube_downloads SET downloaded_bytes=?3,total_bytes=?4,eta_seconds=?5,media_kind=?6 WHERE video_id=?1 AND generation=?2 AND state='downloading'",params![id,version,progress.0,progress.1,progress.2,progress.3])?)).await?;
                    if changed==1 {publish_progress(state, video).await?;}
                }
            }
            _=interval.tick()=> {
                let video=video.to_owned();
                let generation=generation.to_owned();
                let interested=state.db.call(move |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE video_id=?1 AND (watchlist=1 OR pinned=1)) AND EXISTS(SELECT 1 FROM youtube_downloads WHERE video_id=?1 AND generation=?2 AND state='downloading')",params![video,generation],|r|r.get::<_,bool>(0))?)).await?;
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
    state.db.call(move |db| {let changed=db.execute("UPDATE youtube_downloads SET path=?3,size=?4,modified=?5,probe=?6 WHERE video_id=?1 AND generation=?2 AND state='downloading'",params![video,generation,target.to_string_lossy(),metadata.len() as i64,metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),serde_json::to_string(&value)?])?;ensure!(changed==1,"Download was cancelled");Ok(())}).await?;
    Ok("ready")
}
