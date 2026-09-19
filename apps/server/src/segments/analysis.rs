use super::*;
use std::{process::Stdio, time::Duration};
use thelxinoe_playback::Source;

async fn idle(state: &AppState) -> anyhow::Result<bool> {
    state.db.call(|db|Ok(!db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running' OR (state='queued' AND available_at<=?1))",[now()],|r|r.get::<_,bool>(0))?)).await
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    state
        .db
        .call(|db| {
            db.execute(
                "UPDATE segment_analysis SET state='queued' WHERE state='running'",
                [],
            )?;
            Ok(())
        })
        .await?;
    loop {
        if idle(&state).await? {
            let cfg = config(&state).await.map_err(|e| anyhow::anyhow!(e.2))?;
            if cfg.local || cfg.external {
                let work=state.db.call(|db|{
                    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    tx.execute("INSERT OR IGNORE INTO segment_analysis(media_id,file_id,generation,state,requested_at) SELECT m.id,f.id,f.generation,'queued',m.created_at FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='episode' AND f.present=1 AND NOT EXISTS(SELECT 1 FROM segment_analysis a WHERE a.media_id=m.id AND a.file_id=f.id AND a.generation=f.generation) AND NOT EXISTS(SELECT 1 FROM media_sources other WHERE other.file_id=f.id AND other.media_id<>m.id) ORDER BY m.created_at DESC LIMIT 1000",[])?;
                    let work=tx.query_row("UPDATE segment_analysis SET state='running',error=NULL WHERE (media_id,file_id,generation)=(SELECT a.media_id,a.file_id,a.generation FROM segment_analysis a JOIN media_files f ON f.id=a.file_id AND f.generation=a.generation AND f.present=1 WHERE a.state='queued' ORDER BY a.requested_at DESC LIMIT 1) RETURNING media_id,file_id,generation",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
                    tx.commit()?;Ok(work)
                }).await?;
                if let Some((media, file, generation)) = work {
                    let outcome = analyze(&state, &media, &file, &generation, cfg).await;
                    let interrupted = !idle(&state).await?;
                    let message = outcome.err().map(|e| e.to_string());
                    let event_media = media.clone();
                    state.db.call(move|db|{db.execute("UPDATE segment_analysis SET state=?1,completed_at=?2,error=?3 WHERE media_id=?4 AND file_id=?5 AND generation=?6",params![if interrupted{"queued"}else if message.is_some(){"failed"}else{"complete"},now(),message,media,file,generation])?;Ok(())}).await?;
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
    let season = state
        .db
        .call(move |db| {
            let episode = db
                .query_row(
                    "SELECT parent_id FROM media WHERE id=?1 AND kind='episode'",
                    [mid],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            let shared = db.query_row(
                "SELECT COUNT(*) FROM media_sources WHERE file_id=?1",
                [fid],
                |r| r.get::<_, i64>(0),
            )? > 1;
            Ok(if shared { None } else { episode })
        })
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
            let peers=state.db.call({let season=season.clone();move|db|{
                db.execute("INSERT INTO segment_fingerprints VALUES (?1,?2,?3,?4,?5) ON CONFLICT(file_id,generation,window) DO UPDATE SET hashes=excluded.hashes,offset=excluded.offset",params![saved.id,saved.generation,kind,offset,serialized])?;
                Ok(db.prepare("SELECT m.id,f.id,f.generation,p.offset,p.hashes FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id AND f.present=1 JOIN segment_fingerprints p ON p.file_id=f.id AND p.generation=f.generation WHERE m.parent_id=?1 AND m.id<>?2 AND f.id<>?3 AND p.window=?4 ORDER BY m.created_at DESC LIMIT 8")?.query_map(params![season,saved.media_id,saved.id,kind],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,f64>(3)?,serde_json::from_str::<Vec<u32>>(&r.get::<_,String>(4)?).unwrap_or_default())))?.collect::<rusqlite::Result<Vec<_>>>()?)
            }}).await?;
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
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT present AND generation=?2 FROM media_files WHERE id=?1",params![src.id,src.generation],|r|r.get::<_,bool>(0))?;
        anyhow::ensure!(current,"Media generation changed during analysis");
        tx.execute("DELETE FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 AND source=?4 AND (?5 IS NULL OR kind=?5)",params![src.media_id,src.id,src.generation,origin,kind])?;
        for segment in segments {insert(&tx,&src,&segment,origin)?;}
        tx.commit()?;Ok(())
    }).await
}
async fn external(state: &AppState, src: &Source) -> anyhow::Result<()> {
    let media = src.media_id.clone();
    let coordinates=state.db.call(move|db|{
        Ok(db.query_row("SELECT p.series_id,p.season_number,p.episode_number FROM episode_mappings m JOIN provider_episodes p ON p.provider=m.provider AND p.episode_id=m.episode_id WHERE m.media_id=?1 AND m.provider='tmdb' AND m.state='confirmed' AND (SELECT COUNT(*) FROM episode_mappings WHERE media_id=?1)=1",[media],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).optional()?)
    }).await?;
    let Some((series, season, episode)) = coordinates else {
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
            ("tmdb_id", series.clone()),
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
    let segments = parse_external(&value, &series, season, episode, src.duration())?;
    store(state, src, segments, "theintrodb", None).await
}
pub(super) fn parse_external(
    value: &Value,
    series: &str,
    season: i64,
    episode: i64,
    duration: f64,
) -> anyhow::Result<Vec<Segment>> {
    anyhow::ensure!(
        value["tmdb_id"].as_i64() == series.parse::<i64>().ok()
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
