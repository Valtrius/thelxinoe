//! A complete VOD timeline for clients that seek by requesting HLS segments.
//! Segments are generated on demand and may be evicted and regenerated.
use crate::{Options, Source, bitrate};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex, Semaphore, watch},
};

struct Run {
    revision: String,
    directory: PathBuf,
    source: Source,
    options: Options,
    mode: String,
    boundaries: Vec<f64>,
    gate: Mutex<()>,
    canceled: watch::Sender<bool>,
}
pub struct VodCache {
    root: PathBuf,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    slots: Arc<Semaphore>,
}
impl VodCache {
    pub async fn open(root: &Path, slots: Arc<Semaphore>) -> Result<Self> {
        let root = root.join("vod");
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self {
            root,
            runs: Mutex::new(HashMap::new()),
            slots,
        })
    }
    pub async fn start(
        &self,
        id: &str,
        source: &Source,
        options: &Options,
        mode: &str,
    ) -> Result<String> {
        source.validate().await?;
        let duration = source.duration();
        if !duration.is_finite() || !(0.0..=86400.0).contains(&duration) || duration == 0.0 {
            bail!("Seekable conversion requires a duration of at most 24 hours");
        }
        let boundaries = if mode == "remux" && source.video_codec().is_some() {
            let _slot = self
                .slots
                .clone()
                .try_acquire_owned()
                .context("All conversion slots are busy")?;
            keyframes(source).await?
        } else {
            (0..(duration / 6.0).ceil() as usize)
                .map(|i| i as f64 * 6.0)
                .filter(|time| *time == 0.0 || *time < duration - 0.25)
                .chain(std::iter::once(duration))
                .collect()
        };
        self.stop(id).await;
        let mut runs = self.runs.lock().await;
        if runs.len() >= 4 {
            bail!("All four seekable conversion sessions are busy");
        }
        if fs2::available_space(&self.root)? < 1024 * 1024 * 1024 {
            bail!("Conversion requires at least 1 GB of free cache space");
        }
        let revision = uuid::Uuid::new_v4().to_string();
        let directory = self.root.join(&revision);
        tokio::fs::create_dir(&directory).await?;
        let target = boundaries
            .windows(2)
            .map(|s| (s[1] - s[0]).ceil() as u64)
            .max()
            .unwrap_or(6);
        let mut manifest = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-INDEPENDENT-SEGMENTS\n"
        );
        for (index, times) in boundaries.windows(2).enumerate() {
            // Independent muxer runs need a discontinuity even when remuxed
            // packets retain their original presentation timestamps.
            if index > 0 {
                manifest.push_str("#EXT-X-DISCONTINUITY\n");
            }
            manifest.push_str(&format!(
                "#EXTINF:{:.6},\nsegment-{index:06}.ts\n",
                times[1] - times[0]
            ));
        }
        manifest.push_str("#EXT-X-ENDLIST\n");
        tokio::fs::write(directory.join("index.m3u8"), manifest).await?;
        runs.insert(
            id.to_owned(),
            Arc::new(Run {
                revision: revision.clone(),
                directory,
                source: source.clone(),
                options: options.clone(),
                mode: mode.into(),
                boundaries,
                gate: Mutex::new(()),
                canceled: watch::channel(false).0,
            }),
        );
        Ok(revision)
    }
    pub async fn file(&self, id: &str, revision: &str, name: &str) -> Result<Option<PathBuf>> {
        let run = self
            .runs
            .lock()
            .await
            .get(id)
            .filter(|r| r.revision == revision)
            .cloned();
        let Some(run) = run else {
            return Ok(None);
        };
        if name == "index.m3u8" {
            return Ok(Some(run.directory.join(name)));
        }
        let Some(index) = name
            .strip_prefix("segment-")
            .and_then(|s| s.strip_suffix(".ts"))
            .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<usize>().ok())
        else {
            return Ok(None);
        };
        if index + 1 >= run.boundaries.len() {
            return Ok(None);
        }
        let _gate = tokio::time::timeout(Duration::from_secs(45), run.gate.lock())
            .await
            .context("Segment preparation is busy")?;
        if *run.canceled.borrow() {
            return Ok(None);
        }
        let path = run.directory.join(name);
        if tokio::fs::try_exists(&path).await? {
            return Ok(Some(path));
        }
        run.source.validate().await?;
        if fs2::available_space(&run.directory)? < 512 * 1024 * 1024 {
            bail!("Insufficient conversion cache space");
        }
        let _slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("All conversion slots are busy")?;
        let temp = run.directory.join(format!("{name}.tmp"));
        let start = run.boundaries[index];
        let duration = run.boundaries[index + 1] - start;
        let remux_video = run.mode == "remux" && run.source.video_codec().is_some();
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-ss",
                &start.to_string(),
                "-i",
            ])
            .arg(&run.source.path);
        if remux_video {
            command.arg("-copyts");
        } else {
            command.args(["-t", &duration.to_string()]);
        }
        if run.source.video_codec().is_some() {
            command.args(["-map", "0:v:0"]);
        }
        command
            .arg("-map")
            .arg(
                run.options
                    .audio
                    .map(|n| format!("0:{n}"))
                    .unwrap_or_else(|| "0:a:0?".into()),
            )
            .args(["-sn", "-dn", "-map_metadata", "-1"]);
        if remux_video {
            let origin = run.source.probe["format"]["start_time"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            // Input seeking may retain an earlier GOP. Split at every keyframe
            // and select the indexed interval, preserving codec headers and
            // original timestamps across FFmpeg versions.
            clean_work(&run.directory).await?;
            command
                .args([
                    "-c",
                    "copy",
                    "-to",
                    &(origin + start + duration + 0.1).to_string(),
                    "-f",
                    "segment",
                    "-segment_time",
                    "0.000001",
                    "-segment_list_type",
                    "csv",
                    "-segment_list",
                ])
                .arg(run.directory.join("work.csv"))
                .args([
                    "-segment_format",
                    "mpegts",
                    "-segment_format_options",
                    "mpegts_copyts=1:mpegts_flags=+resend_headers:avoid_negative_ts=disabled",
                    "-reset_timestamps",
                    "0",
                    "-avoid_negative_ts",
                    "disabled",
                ])
                .arg(run.directory.join("work-%03d.ts"));
        } else if run.mode == "remux" {
            command.args(["-c:a", "copy"]);
        } else {
            let rate = bitrate(&run.options.quality)?.unwrap_or(8_000_000);
            command.args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-vf",
                "scale=w='min(1920,iw)':h=-2",
                "-b:v",
                &rate.to_string(),
                "-maxrate",
                &rate.to_string(),
                "-bufsize",
                &(rate * 2).to_string(),
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-ac",
                "2",
            ]);
        }
        if !remux_video {
            command
                .args([
                    "-avoid_negative_ts",
                    "make_zero",
                    "-muxdelay",
                    "0",
                    "-muxpreload",
                    "0",
                    "-f",
                    "mpegts",
                    "-fs",
                    "100663296",
                ])
                .arg(&temp);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut canceled = run.canceled.subscribe();
        if *canceled.borrow() {
            bail!("Playback was stopped");
        }
        let mut child = command.spawn()?;
        let status = tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(40),child.wait()) =>
                result.context("Segment conversion timed out").and_then(|r|r.map_err(Into::into)),
            _=canceled.changed()=>Err(anyhow::anyhow!("Playback was stopped")),
            result=monitor_work(&run.directory)=>Err(result),
        };
        if status.is_err() {
            let _ = child.kill().await;
            let _ = tokio::fs::remove_file(&temp).await;
            let _ = clean_work(&run.directory).await;
        }
        let status = status?;
        if status.success() && remux_video {
            let selected = select_part(&run, start, duration).await;
            if let Ok(part) = &selected {
                tokio::fs::rename(part, &temp).await?;
            }
            clean_work(&run.directory).await?;
            selected?;
        }
        let size = tokio::fs::metadata(&temp)
            .await
            .map(|v| v.len())
            .unwrap_or(0);
        if !status.success() || size == 0 || size >= 96 * 1024 * 1024 {
            let _ = tokio::fs::remove_file(&temp).await;
            bail!("Unable to prepare a bounded playback segment");
        }
        tokio::fs::rename(&temp, &path).await?;
        trim(&run.directory, &path).await?;
        Ok(Some(path))
    }
    pub async fn stop(&self, id: &str) {
        let run = self.runs.lock().await.remove(id);
        if let Some(run) = run {
            run.canceled.send_replace(true);
            let _gate = run.gate.lock().await;
            let _ = tokio::fs::remove_dir_all(&run.directory).await;
        }
    }
    pub async fn maintain(&self, active: &[String], budget: u64) -> Result<Vec<String>> {
        let runs = self
            .runs
            .lock()
            .await
            .iter()
            .map(|(id, run)| (id.clone(), run.clone()))
            .collect::<Vec<_>>();
        let mut stopped = Vec::new();
        let mut total = 0;
        let low = fs2::available_space(&self.root)? < 512 * 1024 * 1024;
        for (id, run) in runs {
            let mut entries = match tokio::fs::read_dir(&run.directory).await {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let mut size = 0;
            while let Some(entry) = entries.next_entry().await? {
                match entry.metadata().await {
                    Ok(meta) => size += meta.len(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            total += size;
            if !active.contains(&id) || low || total > budget || size > 288 * 1024 * 1024 {
                stopped.push(id);
            }
        }
        for id in &stopped {
            self.stop(id).await;
        }
        Ok(stopped)
    }
}
async fn trim(directory: &Path, current: &Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let mut files = Vec::new();
    let mut total = 0;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().extension().is_some_and(|e| e == "ts") {
            let meta = entry.metadata().await?;
            total += meta.len();
            files.push((entry.path(), meta.len(), meta.modified()?));
        }
    }
    files.sort_by_key(|item| item.2);
    let mut count = files.len();
    for (path, size, _) in files {
        if total <= 192 * 1024 * 1024 && count <= 24 {
            break;
        }
        if path != current && tokio::fs::remove_file(path).await.is_ok() {
            total -= size;
            count -= 1;
        }
    }
    Ok(())
}
async fn keyframes(source: &Source) -> Result<Vec<f64>> {
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-skip_frame",
            "nokey",
            "-select_streams",
            "v:0",
            "-show_frames",
            "-show_entries",
            "frame=best_effort_timestamp_time",
            "-of",
            "json",
        ])
        .arg(&source.path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .context("Missing keyframe output")?
        .take(8 * 1024 * 1024 + 1);
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), stdout.read_to_end(&mut bytes))
        .await
        .context("Keyframe indexing timed out")??;
    if bytes.len() > 8 * 1024 * 1024 {
        bail!("Keyframe index exceeds its size limit");
    }
    if !tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await??
        .success()
    {
        bail!("Keyframe indexing failed");
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let origin = source.probe["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut result = vec![0.0];
    let duration = source.duration();
    // The first keyframe may follow a short audio encoder delay. Its media
    // interval still begins at zero; do not create a tiny audio-only segment.
    for frame in value["frames"].as_array().into_iter().flatten().skip(1) {
        if let Some(time) = frame["best_effort_timestamp_time"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|n| n - origin)
            .filter(|v| v.is_finite() && *v < duration - 0.05)
            && time - result.last().unwrap() > 0.001
        {
            result.push(time);
        }
    }
    result.push(duration);
    if result.windows(2).any(|s| s[1] - s[0] > 30.0) {
        bail!(
            "Keyframes are too far apart for seekable remux; select a lower quality to convert video"
        );
    }
    Ok(result)
}

async fn clean_work(directory: &Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("work-") || name == "work.csv" {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}
async fn monitor_work(directory: &Path) -> anyhow::Error {
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let size = async {
            let mut total = 0u64;
            let mut entries = tokio::fs::read_dir(directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("work-") || name.ends_with(".tmp") {
                    total += entry.metadata().await?.len();
                }
            }
            anyhow::Ok(total)
        }
        .await;
        match size {
            Ok(size) if size < 192 * 1024 * 1024 => {}
            Ok(_) => return anyhow::anyhow!("Conversion work exceeds its cache limit"),
            Err(error) => return error,
        }
    }
}
async fn select_part(run: &Run, start: f64, duration: f64) -> Result<PathBuf> {
    let origin = run.source.probe["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let start = start + origin;
    let csv = tokio::fs::read_to_string(run.directory.join("work.csv")).await?;
    for (index, line) in csv.lines().enumerate() {
        let mut fields = line.rsplitn(3, ',');
        let end = fields
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .context("Invalid segment end")?;
        let mut begin = fields
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .context("Invalid segment start")?;
        let file = fields.next().context("Missing segment name")?;
        let file = Path::new(file.trim_matches('"'))
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid segment name")?;
        if !file.starts_with("work-")
            || !file.ends_with(".ts")
            || file.len() != 11
            || !file[5..8].bytes().all(|b| b.is_ascii_digit())
        {
            bail!("Invalid segment name");
        }
        let path = run.directory.join(file);
        // The segment muxer reports zero for its first entry even when input
        // seeking starts later. Read that entry's first video packet instead.
        if index == 0 {
            let mut command = Command::new("ffprobe");
            command
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-read_intervals",
                    "%+#1",
                    "-show_entries",
                    "packet=pts_time",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let output = tokio::time::timeout(Duration::from_secs(5), command.output()).await??;
            if !output.status.success() || output.stdout.len() > 8192 {
                bail!("Cannot verify remux timing");
            }
            let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            begin = value["packets"][0]["pts_time"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .context("Missing remux timestamp")?;
        }
        if (begin - start).abs() < 0.05 && end >= start + duration - 0.1 {
            return Ok(path);
        }
    }
    bail!("The requested keyframe interval could not be remuxed")
}
