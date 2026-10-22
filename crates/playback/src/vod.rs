//! A complete VOD timeline for clients that seek by requesting HLS segments.
//! Segments are generated on demand and may be evicted and regenerated.
use crate::{
    CleanupReason, Decision, Encoding, Options, RemoteSource, RunHandle, Source, StoppedRun,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use thelxinoe_tools::{MediaTools, Runtime};
use tokio::{
    io::AsyncBufReadExt,
    process::Command,
    sync::{Mutex, OnceCell, Semaphore, watch},
};

struct Run {
    tools: MediaTools,
    revision: String,
    directory: PathBuf,
    source: Source,
    remote: Option<RemoteSource>,
    options: Options,
    encoding: Encoding,
    boundaries: Vec<f64>,
    video_start: f64,
    gate: Mutex<Option<RemoteWork>>,
    canceled: watch::Sender<bool>,
}
struct RemoteWork {
    first: usize,
    end: usize,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Drop for RemoteWork {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub struct VodCache {
    scanner: Arc<crate::cache::Scanner>,
    tools: Runtime,
    root: PathBuf,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    slots: Arc<Semaphore>,
    /// Keyframe indexes of local files by file generation; see `index`.
    indexes: Mutex<HashMap<String, Arc<OnceCell<Keyframes>>>>,
}
type Keyframes = (Vec<f64>, f64);
impl VodCache {
    pub async fn open(
        root: &Path,
        slots: Arc<Semaphore>,
        tools: Runtime,
        scanner: Arc<crate::cache::Scanner>,
    ) -> Result<Self> {
        let root = root.join("vod");
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self {
            scanner,
            tools,
            root,
            runs: Mutex::new(HashMap::new()),
            slots,
            indexes: Mutex::new(HashMap::new()),
        })
    }
    /// A local file's keyframe index, built once per file generation. Listing a file's
    /// packets reads all of it, which can outlast a client's patience on large files: the
    /// index builds in its own task with a budget scaled to the file, a start waits for it
    /// at most 30 seconds, and the next start reuses it.
    pub(crate) async fn index(&self, source: &Source, tools: &MediaTools) -> Result<Keyframes> {
        let key = format!(
            "{}\0{}\0{}",
            source.path.display(),
            source.size,
            source.modified
        );
        let cell = {
            let mut indexes = self.indexes.lock().await;
            if indexes.len() >= 256 {
                indexes.retain(|_, cell| cell.initialized());
                if indexes.len() >= 256 {
                    indexes.clear();
                }
            }
            indexes.entry(key).or_default().clone()
        };
        let (source, tools, slots) = (source.clone(), tools.clone(), self.slots.clone());
        let task = tokio::spawn(async move {
            cell.get_or_try_init(|| async {
                let _slot = slots.acquire_owned().await?;
                let budget = Duration::from_secs(60 + source.size / (64 << 20));
                keyframes(&source, &tools, None, budget).await
            })
            .await
            .cloned()
        });
        match tokio::time::timeout(Duration::from_secs(30), task).await {
            Ok(index) => index?,
            Err(_) => bail!(
                "The keyframe index of this large file is still being built; start again shortly"
            ),
        }
    }
    pub async fn start(
        &self,
        id: &str,
        source: &Source,
        options: &Options,
        mode: &str,
    ) -> Result<String> {
        self.start_input(
            id,
            source,
            options,
            Encoding::for_mode(source, options, mode)?,
            None,
        )
        .await
    }
    pub async fn start_remote(
        &self,
        id: &str,
        remote: &RemoteSource,
        source: &Source,
        options: &Options,
        decision: &Decision,
    ) -> Result<String> {
        remote.validate()?;
        if remote.live {
            bail!("A live stream has no VOD timeline");
        }
        let mut source = source.clone();
        if !remote.segments.is_empty()
            && let Some(start) = source.probe["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["codec_type"] == "video")
                .and_then(|s| s["start_time"].as_str())
                .map(str::to_owned)
        {
            source.probe["format"]["start_time"] = serde_json::json!(start);
        }
        self.start_input(
            id,
            &source,
            options,
            decision.encoding,
            Some(remote.clone()),
        )
        .await
    }
    async fn start_input(
        &self,
        id: &str,
        source: &Source,
        options: &Options,
        encoding: Encoding,
        remote: Option<RemoteSource>,
    ) -> Result<String> {
        if remote.is_none() {
            source.validate().await?;
        }
        let tools = self.tools.media()?;
        tools.ffmpeg.verify_unchanged().await?;
        tools.ffprobe.verify_unchanged().await?;
        let duration = source.duration();
        if !duration.is_finite() || !(0.0..=86400.0).contains(&duration) || duration == 0.0 {
            bail!("Seekable conversion requires a duration of at most 24 hours");
        }
        // Remote windows prepare adjacent segments over the same connection.
        // Six-second segments also keep conversion overhead bounded on TVs.
        let segment = 6.0;
        let (boundaries, video_start) = if !encoding.video && source.video_codec().is_some() {
            if let Some(remote) = &remote {
                let _slot = self
                    .slots
                    .clone()
                    .try_acquire_owned()
                    .context("All conversion slots are busy")?;
                keyframes(source, &tools, Some(remote), Duration::from_secs(30)).await
            } else {
                self.index(source, &tools).await
            }
            .context("Source keyframe index is unavailable")?
        } else {
            (
                (0..(duration / segment).ceil() as usize)
                    .map(|i| i as f64 * segment)
                    .filter(|time| *time == 0.0 || *time < duration - 0.25)
                    .chain(std::iter::once(duration))
                    .collect(),
                0.0,
            )
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
        let fragmented =
            remote.is_some() && options.capabilities.hls_container.as_deref() == Some("mp4");
        if fragmented {
            manifest = manifest.replace("#EXT-X-VERSION:3", "#EXT-X-VERSION:7");
        }
        for (index, times) in boundaries.windows(2).enumerate() {
            // Independent muxer runs need a discontinuity even when remuxed
            // packets retain their original presentation timestamps.
            if index > 0 {
                manifest.push_str("#EXT-X-DISCONTINUITY\n");
            }
            if fragmented {
                manifest.push_str(&format!("#EXT-X-MAP:URI=\"init-{index:06}.mp4\"\n"));
            }
            manifest.push_str(&format!(
                "#EXTINF:{:.6},\nsegment-{index:06}.{}\n",
                times[1] - times[0],
                if fragmented { "m4s" } else { "ts" }
            ));
        }
        manifest.push_str("#EXT-X-ENDLIST\n");
        tokio::fs::write(directory.join("index.m3u8"), manifest).await?;
        runs.insert(
            id.to_owned(),
            Arc::new(Run {
                tools,
                revision: revision.clone(),
                directory,
                source: source.clone(),
                remote,
                options: options.clone(),
                encoding,
                boundaries,
                video_start,
                gate: Mutex::new(None),
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
        let fragmented = run.remote.is_some()
            && run.options.capabilities.hls_container.as_deref() == Some("mp4");
        let init = name.starts_with("init-");
        let Some(index) = name
            .strip_prefix(if init { "init-" } else { "segment-" })
            .and_then(|s| {
                s.strip_suffix(if init {
                    ".mp4"
                } else if fragmented {
                    ".m4s"
                } else {
                    ".ts"
                })
            })
            .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<usize>().ok())
        else {
            return Ok(None);
        };
        if index + 1 >= run.boundaries.len() {
            return Ok(None);
        }
        let mut gate = tokio::time::timeout(Duration::from_secs(45), run.gate.lock())
            .await
            .context("Segment preparation is busy")?;
        if *run.canceled.borrow() {
            return Ok(None);
        }
        let path = run.directory.join(if init {
            format!("segment-{index:06}.m4s")
        } else {
            name.into()
        });
        if tokio::fs::try_exists(&path).await? {
            if let Some(work) = gate.as_ref() {
                publish_initializations(&run, work).await?;
            }
            return Ok(Some(if init { run.directory.join(name) } else { path }));
        }
        if run.remote.is_some() && run.encoding.video {
            let path = self.remote_file(&run, index, path, &mut gate).await?;
            return Ok(Some(if init { run.directory.join(name) } else { path }));
        }
        if run.remote.is_none() {
            run.source.validate().await?;
        }
        if fs2::available_space(&run.directory)? < 512 * 1024 * 1024 {
            bail!("Insufficient conversion cache space");
        }
        let _slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("All conversion slots are busy")?;
        // The gate makes this the directory's only conversion. Dropped after the children
        // below, so a killed conversion's output goes too.
        let _partials = Partials {
            directory: run.directory.clone(),
            work: true,
        };
        let temp = path.with_extension("tmp");
        let start = run.boundaries[index];
        let duration = run.boundaries[index + 1] - start;
        let remux_video = !run.encoding.video && run.source.video_codec().is_some();
        let mut command = Command::new(&run.tools.ffmpeg.path);
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-fflags",
            "+genpts",
        ]);
        // Some AVI indexes skip the first GOP even for an explicit -ss 0.
        if start > 0.0 && run.remote.is_none() {
            command.args(["-ss", &start.to_string()]);
        }
        if let Some(remote) = &run.remote {
            command.env_remove("FFREPORT");
            for address in std::iter::once(&remote.video).chain(remote.audio.iter()) {
                command
                    .args(remote.network_args())
                    .args(["-rw_timeout", "15000000"]);
                if start > 0.0 {
                    command.args(["-ss", &start.to_string()]);
                }
                command.args(["-i", address]);
            }
        } else {
            command.arg("-i").arg(&run.source.path);
        }
        command.args(run.encoding.args(&run.options, &run.source.probe)?);
        if remux_video {
            command.arg("-copyts");
        } else if index + 2 < run.boundaries.len() {
            // The final fragment runs to EOF: container duration can omit the
            // last delayed frame or audio packet. Work remains time/size bounded.
            command.args(["-t", &duration.to_string()]);
        }
        if run.source.video_codec().is_some() {
            command.args(["-map", "0:v:0"]);
        }
        command
            .arg("-map")
            .arg(if run.remote.as_ref().is_some_and(|r| r.audio.is_some()) {
                "1:a:0".into()
            } else {
                run.options
                    .audio
                    .map(|n| format!("0:{n}"))
                    .unwrap_or_else(|| "0:a:0?".into())
            })
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
                    if fragmented { "mp4" } else { "mpegts" },
                    "-segment_format_options",
                    if fragmented {
                        "avoid_negative_ts=disabled"
                    } else {
                        "mpegts_copyts=1:mpegts_flags=+resend_headers:avoid_negative_ts=disabled"
                    },
                    "-reset_timestamps",
                    "0",
                    "-avoid_negative_ts",
                    "disabled",
                ])
                .arg(run.directory.join(if fragmented {
                    "work-%03d.mp4"
                } else {
                    "work-%03d.ts"
                }));
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
            bail!("Unable to prepare a bounded playback segment");
        }
        if fragmented {
            // Verify the copied interval before MP4 normalizes decode times.
            // Each HLS interval has its own discontinuity and initialization.
            let muxed = run.directory.join(format!("fragment-{index:06}.tmp"));
            let mut command = Command::new(&run.tools.ffmpeg.path);
            command
                .env_remove("FFREPORT")
                .args(["-v", "error", "-nostdin", "-y", "-copyts", "-i"])
                .arg(&temp)
                .args(["-map", "0", "-c", "copy"]);
            if start > 0.0 && !run.encoding.audio {
                // PTS would trim valid audio alongside B frames. Drop only
                // packets that belong before this GOP's decode boundary.
                let (_, dts) = packet_start(&run.tools, &temp).await?;
                command.args([
                    "-bsf:a",
                    &format!("noise=amount=0:drop='lt(pts*tb,{dts:.9})'"),
                ]);
            }
            command
                .args([
                    "-movflags",
                    "+frag_keyframe+empty_moov+default_base_moof",
                    "-avoid_negative_ts",
                    "make_zero",
                    "-f",
                    "mp4",
                ])
                .arg(&muxed)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let mut child = command.spawn()?;
            let status = tokio::select! { result=tokio::time::timeout(Duration::from_secs(20),child.wait())=>result.context("Media muxing timed out")??,_=canceled.changed()=>{let _=child.kill().await;bail!("Playback was stopped");} };
            if !status.success() {
                bail!("Unable to mux original packets");
            }
            tokio::fs::rename(&muxed, &temp).await?;
            let bytes = tokio::fs::read(&temp).await?;
            let mut offset = 0;
            while offset + 8 <= bytes.len() {
                let size =
                    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                if &bytes[offset + 4..offset + 8] == b"moof" {
                    break;
                }
                if size < 8 || offset + size > bytes.len() {
                    bail!("Invalid fragmented media output");
                }
                offset += size;
            }
            if offset == 0 || offset + 8 > bytes.len() {
                bail!("Missing media initialization");
            }
            tokio::fs::write(
                run.directory.join(format!("init-{index:06}.mp4")),
                &bytes[..offset],
            )
            .await?;
            tokio::fs::write(&temp, &bytes[offset..]).await?;
        }
        tokio::fs::rename(&temp, &path).await?;
        trim(&run.directory, &path).await?;
        Ok(Some(if init { run.directory.join(name) } else { path }))
    }
    pub async fn stop(&self, id: &str) {
        let run = self.runs.lock().await.remove(id);
        if let Some(run) = run {
            Self::dispose(run).await;
        }
    }
    async fn dispose(run: Arc<Run>) {
        run.canceled.send_replace(true);
        let mut gate = run.gate.lock().await;
        if let Some(mut work) = gate.take() {
            work.task.abort();
            let _ = (&mut work.task).await;
        }
        let _ = tokio::fs::remove_dir_all(&run.directory).await;
    }
    pub async fn runs(&self) -> Vec<RunHandle> {
        self.runs
            .lock()
            .await
            .iter()
            .map(|(id, run)| RunHandle {
                session: id.clone(),
                revision: run.revision.clone(),
            })
            .collect()
    }
    pub async fn encoding(&self, handle: &RunHandle) -> Option<Encoding> {
        self.runs
            .lock()
            .await
            .get(&handle.session)
            .filter(|run| run.revision == handle.revision)
            .map(|run| run.encoding)
    }
    pub async fn stop_revision(&self, handle: &RunHandle) -> bool {
        let run = {
            let mut runs = self.runs.lock().await;
            if runs
                .get(&handle.session)
                .is_some_and(|run| run.revision == handle.revision)
            {
                runs.remove(&handle.session)
            } else {
                None
            }
        };
        if let Some(run) = run {
            Self::dispose(run).await;
            true
        } else {
            false
        }
    }
    async fn remote_file(
        &self,
        run: &Run,
        index: usize,
        path: PathBuf,
        work: &mut Option<RemoteWork>,
    ) -> Result<PathBuf> {
        if work
            .as_ref()
            .is_some_and(|w| index < w.first || index >= w.end)
        {
            let mut old = work.take().unwrap();
            old.task.abort();
            let _ = (&mut old.task).await;
            publish_initializations(run, &old).await?;
        }
        if work.is_none() {
            let remote = run.remote.as_ref().unwrap();
            remote.validate()?;
            if fs2::available_space(&run.directory)? < 512 * 1024 * 1024 {
                bail!("Insufficient conversion cache space");
            }
            let slot = self
                .slots
                .clone()
                .try_acquire_owned()
                .context("All conversion slots are busy")?;
            // At most 48 seconds of media per connection. Publish each segment
            // atomically as soon as it is ready, while preparing the next ones.
            let end = (index + 8).min(run.boundaries.len() - 1);
            let start = run.boundaries[index];
            let duration = run.boundaries[end] - start;
            let mut command = Command::new(&run.tools.ffmpeg.path);
            command.env_clear();
            for name in ["PATH", "SystemRoot", "WINDIR"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
            for address in std::iter::once(&remote.video).chain(remote.audio.iter()) {
                command
                    .args(remote.network_args())
                    .args(["-rw_timeout", "15000000"]);
                if start > 0.0 {
                    command.args(["-ss", &start.to_string()]);
                }
                command.args(["-i", address]);
            }
            command.args([
                "-t",
                &duration.to_string(),
                "-map",
                "0:v:0",
                "-map",
                if remote.audio.is_some() {
                    "1:a:0"
                } else {
                    "0:a:0?"
                },
                "-sn",
                "-dn",
                "-map_metadata",
                "-1",
            ]);
            command.args(run.encoding.args(&run.options, &run.source.probe)?);
            let fragmented = run.options.capabilities.hls_container.as_deref() == Some("mp4");
            if fragmented {
                command.args([
                    "-hls_segment_type",
                    "fmp4",
                    "-hls_fmp4_init_filename",
                    "work-init.mp4",
                ]);
            }
            command
                .args([
                    "-force_key_frames",
                    "expr:gte(t,n_forced*6)",
                    "-avoid_negative_ts",
                    "make_zero",
                    "-f",
                    "hls",
                    "-hls_time",
                    "6",
                    "-hls_list_size",
                    "0",
                    "-start_number",
                    &index.to_string(),
                    "-hls_flags",
                    "independent_segments+temp_file",
                    "-hls_segment_filename",
                ])
                .arg(run.directory.join(if fragmented {
                    "segment-%06d.m4s"
                } else {
                    "segment-%06d.ts"
                }))
                .arg(run.directory.join("work.m3u8"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let child = command.spawn().context("Start public video conversion")?;
            let mut canceled = run.canceled.subscribe();
            let directory = run.directory.clone();
            let task = tokio::spawn(async move {
                // Another window replaces this one by aborting it, which leaves the muxer's
                // unfinished segment behind; its work files stay for the published segments.
                // Dropped last, after the child is killed.
                let _partials = Partials {
                    directory: directory.clone(),
                    work: false,
                };
                let _slot = slot;
                let mut child = child;
                let result = tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(60), child.wait()) =>
                        result.context("Public video conversion timed out").and_then(|r| r.map_err(Into::into)),
                    _ = canceled.changed() => Err(anyhow::anyhow!("Playback was stopped")),
                    result = monitor_work(&directory) => Err(result),
                };
                if result.is_err() {
                    let _ = child.kill().await;
                }
                anyhow::ensure!(result?.success(), "Unable to prepare public video segments");
                Ok(())
            });
            *work = Some(RemoteWork {
                first: index,
                end,
                task,
            });
        }
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if *run.canceled.borrow() {
                    bail!("Playback was stopped");
                }
                if tokio::fs::try_exists(&path).await? {
                    publish_initializations(run, work.as_ref().unwrap()).await?;
                    trim(&run.directory, &path).await?;
                    return Ok(path);
                }
                if work.as_ref().unwrap().task.is_finished() {
                    let mut finished = work.take().unwrap();
                    (&mut finished.task).await??;
                    publish_initializations(run, &finished).await?;
                    // A successful muxer must have published the requested segment.
                    if tokio::fs::try_exists(&path).await? {
                        return Ok(path);
                    }
                    bail!("Public video segment is unavailable");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .context("Public video segment timed out")?
    }
    pub async fn maintain(
        &self,
        candidates: &[RunHandle],
        active: &[String],
        budget: u64,
    ) -> Result<Vec<StoppedRun>> {
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
            let handle = RunHandle {
                session: id.clone(),
                revision: run.revision.clone(),
            };
            if !candidates.contains(&handle) {
                continue;
            }
            let size = match self.scanner.usage(&run.directory).await {
                Ok(size) => size,
                Err(error) => {
                    tracing::warn!(session = %id, revision = %run.revision, %error, "Stopping VOD with unreadable disposable cache");
                    stopped.push(StoppedRun {
                        handle,
                        reason: CleanupReason::CacheUnreadable,
                    });
                    continue;
                }
            };
            total += size;
            if !active.contains(&id) || low || total > budget || size > 288 * 1024 * 1024 {
                let reason = if !active.contains(&id) {
                    CleanupReason::Inactive
                } else {
                    CleanupReason::CacheLimit
                };
                stopped.push(StoppedRun { handle, reason });
            }
        }
        let mut removed = Vec::new();
        for stopped in stopped {
            if self.stop_revision(&stopped.handle).await {
                removed.push(stopped);
            }
        }
        Ok(removed)
    }
}
async fn publish_initializations(run: &Run, work: &RemoteWork) -> Result<()> {
    if run.options.capabilities.hls_container.as_deref() != Some("mp4") {
        return Ok(());
    }
    for index in work.first..work.end {
        let init = run.directory.join(format!("init-{index:06}.mp4"));
        if tokio::fs::try_exists(run.directory.join(format!("segment-{index:06}.m4s"))).await?
            && !tokio::fs::try_exists(&init).await?
        {
            tokio::fs::copy(run.directory.join("work-init.mp4"), init).await?;
        }
    }
    Ok(())
}
async fn trim(directory: &Path, current: &Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let mut files = Vec::new();
    let mut total = 0;
    while let Some(entry) = entries.next_entry().await? {
        if entry
            .path()
            .extension()
            .is_some_and(|e| e == "ts" || e == "m4s")
        {
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
        if path != current && tokio::fs::remove_file(&path).await.is_ok() {
            if let Some(index) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix("segment-"))
            {
                let _ = tokio::fs::remove_file(directory.join(format!("init-{index}.mp4"))).await;
            }
            total -= size;
            count -= 1;
        }
    }
    Ok(())
}
async fn keyframes(
    source: &Source,
    tools: &MediaTools,
    remote: Option<&RemoteSource>,
    budget: Duration,
) -> Result<Keyframes> {
    if let Some(remote) = remote.filter(|remote| !remote.segments.is_empty()) {
        let mut boundaries = remote
            .segments
            .iter()
            .copied()
            .filter(|time| *time >= 0.0 && *time < source.duration())
            .collect::<Vec<_>>();
        boundaries.push(source.duration());
        if boundaries.first() != Some(&0.0)
            || boundaries
                .windows(2)
                .any(|pair| !pair[0].is_finite() || pair[1] <= pair[0] || pair[1] - pair[0] > 30.0)
        {
            bail!("Cannot verify the provider's segment index");
        }
        let origin = source.probe["format"]["start_time"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        return Ok((boundaries, origin));
    }
    let mut command = Command::new(&tools.ffprobe.path);
    command.env_remove("FFREPORT");
    if let Some(remote) = remote {
        command
            .args(remote.network_args())
            .args(["-rw_timeout", "15000000"]);
    }
    // Packet flags mark keyframes without decoding anything.
    command
        .args([
            "-v",
            "error",
            "-fflags",
            "+genpts",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,dts_time,flags",
            "-of",
            "csv=p=0",
        ])
        .arg(remote.map_or_else(
            || source.path.to_string_lossy().into_owned(),
            |r| r.video.clone(),
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    let mut lines =
        tokio::io::BufReader::new(child.stdout.take().context("Missing keyframe output")?).lines();
    // Only keyframes are kept: one line per packet would be large for long, high-rate files.
    let mut frames = Vec::new();
    tokio::time::timeout(budget, async {
        while let Some(line) = lines.next_line().await? {
            let mut fields = line.split(',');
            let (pts, dts, flags) = (fields.next(), fields.next(), fields.next());
            if flags.is_some_and(|f| f.contains('K') && !f.contains('D'))
                && let Some(time) = [pts, dts]
                    .into_iter()
                    .flatten()
                    .find_map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite()))
            {
                frames.push(time);
            }
        }
        anyhow::Ok(())
    })
    .await
    .context("Keyframe indexing timed out")??;
    if !tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await??
        .success()
    {
        bail!("Keyframe indexing failed");
    }
    let video_start = *frames.first().context("Missing first video keyframe")?;
    let origin = source.probe["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut result = vec![0.0];
    let duration = source.duration();
    // The first keyframe may follow a short audio encoder delay. Its media
    // interval still begins at zero; do not create a tiny audio-only segment.
    let mut frames = frames.into_iter().skip(1).collect::<Vec<_>>();
    // Packets come in decoding order.
    frames.sort_by(f64::total_cmp);
    for time in frames {
        let time = time - origin;
        if time < duration - 0.05 && time - result.last().unwrap() > 0.001 {
            result.push(time);
        }
    }
    result.push(duration);
    if result.windows(2).any(|s| s[1] - s[0] > 30.0) {
        bail!(
            "Keyframes are too far apart for seekable remux; select a lower quality to convert video"
        );
    }
    Ok((result, video_start))
}

/// Removes a run's partial conversion output when the conversion ends, however it ends. A
/// seek drops the request preparing a segment mid-conversion, and leftovers would count
/// against the run's work and cache limits until it stops. Published output has been
/// renamed away by then.
struct Partials {
    directory: PathBuf,
    /// Also the remux work files, which only a request's own conversion uses.
    work: bool,
}
impl Drop for Partials {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".tmp")
                || (self.work && (name.starts_with("work-") || name == "work.csv"))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
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
    let last = start + duration >= run.source.duration() - 0.001;
    let origin = run.source.probe["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let expected = if start == 0.0 {
        run.video_start
    } else {
        start + origin
    };
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
            || !(file.ends_with(".ts") || file.ends_with(".mp4"))
            || !(file.len() == 11 || file.len() == 12)
            || !file[5..8].bytes().all(|b| b.is_ascii_digit())
        {
            bail!("Invalid segment name");
        }
        let path = run.directory.join(file);
        // The segment muxer reports zero for its first entry even when input
        // seeking starts later. Read that entry's first video packet instead.
        if index == 0 {
            begin = packet_start(&run.tools, &path).await?.0;
        }
        // The CSV end describes video; the final audio packet can end later.
        tracing::debug!(
            begin,
            end,
            expected,
            start,
            duration,
            "Checking original segment interval"
        );
        if (begin - expected).abs() < 0.01 && (last || end >= start + duration - 0.1) {
            return Ok(path);
        }
    }
    bail!("The requested keyframe interval could not be remuxed")
}
async fn packet_start(tools: &MediaTools, path: &Path) -> Result<(f64, f64)> {
    let mut command = Command::new(&tools.ffprobe.path);
    command
        .env_remove("FFREPORT")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+#1",
            "-show_entries",
            "packet=pts_time,dts_time",
            "-of",
            "json",
        ])
        .arg(path)
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
    let pts = value["packets"][0]["pts_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .context("Missing remux timestamp")?;
    let dts = value["packets"][0]["dts_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(pts);
    Ok((pts, dts))
}
