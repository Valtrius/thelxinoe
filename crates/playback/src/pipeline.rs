use crate::{Options, RemoteSource, Source, conversion_args};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
};

struct Running {
    _slot: OwnedSemaphorePermit,
    child: Child,
    directory: PathBuf,
    revision: String,
}
enum Input<'a> {
    Local(&'a Source),
    Remote(&'a RemoteSource),
}
pub struct Pipelines {
    root: PathBuf,
    running: Mutex<HashMap<String, Running>>,
    slots: Arc<Semaphore>,
    vod: crate::vod::VodCache,
}
impl Pipelines {
    pub async fn open(cache: &Path) -> Result<Self> {
        let root = cache.join("playback");
        // Only our dedicated cache subtree is disposable. No media/state paths enter it.
        if root.exists() {
            tokio::fs::remove_dir_all(&root).await?;
        }
        tokio::fs::create_dir_all(&root).await?;
        let slots = Arc::new(Semaphore::new(4));
        let vod = crate::vod::VodCache::open(&root, slots.clone()).await?;
        Ok(Self {
            root,
            running: Mutex::new(HashMap::new()),
            slots,
            vod,
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
        self.start_input(id, Input::Local(source), options, mode, timeline_start)
            .await
    }
    pub async fn start_remote(
        &self,
        id: &str,
        source: &RemoteSource,
        options: &Options,
        mode: &str,
        start: f64,
    ) -> Result<(String, f64)> {
        source.validate()?;
        if !start.is_finite() || start < 0.0 {
            bail!("Invalid stream position");
        }
        let start = if mode == "remux" && start > 0.0 {
            remote_keyframe_start(source, start).await?
        } else {
            start
        };
        self.start_input(
            id,
            Input::Remote(source),
            options,
            mode,
            if source.live { 0.0 } else { start },
        )
        .await
    }
    async fn start_input(
        &self,
        id: &str,
        input: Input<'_>,
        options: &Options,
        mode: &str,
        timeline_start: f64,
    ) -> Result<(String, f64)> {
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
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("All four conversion slots are busy")?;
        if fs2::available_space(&self.root)? < 1024 * 1024 * 1024 {
            bail!("Conversion requires at least 1 GB of free cache space");
        }
        let revision = uuid::Uuid::new_v4().to_string();
        let directory = self.root.join(&revision);
        tokio::fs::create_dir(&directory).await?;
        // Publish online streams sooner, retaining the two-minute live window.
        let online = matches!(input, Input::Remote(_));
        let segment_seconds = if online { "2" } else { "6" };
        let mut command = Command::new("ffmpeg");
        if let Input::Remote(source) = &input {
            // FFREPORT may otherwise write signed upstream addresses to disk.
            command.env_remove("FFREPORT");
            command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
            for address in std::iter::once(&source.video).chain(source.audio.iter()) {
                command.args([
                    "-protocol_whitelist",
                    "https,tls,tcp,crypto",
                    "-rw_timeout",
                    "15000000",
                ]);
                if !source.live {
                    command.args([
                        "-readrate",
                        "1",
                        "-readrate_initial_burst",
                        "30",
                        "-ss",
                        &timeline_start.to_string(),
                    ]);
                }
                command.args(["-i", address]);
            }
            command.args([
                "-map",
                "0:v:0",
                "-map",
                if source.audio.is_some() {
                    "1:a:0"
                } else {
                    "0:a:0?"
                },
            ]);
        } else if let Input::Local(source) = &input {
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
            command.args(["-map"]).arg(
                options
                    .audio
                    .map(|n| format!("0:{n}"))
                    .unwrap_or_else(|| "0:a:0?".into()),
            );
        }
        command.args(["-sn", "-dn", "-map_metadata", "-1"]);
        if mode == "remux" {
            command.args(["-c", "copy"]);
        } else {
            command.args(conversion_args(options)?).args([
                "-force_key_frames",
                &format!("expr:gte(t,n_forced*{segment_seconds})"),
            ]);
        }
        let fragmented = online && mode == "remux";
        if fragmented {
            command.args([
                "-hls_segment_type",
                "fmp4",
                "-hls_fmp4_init_filename",
                "init.mp4",
            ]);
        }
        command
            .args([
                "-avoid_negative_ts",
                "make_zero",
                "-f",
                "hls",
                "-hls_time",
                segment_seconds,
                "-hls_list_size",
                if online { "60" } else { "20" },
                "-hls_delete_threshold",
                "2",
                "-hls_flags",
                "delete_segments+independent_segments+temp_file",
                "-hls_segment_filename",
            ])
            .arg(directory.join(if fragmented {
                "segment-%06d.m4s"
            } else {
                "segment-%06d.ts"
            }))
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
                _slot: slot,
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
    pub async fn start_vod(
        &self,
        id: &str,
        source: &Source,
        options: &Options,
        mode: &str,
    ) -> Result<(String, f64)> {
        Ok((self.vod.start(id, source, options, mode).await?, 0.0))
    }
    pub async fn start_remote_vod(
        &self,
        id: &str,
        source: &RemoteSource,
        duration: f64,
        options: &Options,
    ) -> Result<(String, f64)> {
        Ok((
            self.vod.start_remote(id, source, duration, options).await?,
            0.0,
        ))
    }
    pub async fn file(&self, id: &str, revision: &str, name: &str) -> Result<Option<PathBuf>> {
        let valid = name == "index.m3u8"
            || name == "init.mp4"
            || (name.starts_with("segment-")
                && ((name.ends_with(".ts") && name.len() == 17)
                    || (name.ends_with(".m4s") && name.len() == 18))
                && name[8..14].bytes().all(|v| v.is_ascii_digit()));
        if !valid {
            return Ok(None);
        }
        if let Some(path) = self.vod.file(id, revision, name).await? {
            return Ok(Some(path));
        }
        Ok(self
            .running
            .lock()
            .await
            .get(id)
            .filter(|run| run.revision == revision)
            .map(|run| run.directory.join(name)))
    }
    pub async fn stop(&self, id: &str) {
        self.vod.stop(id).await;
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
        drop(runs);
        stop.extend(
            self.vod
                .maintain(active, (2u64 * 1024 * 1024 * 1024).saturating_sub(total))
                .await?,
        );
        Ok(stop)
    }
}

async fn keyframe_start(source: &Source, position: f64) -> Result<f64> {
    probe_keyframe(&source.path.to_string_lossy(), position, false).await
}
async fn remote_keyframe_start(source: &RemoteSource, position: f64) -> Result<f64> {
    source.validate()?;
    probe_keyframe(&source.video, position, true).await
}
async fn probe_keyframe(input: &str, position: f64, remote: bool) -> Result<f64> {
    let mut command = Command::new("ffprobe");
    if remote {
        command.env_remove("FFREPORT").args([
            "-protocol_whitelist",
            "https,tls,tcp,crypto",
            "-rw_timeout",
            "10000000",
        ]);
    }
    if remote {
        // The demuxer seeks to the preceding keyframe using the file index.
        // Inspect packets there instead of downloading and decoding a minute.
        command.args([
            "-read_intervals",
            &format!("{position}%+0.1"),
            "-show_packets",
            "-show_entries",
            "packet=pts_time,flags",
        ]);
    } else {
        command.args([
            "-skip_frame",
            "nokey",
            "-read_intervals",
            &format!("{}%+60", (position - 30.0).max(0.0)),
            "-show_frames",
            "-show_entries",
            "frame=best_effort_timestamp_time",
        ]);
    }
    command
        .args(["-v", "error", "-select_streams", "v:0", "-of", "json"])
        .arg(input)
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
    frames[if remote { "packets" } else { "frames" }]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            if remote && !f["flags"].as_str().is_some_and(|flags| flags.contains('K')) {
                return None;
            }
            f[if remote {
                "pts_time"
            } else {
                "best_effort_timestamp_time"
            }]
            .as_str()?
            .parse::<f64>()
            .ok()
        })
        .filter(|t| t.is_finite() && *t >= 0.0 && *t <= position + 0.001)
        .max_by(f64::total_cmp)
        .context("No keyframe was found before the requested position")
}
