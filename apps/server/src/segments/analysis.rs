#[path = "../storage/segments/analysis.rs"]
mod storage;

use super::*;
use std::{process::Stdio, time::Duration};
use thelxinoe_playback::Source;

async fn idle(state: &AppState) -> anyhow::Result<bool> {
    storage::idle(&state.db).await
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    storage::run_write_segment_analysis(&state.db).await?;
    loop {
        if idle(&state).await? {
            let cfg = config(&state).await.map_err(|e| anyhow::anyhow!(e.2))?;
            if cfg.local || cfg.external {
                let work = storage::claim_analysis(&state.db).await?;
                if let Some((media, file, generation)) = work {
                    let outcome = analyze(&state, &media, &file, &generation, cfg).await;
                    let interrupted = !idle(&state).await?;
                    let message = outcome.err().map(|e| e.to_string());
                    let event_media = media.clone();
                    storage::finish_analysis(
                        interrupted,
                        message,
                        media,
                        file,
                        generation,
                        &state.db,
                    )
                    .await?;
                    state
                        .emit(None, "segments.changed", json!({"media_id":event_media}))
                        .await?;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
async fn extract(
    state: &AppState,
    src: &Source,
    offset: f64,
    length: f64,
) -> anyhow::Result<Vec<u32>> {
    src.validate().await?;
    #[cfg(unix)]
    let mut command = {
        let mut c = tokio::process::Command::new("nice");
        c.args(["-n", "15", "ffmpeg"]);
        c
    };
    #[cfg(not(unix))]
    let mut command = tokio::process::Command::new("ffmpeg");
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
            "-ss",
            &offset.to_string(),
            "-i",
        ])
        .arg(&src.path)
        .args([
            "-t",
            &length.to_string(),
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "11025",
            "-threads",
            "1",
            "-f",
            "chromaprint",
            "-algorithm",
            "1",
            "-fp_format",
            "raw",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let process = command.spawn()?.wait_with_output();
    tokio::pin!(process);
    let deadline = tokio::time::sleep(Duration::from_secs(180));
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            output=&mut process=>{let output=output?;anyhow::ensure!(output.status.success(),"Audio fingerprint extraction failed");anyhow::ensure!(output.stdout.len()<=40000,"Fingerprint exceeded its bound");src.validate().await?;return Ok(fingerprint::decode(&output.stdout));},
            _=&mut deadline=>anyhow::bail!("Audio analysis timed out"),
            _=check.tick()=>{anyhow::ensure!(idle(state).await?,"Analysis yielded to playback or library work");}
        }
    }
}
async fn analyze(
    state: &AppState,
    media: &str,
    file: &str,
    generation: &str,
    cfg: Config,
) -> anyhow::Result<()> {
    let _lease = state.media_operations.read().await;
    let src = playback::source(state, media, Some(file))
        .await
        .map_err(|e| anyhow::anyhow!(e.2))?;
    anyhow::ensure!(src.generation == generation, "Media generation changed");
    let mid = media.to_owned();
    let fid = file.to_owned();
    let season = storage::analyze_read_media(mid, fid, &state.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Analysis requires a single episode file"))?;
    anyhow::ensure!(
        src.duration().is_finite() && src.duration() > 30.0,
        "Episode is too short for analysis"
    );
    src.validate().await?;
    if cfg.local {
        let length = (src.duration() * 0.25).min(600.0);
        for (kind, offset) in [("Intro", 0.0), ("Credits", src.duration() - length)] {
            let hashes = extract(state, &src, offset, length).await?;
            let saved = src.clone();
            let serialized = serde_json::to_string(&hashes)?;
            let peers = storage::analyze_write_segment_fingerprints(
                &season, kind, offset, saved, serialized, &state.db,
            )
            .await?;
            for (other_media, other_file, other_generation, other_offset, other_hashes) in peers {
                let values = hashes.clone();
                let matched = tokio::task::spawn_blocking(move || {
                    fingerprint::recurring(&values, &other_hashes)
                })
                .await?;
                if let Some((a, b, c, d)) = matched {
                    let other = playback::source(state, &other_media, Some(&other_file))
                        .await
                        .map_err(|e| anyhow::anyhow!(e.2))?;
                    if other.generation != other_generation {
                        continue;
                    }
                    for (source, start, end) in [
                        (&src, a + offset, b + offset),
                        (&other, c + other_offset, d + other_offset),
                    ] {
                        let segment = Segment {
                            id: String::new(),
                            kind: kind.into(),
                            start,
                            end,
                            source: "local".into(),
                            confidence: 0.85,
                        };
                        if valid(&segment, source.duration()) {
                            store(state, source, vec![segment], "local", Some(kind)).await?;
                        }
                    }
                    break;
                }
            }
        }
    }
    if cfg.external {
        external(state, &src).await?;
    }
    Ok(())
}
async fn store(
    state: &AppState,
    src: &Source,
    segments: Vec<Segment>,
    origin: &'static str,
    kind: Option<&'static str>,
) -> anyhow::Result<()> {
    src.validate().await?;
    let src = src.clone();
    storage::store(src, &state.db, segments, origin, kind).await
}
async fn external(state: &AppState, src: &Source) -> anyhow::Result<()> {
    let media = src.media_id.clone();
    let coordinates = storage::external(media, &state.db).await?;
    let Some((series, tmdb_id, season, episode)) = coordinates else {
        return Ok(());
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()?;
    let mut response = client
        .get("https://api.theintrodb.org/v3/media")
        .query(&[
            ("tvdb_id", series.clone()),
            ("season", season.to_string()),
            ("episode", episode.to_string()),
            (
                "duration_ms",
                ((src.duration() * 1000.0) as i64).to_string(),
            ),
        ])
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return store(state, src, vec![], "theintrodb", None).await;
    }
    anyhow::ensure!(
        response.status().is_success(),
        "External timestamp provider returned HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 256000,
            "External timestamp response exceeded limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    let segments = parse_external(&value, tmdb_id, season, episode, src.duration())?;
    store(state, src, segments, "theintrodb", None).await
}
pub(super) fn parse_external(
    value: &Value,
    tmdb_id: Option<i64>,
    season: i64,
    episode: i64,
    duration: f64,
) -> anyhow::Result<Vec<Segment>> {
    anyhow::ensure!(
        tmdb_id.is_none_or(|id| value["tmdb_id"].as_i64() == Some(id))
            && value["season"].as_i64() == Some(season)
            && value["episode"].as_i64() == Some(episode)
            && value["type"] == "tv",
        "External timestamp identity does not match this episode"
    );
    let mut segments = Vec::new();
    for kind in KINDS {
        for item in value[kind.to_lowercase()]
            .as_array()
            .into_iter()
            .flatten()
            .take(10)
        {
            if item.get("start_ms").is_none() || item.get("end_ms").is_none() {
                continue;
            }
            let start = if item["start_ms"].is_null() {
                0.0
            } else {
                item["start_ms"].as_f64().unwrap_or(f64::NAN) / 1000.0
            };
            let end = if item["end_ms"].is_null() {
                duration
            } else {
                item["end_ms"].as_f64().unwrap_or(f64::NAN) / 1000.0
            };
            let s = Segment {
                id: String::new(),
                kind: kind.into(),
                start,
                end,
                source: "theintrodb".into(),
                confidence: 0.8,
            };
            if valid(&s, duration) {
                segments.push(s);
            }
        }
    }
    Ok(segments)
}
