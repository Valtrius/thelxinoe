use super::{backend::Backend, ipc::Ipc};
use crate::tools::{LaunchTools, ToolManager, models::ToolId, process::ProcessJob};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    net::windows::named_pipe::ClientOptions,
    process::{Child, Command},
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
};

#[derive(Clone, Deserialize)]
pub struct Choice {
    pub id: String,
    pub title: String,
    #[serde(rename = "fileId")]
    pub file_id: Option<String>,
    pub position: Option<f64>,
    /// This choice's place in the queue it was taken from.
    #[serde(rename = "queueIndex")]
    pub queue_index: Option<usize>,
}
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub run: String,
    pub file_id: String,
    pub generation: String,
    pub media_id: String,
    pub mode: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub status: String,
    pub music: bool,
    pub index: usize,
    pub count: usize,
    pub error: Option<String>,
    pub video_ready: bool,
}
pub enum Control {
    Pause,
    Seek(f64),
    Quality { session: String, quality: String },
    Volume(f64),
    Next,
    Stop,
}
struct Run {
    id: String,
    music: bool,
    commands: mpsc::Sender<Control>,
    task: JoinHandle<()>,
    view: Arc<Mutex<View>>,
    app: AppHandle,
}
impl Run {
    async fn stop(self) {
        if self.view.lock().await.status == "starting" {
            self.task.abort();
            let _ = self.task.await;
            // An aborted start never reports its final state itself.
            let mut view = self.view.lock().await;
            view.status = "stopped".into();
            let _ = self.app.emit("mpv-state", view.clone());
            return;
        }
        let _ = self.commands.send(Control::Stop).await;
        let mut task = self.task;
        if tokio::time::timeout(Duration::from_secs(10), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }
}
/// Stops runs on their own task, so a canceled caller cannot leave one half stopped.
async fn stop_runs(runs: Vec<Run>) {
    if runs.is_empty() {
        return;
    }
    let _ = tokio::spawn(futures::future::join_all(runs.into_iter().map(Run::stop))).await;
}
fn ended(status: &str) -> bool {
    matches!(status, "stopped" | "ended" | "error" | "failed" | "")
}
/// Each video plays in its own MPV window, so several can play at once. One music
/// queue plays at a time, and never alongside videos.
pub struct Player {
    operation: Mutex<()>,
    runs: Mutex<Vec<Run>>,
    stops: watch::Sender<u64>,
}
impl Default for Player {
    fn default() -> Self {
        Self {
            operation: Mutex::default(),
            runs: Mutex::default(),
            stops: watch::channel(0).0,
        }
    }
}
impl Player {
    pub async fn restart_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operation.lock().await
    }
    /// The given run, or the most recently started one.
    pub async fn view(&self, run: Option<&str>) -> View {
        let runs = self.runs.lock().await;
        let found = match run {
            Some(id) => runs.iter().find(|r| r.id == id),
            None => runs.last(),
        };
        match found {
            Some(found) => found.view.lock().await.clone(),
            None => View {
                run: run.unwrap_or_default().into(),
                status: "stopped".into(),
                ..View::default()
            },
        }
    }
    /// Every run that has not ended, oldest first.
    pub async fn views(&self) -> Vec<View> {
        let runs = self.runs.lock().await;
        let mut views = Vec::with_capacity(runs.len());
        for run in runs.iter() {
            let view = run.view.lock().await.clone();
            if !ended(&view.status) {
                views.push(view);
            }
        }
        views
    }
    pub async fn active(&self) -> bool {
        !self.views().await.is_empty()
    }
    /// Stops one run, or without one every run and every start still resolving.
    pub async fn stop(&self, run: Option<&str>) {
        if run.is_none() {
            self.stops
                .send_modify(|revision| *revision = revision.wrapping_add(1));
        }
        let _operation = self.operation.lock().await;
        let stopping = {
            let mut runs = self.runs.lock().await;
            match run {
                Some(id) => runs
                    .iter()
                    .position(|r| r.id == id)
                    .map(|index| vec![runs.remove(index)])
                    .unwrap_or_default(),
                None => runs.drain(..).collect(),
            }
        };
        stop_runs(stopping).await;
    }
    pub async fn control(&self, run: Option<&str>, command: Control) -> Result<()> {
        let runs = self.runs.lock().await;
        let found = match run {
            Some(id) => runs.iter().find(|r| r.id == id),
            None => runs.last(),
        };
        found
            .context("MPV is not playing")?
            .commands
            .send(command)
            .await
            .map_err(|_| anyhow::anyhow!("MPV has stopped"))
    }
    /// Returns once MPV is launching; the run reports its progress as `mpv-state`.
    pub async fn play(
        &self,
        app: AppHandle,
        tools: Arc<ToolManager>,
        requested: Option<String>,
        choices: Vec<Choice>,
        music: bool,
    ) -> Result<View> {
        let mut stops = self.stops.subscribe();
        let revision = *stops.borrow_and_update();
        tokio::select! {
            biased;
            _ = stops.wait_for(|current| *current != revision) => Err(anyhow::anyhow!("Playback canceled")),
            result = self.play_inner(app, tools, requested, choices, music) => result,
        }
    }
    async fn play_inner(
        &self,
        app: AppHandle,
        tools: Arc<ToolManager>,
        requested: Option<String>,
        choices: Vec<Choice>,
        music: bool,
    ) -> Result<View> {
        ensure!(
            !choices.is_empty() && choices.len() <= 500,
            "Choose between one and 500 media items"
        );
        ensure!(
            choices
                .iter()
                .all(|c| c.id.len() <= 100 && c.title.len() <= 1000),
            "Invalid media queue"
        );
        let id = requested.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        ensure!(
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "Invalid playback run"
        );
        let _operation = self.operation.lock().await;
        let replaced = {
            let mut runs = self.runs.lock().await;
            ensure!(
                !runs.iter().any(|r| r.id == id),
                "This playback already started"
            );
            let (replaced, kept): (Vec<_>, Vec<_>) = runs
                .drain(..)
                .filter(|r| !r.task.is_finished())
                .partition(|r| music || r.music);
            *runs = kept;
            replaced
        };
        stop_runs(replaced).await;
        let (mpv_path, launch) = tools.resolve_launch(&[ToolId::Mpv], true).await?;
        let diagnostic =
            crate::tools::discovery::detect_named_executable("mpv", mpv_path.as_deref()).await;
        let executable = diagnostic
            .path
            .context("Install or select MPV in Windows player settings")?;
        let backend = Backend::new(&app)?;
        let (commands, rx) = mpsc::channel(32);
        let initial = View {
            run: id.clone(),
            media_id: choices[0].id.clone(),
            title: choices[0].title.clone(),
            status: "starting".into(),
            music,
            count: choices.len(),
            ..View::default()
        };
        let view = Arc::new(Mutex::new(initial.clone()));
        let state = view.clone();
        let controls = commands.clone();
        let task_app = app.clone();
        let mut runs = self.runs.lock().await;
        // Publish the run before any other await, so stopping can always reach it.
        let task = tokio::spawn(async move {
            if let Err(error) = run(
                &task_app,
                backend,
                launch,
                executable,
                choices,
                music,
                state.clone(),
                rx,
                controls,
            )
            .await
            {
                let mut state = state.lock().await;
                state.status = "failed".into();
                state.error = Some(error.to_string());
                let _ = task_app.emit("mpv-state", state.clone());
            }
        });
        runs.push(Run {
            id,
            music,
            commands,
            task,
            view,
            app,
        });
        Ok(initial)
    }
}
struct Prepared {
    playlist_id: i64,
    id: String,
    url: String,
    data: Value,
    position: f64,
    sequence: i64,
    finished: bool,
    activity: thelxinoe_core::activity::ActivityClock,
}
fn capabilities() -> Value {
    json!({"containers":["mp4","m4v","m4a","mkv","avi","mov","webm","ts","m2ts","mpg","mpeg","flac","mp3","ogg","opus","wav","wma","aac","aiff","alac"],"video":["h264","hevc","vp8","vp9","av1","mpeg4","mpeg2video","mpeg1video","wmv3","vc1","prores","mjpeg"],"audio":["aac","mp3","flac","vorbis","opus","ac3","eac3","dts","truehd","alac","pcm_s16le","pcm_s24le","pcm_s32le","pcm_f32le","wmav2"],"hls":true,"hls_video":["h264","hevc","av1"],"native_tracks":true,"native_remote":true})
}
async fn prepare(backend: &Backend, choice: &Choice, first: bool) -> Result<Prepared> {
    prepare_quality(backend, choice, first, None).await
}
async fn prepare_quality(
    backend: &Backend,
    choice: &Choice,
    first: bool,
    quality: Option<&str>,
) -> Result<Prepared> {
    let preferences = backend.call("/playback/preferences", "GET", None).await?;
    let quality = quality.unwrap_or(preferences["quality"].as_str().unwrap_or("auto"));
    let data=backend.call("/playback","POST",Some(json!({"media_id":choice.id,"file_id":choice.file_id,"position":if first {json!(choice.position)}else{json!(0)},"options":{"quality":quality,"audio":null,"subtitle":null,"capabilities":capabilities()}}))).await?;
    Ok(Prepared {
        playlist_id: -1,
        id: data["id"]
            .as_str()
            .context("Playback session is missing")?
            .into(),
        url: format!(
            "{}{}",
            backend.origin,
            data["url"].as_str().context("Playback URL is missing")?
        ),
        position: data["position"].as_f64().unwrap_or(0.0),
        data,
        sequence: 0,
        finished: false,
        activity: Default::default(),
    })
}
async fn load(ipc: &mut Ipc, entry: &mut Prepared, title: &str, mode: &str) -> Result<()> {
    let data = &entry.data;
    let mut options = json!({"start":(entry.position-data["timeline_start"].as_f64().unwrap_or(0.0)).max(0.0).to_string(),"force-media-title":title,"replaygain":data["replay_gain"].as_str().filter(|s|*s!="off").unwrap_or("no"),"replaygain-preamp":"0","replaygain-clip":"no"});
    if let Some(audio) = data["external_audio"].as_str() {
        // Use the same server origin as the video. CDN addresses and extractor
        // credentials stay on the server, including for separate audio tracks.
        ensure!(
            audio.starts_with("/api/v1/playback/") && !audio.contains(['\r', '\n']),
            "Invalid external audio path"
        );
        let origin = entry
            .url
            .strip_suffix(data["url"].as_str().unwrap_or_default())
            .context("Invalid playback URL")?;
        options["audio-file"] = json!(format!("{origin}{audio}"));
    }
    options["sub-delay"] = json!((-data["timeline_start"].as_f64().unwrap_or(0.0)).to_string());
    if data["mode"] == "direct" {
        if let Some(audio) = data["options"]["audio"].as_i64() {
            let tracks = data["tracks"]
                .as_array()
                .context("Media tracks are missing")?;
            if let Some(index) = tracks
                .iter()
                .filter(|t| t["kind"] == "audio")
                .position(|t| t["index"].as_i64() == Some(audio))
            {
                options["aid"] = json!((index + 1).to_string());
            }
        }
        if let Some(sub) = data["selected_subtitle"].as_str() {
            if sub == "off" {
                options["sid"] = json!("no");
            } else if let Some(index) = data["tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["kind"] == "subtitle" && t["index"].is_number())
                .position(|t| t["id"].as_str() == Some(sub))
            {
                options["sid"] = json!((index + 1).to_string());
            }
        }
    }
    ipc.call(json!(["loadfile", entry.url, mode, -1, options]))
        .await?;
    let playlist = ipc.call(json!(["get_property", "playlist"])).await?;
    entry.playlist_id = playlist
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["filename"].as_str() == Some(&entry.url))
        .and_then(|item| item["id"].as_i64())
        .context("MPV playlist entry is missing")?;
    Ok(())
}
async fn report(backend: &Backend, entry: &mut Prepared, state: &str) -> Result<()> {
    if entry.finished {
        return Ok(());
    }
    backend
        .call(
            &format!("/playback/{}/progress", entry.id),
            "POST",
            Some(json!({"sequence":entry.sequence,"position":entry.position,"state":state,"active_seconds":entry.activity.seconds()})),
        )
        .await?;
    entry.sequence += 1;
    if state == "stopped" {
        entry.finished = true;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run(
    app: &AppHandle,
    backend: Backend,
    launch: LaunchTools,
    executable: String,
    choices: Vec<Choice>,
    music: bool,
    view: Arc<Mutex<View>>,
    mut commands: mpsc::Receiver<Control>,
    controls: mpsc::Sender<Control>,
) -> Result<()> {
    let mut prepared = BTreeMap::new();
    let mut next = 0;
    for _ in 0..if music { choices.len().min(2) } else { 1 } {
        let entry = prepare(&backend, &choices[next], next == 0).await?;
        prepared.insert(next, entry);
        next += 1;
    }
    let pipe = format!(r"\\.\pipe\thelxinoe-{}", uuid::Uuid::new_v4());
    let segment_script = app.path().app_local_data_dir()?.join("segments.lua");
    install_script(&segment_script, include_str!("segments.lua"))?;
    let quality_script = app
        .path()
        .app_local_data_dir()?
        .join("thelxinoe-quality.lua");
    install_script(&quality_script, include_str!("quality.lua"))?;
    let mut command = Command::new(executable);
    launch.configure(&mut command)?;
    command
        .args(&launch.mpv_args)
        .arg(format!("--scripts-append={}", segment_script.display()))
        .arg(format!("--scripts-append={}", quality_script.display()))
        .args([
            "--config=yes",
            "--idle=yes",
            "--keep-open=no",
            "--save-position-on-quit=no",
            "--write-filename-in-watch-later-config=no",
            "--save-watch-history=no",
            "--ytdl=no",
            "--gapless-audio=yes",
            "--prefetch-playlist=yes",
            "--replaygain-clip=no",
            "--msg-level=all=no",
        ])
        .arg(format!("--input-ipc-server={pipe}"))
        .args(if music {
            vec!["--vid=no", "--audio-display=no", "--force-window=no"]
        } else {
            vec!["--force-window=immediate"]
        });
    let (mut child, _job) = spawn_player(&mut command)?;
    let client = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match ClientOptions::new().open(&pipe) {
                Ok(client) => return Ok::<_, anyhow::Error>(client),
                Err(_) => {
                    ensure!(
                        child.try_wait()?.is_none(),
                        "MPV exited before opening its control connection"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    })
    .await
    .context("MPV did not open its control connection")??;
    let (mut ipc, mut events) = Ipc::new(client);
    for (index, property) in ["time-pos", "pause", "video-out-params", "core-idle"]
        .iter()
        .enumerate()
    {
        ipc.call(json!(["observe_property", index, property]))
            .await?;
    }
    let mut current = 0;
    let mut paused = false;
    let mut idle = true;
    let mut loaded = false;
    let mut video_ready = false;
    let mut started = false;
    let work = async {
        for (index, entry) in &mut prepared {
            load(&mut ipc, entry, &choices[*index].title, "append").await?;
        }
        // Prepare both entries before audio starts, even for very short tracks.
        ipc.call(json!(["playlist-play-index", 0])).await?;
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        let mut last_report = Instant::now();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Control::Pause) => { ipc.call(json!(["cycle", "pause"])).await?; }
                    Some(Control::Seek(position)) => {
                        let entry = prepared.get_mut(&current).context("No active media")?;
                        if !loaded || entry.data["live"]==true {continue;}
                        entry.activity.set_active(false);
                        let position = position.min((entry.data["duration"].as_f64().unwrap_or(0.0) - 0.1).max(0.0));
                        if entry.data["mode"] == "direct" {
                            ipc.call(json!(["seek", position, "absolute+exact"])).await?;
                        } else {
                            // HLS windows are bounded. Ask the server to rebuild at
                            // the desired point rather than seeking a missing segment.
                            let result = backend.call(&format!("/playback/{}/seek", entry.id), "POST", Some(json!({"position":position}))).await?;
                            entry.url = format!("{}{}", backend.origin, result["url"].as_str().context("Seek URL is missing")?);
                            entry.data["timeline_start"] = result["timeline_start"].clone();
                            entry.position = position;
                            loaded = false;
                            load(&mut ipc, entry, &choices[current].title, "replace").await?;
                            ipc.call(json!(["set_property", "pause", paused])).await?;
                            // Replacing the MPV playlist also removed the prefetched
                            // entry; append it again without creating another session.
                            if let Some(entry) = prepared.get_mut(&(current + 1)) {
                                load(&mut ipc, entry, &choices[current + 1].title, "append-play").await?;
                            }
                        }
                    }
                    Some(Control::Quality { session, quality }) => {
                        let Some(entry) = prepared.get_mut(&current).filter(|entry| entry.id == session && quality_allowed(&entry.data, &quality)) else { continue; };
                        let resume = !paused;
                        ipc.call(json!(["set_property", "pause", true])).await?;
                        entry.activity.set_active(false);
                        let mut choice = choices[current].clone();
                        choice.position = Some(entry.position);
                        match prepare_quality(&backend, &choice, true, Some(&quality)).await {
                            Ok(mut replacement) => {
                                match load(&mut ipc, &mut replacement, &choice.title, "replace").await {
                                    Ok(()) => {
                                        report(&backend, entry, "stopped").await?;
                                        *entry = replacement;
                                        loaded = false;
                                        video_ready = false;
                                    }
                                    Err(_) => {
                                        let _ = backend.call(&format!("/playback/{}", replacement.id), "DELETE", None).await;
                                        load(&mut ipc, entry, &choice.title, "replace").await?;
                                        let _ = ipc.call(json!(["show-text", "Could not switch quality", 4000])).await;
                                    }
                                }
                            }
                            Err(_) => { let _ = ipc.call(json!(["show-text", "This quality is temporarily unavailable", 4000])).await; }
                        }
                        ipc.call(json!(["set_property", "pause", !resume])).await?;
                    }
                    Some(Control::Volume(volume)) => { ipc.call(json!(["set_property", "volume", volume])).await?; }
                    Some(Control::Next) if current + 1 < choices.len() => { ipc.call(json!(["playlist-next", "force"])).await?; }
                    Some(Control::Next | Control::Stop) | None => { break; }
                },
                message = events.recv() => {
                    let Some(message) = message else { anyhow::bail!("MPV control connection closed"); };
                    match message["event"].as_str() {
                        Some("start-file") => {
                            if let Some(entry) = prepared.get_mut(&current) { entry.activity.set_active(false); }
                            if let Some((index, _)) = prepared.iter().find(|(_, e)| Some(e.playlist_id) == message["playlist_entry_id"].as_i64()) {
                                let index = *index;
                                if index != current && let Some(previous) = prepared.get_mut(&current) {
                                    report(&backend, previous, "stopped").await?;
                                }
                                current = index;
                                loaded = false;
                                video_ready = false;
                                started = true;
                            }
                        }
                        Some("property-change") => match message["name"].as_str() {
                            Some("time-pos") if loaded => {
                                if let Some(position) = message["data"].as_f64()
                                    && let Some(entry) = prepared.get_mut(&current)
                                    && !entry.finished
                                { entry.position = (position + entry.data["timeline_start"].as_f64().unwrap_or(0.0)).max(0.0); }
                            }
                            Some("pause" | "core-idle") => {
                                if message["name"]=="pause" { paused=message["data"].as_bool().unwrap_or(true); }
                                else { idle=message["data"].as_bool().unwrap_or(true); }
                                if let Some(entry)=prepared.get_mut(&current)
                                    && !entry.finished && entry.activity.set_active(loaded && !paused && !idle) {
                                    report(&backend,entry,if paused || idle {"paused"}else{"playing"}).await?;
                                }
                            }
                            Some("video-out-params") => { video_ready = message["data"]["w"].as_u64().unwrap_or(0) > 0; }
                            _ => {}
                        },
                        Some("client-message") if message["args"][0] == "thelxinoe-seek" => {
                            if let Some(entry) = prepared.get(&current)
                                && message["args"][1] == entry.data["file_id"]
                                && message["args"][2] == entry.data["generation"]
                                && let Some(at) = message["args"][3].as_str().and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0)
                            { let _ = controls.try_send(Control::Seek(at)); }
                        }
                        Some("client-message") if message["args"][0] == "thelxinoe-quality" => {
                            if let (Some(session), Some(quality)) = (message["args"][1].as_str(), message["args"][2].as_str()) {
                                let _ = controls.try_send(Control::Quality { session: session.into(), quality: quality.into() });
                            }
                        }
                        Some("file-loaded") => {
                            loaded = true;
                            if let Some(entry) = prepared.get(&current) {
                                let mut qualities = entry.data["qualities"].as_array().cloned().unwrap_or_default();
                                if !qualities.is_empty() { qualities.insert(0, json!({"value":"auto","label":"Auto"})); }
                                let payload = json!({"session":entry.id,"qualities":qualities,"selected":entry.data["options"]["quality"]});
                                let _ = ipc.call(json!(["script-message", "thelxinoe-qualities", payload.to_string()])).await;
                            }
                            if let Some(entry) = prepared.get_mut(&current) {
                                entry.activity.set_active(!paused && !idle);
                                report(&backend, entry, if paused || idle { "paused" } else { "playing" }).await?;
                                if let Some(sub) = entry.data["selected_subtitle"].as_str().filter(|s| s.starts_with("sidecar-") || (entry.data["mode"] != "direct" && *s != "off"))
                                    && let Some(url) = entry.data["subtitles"].as_array().into_iter().flatten().find(|s| s["id"].as_str() == Some(sub)).and_then(|s| s["url"].as_str())
                                { ipc.call(json!(["sub-add", format!("{}{url}", backend.origin), "select"])).await?; }
                            }
                            if !music && let Some(entry) = prepared.get(&current)
                                && let Some(file) = entry.data["file_id"].as_str()
                                && let Ok(mut segments) = backend.call(&format!("/catalog/{}/segments?file_id={}", choices[current].id, file), "GET", None).await
                                && segments["generation"] == entry.data["generation"]
                            {
                                segments["file"] = json!(file);
                                segments["timeline"] = entry.data["timeline_start"].clone();
                                let _ = ipc.call(json!(["script-message", "thelxinoe-segments", segments.to_string()])).await;
                            }
                            if music && next < choices.len() && next <= current + 1 {
                                let mut entry = prepare(&backend, &choices[next], false).await?;
                                load(&mut ipc, &mut entry, &choices[next].title, "append-play").await?;
                                prepared.insert(next, entry);
                                next += 1;
                            }
                        }
                        Some("end-file") => {
                            let index = prepared.iter().find(|(_, e)| Some(e.playlist_id) == message["playlist_entry_id"].as_i64()).map(|(i, _)| *i);
                            if let Some(index) = index {
                                let entry = prepared.get_mut(&index).unwrap();
                                entry.activity.set_active(false);
                                // MPV emits stop/redirect while a script reloads the
                                // same playlist entry. Keep its server session alive.
                                if reload_event(&message) {
                                    if index == current { loaded = false; }
                                    continue;
                                }
                                if message["reason"] == "error" { anyhow::bail!("MPV could not decode or retrieve this media"); }
                                if message["reason"] == "eof" && entry.data["live"]!=true { entry.position = entry.data["duration"].as_f64().unwrap_or(entry.position); }
                                report(&backend, entry, "stopped").await?;
                                if index == current { loaded = false; }
                                if index + 1 >= choices.len() { break; }
                            }
                        }
                        Some("shutdown") => { break; }
                        _ => {}
                    }
                },
                _ = tick.tick() => {
                    if child.try_wait()?.is_some() { break; }
                    for (index,entry) in &mut prepared {
                        entry.activity.set_active(*index==current && loaded && !paused && !idle && !entry.finished);
                    }
                    // MPV can keep the same output format across a reload, so its
                    // property observer need not emit another dimensions change.
                    if loaded && !music && !video_ready {
                        video_ready = ipc.call(json!(["get_property", "video-out-params"])).await.ok()
                            .and_then(|v| v["w"].as_u64()).unwrap_or(0) > 0;
                    }
                    if last_report.elapsed() >= Duration::from_secs(5) {
                        for (index, entry) in &mut prepared {
                            if entry.finished { continue; }
                            if *index == current && loaded { report(&backend, entry, if paused || idle { "paused" } else { "playing" }).await?; }
                            else { backend.call(&format!("/playback/{}/keepalive", entry.id), "POST", None).await?; }
                        }
                        last_report = Instant::now();
                    }
                    if let Some(entry) = prepared.get(&current) {
                        let mut state = view.lock().await;
                        *state = View { run: state.run.clone(), file_id: entry.data["file_id"].as_str().unwrap_or_default().into(), generation: entry.data["generation"].as_str().unwrap_or_default().into(), media_id: choices[current].id.clone(), mode: entry.data["mode"].as_str().unwrap_or_default().into(), title: choices[current].title.clone(), position: entry.position, duration: entry.data["duration"].as_f64().unwrap_or(0.0), paused, status: if !loaded { "starting" } else if paused { "paused" } else { "playing" }.into(), music, index: current, count: choices.len(), error: None, video_ready: loaded && (video_ready || entry.data["video"] == false) };
                        let _ = app.emit("mpv-state", state.clone());
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    for entry in prepared.values_mut() {
        entry.activity.set_active(false);
    }
    // Preserve the last observed position before MPV unload resets its properties.
    // A normal quit flushes output devices (including PCM) before process teardown.
    let _ = tokio::time::timeout(Duration::from_secs(1), ipc.call(json!(["quit"]))).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
    let _ = child.kill().await;
    for (index, entry) in &mut prepared {
        if entry.finished {
            continue;
        }
        if *index == current && started {
            let _ = report(&backend, entry, "stopped").await;
        } else {
            let _ = backend
                .call(&format!("/playback/{}", entry.id), "DELETE", None)
                .await;
        }
    }
    let mut state = view.lock().await;
    if let Some(entry) = prepared.get(&current) {
        state.media_id = choices[current].id.clone();
        state.title = choices[current].title.clone();
        state.index = current;
        state.position = entry.position;
        state.duration = entry.data["duration"].as_f64().unwrap_or(0.0);
    }
    state.paused = true;
    // A failed run reports only its failure, from the task that owns it.
    if work.is_ok() {
        state.status = "stopped".into();
        let _ = app.emit("mpv-state", state.clone());
    }
    drop(state);
    drop(launch); // Keep versions and the plugin configuration snapshot leased until MPV exits.
    work
}

/// MPV runs in a kill-on-close job. Windows closes the job with the desktop process,
/// so a crashed or ended desktop cannot leave MPV playing with no window to stop it.
fn spawn_player(command: &mut Command) -> Result<(Child, ProcessJob)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    ProcessJob::spawn(command).context("Launch MPV")
}

/// Every MPV window loads the same script files. Rewrite one only when it changed,
/// so a window that is starting never reads a script while it is being replaced.
fn install_script(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == contents.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, contents)
}
fn quality_allowed(data: &Value, quality: &str) -> bool {
    let choices = data["qualities"].as_array();
    choices.is_some_and(|choices| {
        !choices.is_empty()
            && (quality == "auto" || choices.iter().any(|choice| choice["value"] == quality))
    })
}
fn reload_event(message: &Value) -> bool {
    matches!(message["reason"].as_str(), Some("stop" | "redirect"))
}

#[cfg(test)]
#[path = "quality_smoke.rs"]
mod quality_smoke;

#[cfg(test)]
mod capability_tests {
    use super::*;
    use thelxinoe_playback::{Capabilities, Options, Source, decision};
    #[test]
    fn original_quality_copies_live_hevc_and_av1() {
        let mut capabilities: Capabilities = serde_json::from_value(capabilities()).unwrap();
        // The server always delivers live streams as HLS.
        capabilities.containers.clear();
        let options = Options {
            quality: "original".into(),
            audio: None,
            subtitle: None,
            capabilities,
        };
        for video in ["h264", "hevc", "av1"] {
            let source = Source {
                id: String::new(),
                media_id: String::new(),
                generation: String::new(),
                edition: "public".into(),
                path: "public.ts".into(),
                root: Default::default(),
                size: 0,
                modified: String::new(),
                probe: json!({"format":{},"streams":[
                    {"index":0,"codec_type":"video","codec_name":video,"height":1920},
                    {"index":1,"codec_type":"audio","codec_name":"aac"}]}),
            };
            assert_eq!(decision(&source, &options).unwrap().mode, "remux", "{video}");
        }
    }
}

#[cfg(test)]
mod restart_tests {
    use super::*;
    #[tokio::test]
    async fn installation_waits_for_playback_preparation_and_fences_new_starts() {
        let player = Player::default();
        // play() owns this lock even before a Run has been published.
        let preparing = player.operation.lock().await;
        assert_eq!(player.view(None).await.status, "stopped");
        let mut install = std::pin::pin!(player.restart_guard());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut install)
                .await
                .is_err()
        );
        drop(preparing);
        let installing = install.await;
        assert!(player.operation.try_lock().is_err());
        drop(installing);
        assert!(player.operation.try_lock().is_ok());
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;
    // Windows closes every handle of a process that crashes or is ended, including the job.
    #[tokio::test]
    async fn player_ends_when_the_desktop_closes_its_job() {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "ping -n 30 127.0.0.1 > nul"]);
        let (mut child, job) = spawn_player(&mut command).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("closing the job must end the player")
            .unwrap();
    }
}
