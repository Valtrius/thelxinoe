use crate::{Options, Source, bitrate};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};

struct Running {
    child: Child,
    directory: PathBuf,
    revision: String,
}
pub struct Pipelines {
    root: PathBuf,
    running: Mutex<HashMap<String, Running>>,
}
impl Pipelines {
    pub async fn open(cache: &Path) -> Result<Self> {
        let root = cache.join("playback");
        // Only our dedicated cache subtree is disposable. No media/state paths enter it.
        if root.exists() {
            tokio::fs::remove_dir_all(&root).await?;
        }
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self {
            root,
            running: Mutex::new(HashMap::new()),
        })
    }
    pub async fn start(
        &self,
        id: &str,
        source: &Source,
        options: &Options,
        mode: &str,
        start: f64,
    ) -> Result<(String, f64)> {
        source.validate().await?;
        let timeline_start = if mode == "remux" && start > 0.0 && source.video_codec().is_some() {
            keyframe_start(source, start).await?
        } else {
            start
        };
        let mut running = self.running.lock().await;
        if let Some(mut old) = running.remove(id) {
            let _ = old.child.kill().await;
            let _ = tokio::fs::remove_dir_all(old.directory).await;
        }
        if running.len() >= 4 {
            bail!(
                "All four conversion slots are busy; try direct playback or wait for a session to finish"
            );
        }
        if fs2::available_space(&self.root)? < 1024 * 1024 * 1024 {
            bail!("Conversion requires at least 1 GB of free cache space");
        }
        let revision = uuid::Uuid::new_v4().to_string();
        let directory = self.root.join(&revision);
        tokio::fs::create_dir(&directory).await?;
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-readrate",
                "1",
                "-ss",
                &timeline_start.to_string(),
                "-seek_timestamp",
                "1",
                "-i",
            ])
            .arg(&source.path);
        if source.video_codec().is_some() {
            command.args(["-map", "0:v:0"]);
        }
        command
            .args(["-map"])
            .arg(
                options
                    .audio
                    .map(|n| format!("0:{n}"))
                    .unwrap_or_else(|| "0:a:0?".into()),
            )
            .args(["-sn", "-dn", "-map_metadata", "-1"]);
        if mode == "remux" {
            command.args(["-c", "copy"]);
        } else {
            let rate = bitrate(&options.quality)?.unwrap_or(8_000_000).to_string();
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
                &rate,
                "-maxrate",
                &rate,
                "-bufsize",
                &format!("{}", rate.parse::<u32>()? * 2),
                "-force_key_frames",
                "expr:gte(t,n_forced*6)",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-ac",
                "2",
            ]);
        }
        command
            .args([
                "-avoid_negative_ts",
                "make_zero",
                "-f",
                "hls",
                "-hls_time",
                "6",
                "-hls_list_size",
                "20",
                "-hls_delete_threshold",
                "2",
                "-hls_flags",
                "delete_segments+independent_segments+temp_file",
                "-hls_segment_filename",
            ])
            .arg(directory.join("segment-%06d.ts"))
            .arg(directory.join("index.m3u8"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command
            .spawn()
            .context("Start FFmpeg playback conversion")?;
        running.insert(
            id.to_owned(),
            Running {
                child,
                directory: directory.clone(),
                revision: revision.clone(),
            },
        );
        drop(running);
        for _ in 0..200 {
            if tokio::fs::try_exists(directory.join("index.m3u8")).await? {
                return Ok((revision, timeline_start));
            }
            {
                let mut runs = self.running.lock().await;
                let Some(run) = runs.get_mut(id) else {
                    bail!("Playback was stopped");
                };
                if run.revision != revision {
                    bail!("Playback was superseded");
                }
                if run.child.try_wait()?.is_some() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.stop(id).await;
        bail!("FFmpeg could not prepare playback within 20 seconds")
    }
    pub async fn file(&self, id: &str, revision: &str, name: &str) -> Option<PathBuf> {
        let valid = name == "index.m3u8"
            || (name.starts_with("segment-")
                && name.ends_with(".ts")
                && name.len() == 17
                && name[8..14].bytes().all(|v| v.is_ascii_digit()));
        if !valid {
            return None;
        }
        self.running
            .lock()
            .await
            .get(id)
            .filter(|run| run.revision == revision)
            .map(|run| run.directory.join(name))
    }
    pub async fn stop(&self, id: &str) {
        if let Some(mut run) = self.running.lock().await.remove(id) {
            let _ = run.child.kill().await;
            let _ = tokio::fs::remove_dir_all(run.directory).await;
        }
    }
    pub async fn maintain(&self, active: &[String]) -> Result<Vec<String>> {
        let mut runs = self.running.lock().await;
        let mut stop = Vec::new();
        let mut total = 0u64;
        let low = fs2::available_space(&self.root)? < 512 * 1024 * 1024;
        for (id, run) in runs.iter_mut() {
            let mut size = 0;
            let mut entries = tokio::fs::read_dir(&run.directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                size += entry.metadata().await?.len();
            }
            total += size;
            let failed = run
                .child
                .try_wait()?
                .is_some_and(|status| !status.success());
            if !active.contains(id)
                || failed
                || low
                || size > 768 * 1024 * 1024
                || total > 2 * 1024 * 1024 * 1024
            {
                stop.push(id.clone());
            }
        }
        for id in &stop {
            if let Some(mut run) = runs.remove(id) {
                let _ = run.child.kill().await;
                let _ = tokio::fs::remove_dir_all(run.directory).await;
            }
        }
        Ok(stop)
    }
}

async fn keyframe_start(source: &Source, position: f64) -> Result<f64> {
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-skip_frame",
            "nokey",
            "-read_intervals",
            &format!("{}%+60", (position - 30.0).max(0.0)),
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
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    if !output.status.success() || output.stdout.len() > 1024 * 1024 {
        bail!("Unable to locate a safe keyframe for seeking");
    }
    let frames: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    frames["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            f["best_effort_timestamp_time"]
                .as_str()?
                .parse::<f64>()
                .ok()
        })
        .filter(|t| t.is_finite() && *t >= 0.0 && *t <= position + 0.001)
        .max_by(f64::total_cmp)
        .context("No keyframe was found before the requested position")
}
