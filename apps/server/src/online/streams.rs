#[path = "../storage/online/streams.rs"]
mod storage;

#[cfg(test)]
#[path = "../../tests/product/online_quality.rs"]
mod compatibility_tests;
#[path = "stream_formats.rs"]
mod formats;
#[cfg(test)]
#[path = "../../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;

use super::{downloads, extract, live};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use thelxinoe_core::{Principal, now};
use thelxinoe_playback::{Decision, Options, RemoteSource, Source};
use tokio::sync::Mutex;

#[derive(Default)]
pub(crate) struct Runtime {
    prepared: Mutex<HashMap<String, Prepared>>,
    sessions: Mutex<HashMap<String, Prepared>>,
    pub(super) relay: super::relay::Relay,
}
#[derive(Clone)]
struct Prepared {
    source: RemoteSource,
    duration: f64,
    expires: i64,
    native: bool,
    formats: Vec<formats::Format>,
    remux: bool,
    quality: Option<String>,
    probe: Value,
    decision: Option<Decision>,
    options: Option<Options>,
}
impl Prepared {
    fn media(&self) -> Source {
        let mut probe = self.probe.clone();
        probe["format"]["duration"] = json!(self.duration.to_string());
        Source {
            id: String::new(),
            media_id: String::new(),
            generation: String::new(),
            edition: "public".into(),
            path: if self.source.live {
                "public.ts"
            } else {
                "public.mp4"
            }
            .into(),
            root: std::path::PathBuf::new(),
            size: 0,
            modified: String::new(),
            probe,
        }
    }
}
pub(crate) async fn negotiate_source(
    state: &AppState,
    video: &str,
    codecs: &[String],
    max_bitrate: Option<i64>,
    selected: Option<&Value>,
    negotiate: impl Fn(&Source) -> thelxinoe_playback::Capabilities,
) -> Result<(Source, thelxinoe_playback::Capabilities, Option<String>)> {
    let mut prepared = prepare(state, video).await?;
    let matches = |format: &&formats::Format| {
        selected.is_none_or(|selected| {
            format.quality() == selected["quality"]
                && format.video == selected["video"]
                && format.audio == selected["audio"]
        })
    };
    if selected.is_some() {
        let format = prepared
            .formats
            .iter()
            .rev()
            .find(matches)
            .ok_or_else(ApiError::not_found)?;
        if prepared.source.video != format.source.video
            || prepared.source.audio != format.source.audio
        {
            prepared.probe = inspect_codecs(state, &format.source).await?;
        }
        prepared.source = format.source.clone();
        prepared.quality = Some(format.quality());
    }
    let mut partial = None;
    for format in prepared
        .formats
        .iter()
        .rev()
        .filter(matches)
        .filter(|format| codecs.iter().any(|codec| codec == &format.video))
    {
        let mut candidate = prepared.clone();
        if candidate.source.video != format.source.video
            || candidate.source.audio != format.source.audio
        {
            candidate.probe = inspect_codecs(state, &format.source).await?;
        }
        candidate.source = format.source.clone();
        if max_bitrate.is_some_and(|max| {
            candidate.probe["format"]["bit_rate"]
                .as_str()
                .and_then(|v| v.parse::<i64>().ok())
                .is_none_or(|rate| rate <= 0 || rate > max)
        }) {
            continue;
        }
        let source = candidate.media();
        let capabilities = negotiate(&source);
        let options = Options {
            quality: "auto".into(),
            audio: None,
            subtitle: None,
            capabilities: capabilities.clone(),
        };
        if let Ok(decision) = thelxinoe_playback::decision(&source, &options) {
            if !decision.encoding.video && !decision.encoding.audio {
                return Ok((source, capabilities, Some(format.quality())));
            }
            if !decision.encoding.video && partial.is_none() {
                partial = Some((source, capabilities, Some(format.quality())));
            }
        }
    }
    if let Some(partial) = partial {
        return Ok(partial);
    }
    let source = prepared.media();
    let capabilities = negotiate(&source);
    Ok((source, capabilities, prepared.quality))
}
pub(crate) async fn variants(state: &AppState, p: &Principal, video: &str) -> Result<Vec<Value>> {
    if live::domain(video) {
        live::authorize(state, p, video).await?;
    } else {
        downloads::authorize(state, p, video).await?;
    }
    let prepared = prepare(state, video).await?;
    let mut variants = Vec::new();
    for format in prepared.formats.iter().rev() {
        let variant = format.descriptor();
        if !variants.iter().any(|v: &Value| {
            v["quality"] == variant["quality"]
                && v["video"] == variant["video"]
                && v["audio"] == variant["audio"]
        }) {
            variants.push(variant);
        }
    }
    Ok(variants)
}

async fn prepare(state: &AppState, video: &str) -> Result<Prepared> {
    {
        let mut cached = state.online.streams.prepared.lock().await;
        cached.retain(|_, p| p.expires > now());
        if let Some(value) = cached.get(video) {
            return Ok(value.clone());
        }
    }
    let prepared = if live::domain(video) {
        let mut selected = select(&live::extract(state, video).await?, None)?;
        selected.expires = now() + 60;
        selected.probe = inspect_codecs(state, &selected.source).await?;
        selected
    } else {
        extract::with_metadata(state, video, |metadata, family| async move {
            let proxy = state
                .online
                .network
                .egress
                .proxy(family)
                .await
                .map_err(|_| ApiError::conflict("Could not prepare the public video connection"))?;
            let mut selected = select(&metadata, Some(proxy))?;
            selected.probe = inspect_codecs(state, &selected.source).await?;
            Ok(selected)
        })
        .await?
        .0
    };
    let mut cached = state.online.streams.prepared.lock().await;
    if cached.len() >= 64 {
        cached.clear();
    }
    cached.insert(video.into(), prepared.clone());
    Ok(prepared)
}
async fn inspect_codecs(state: &AppState, source: &RemoteSource) -> Result<Value> {
    crate::tools::require(state, &["ffmpeg"]).await?;
    source
        .probe(&state.tools.runtime.media()?)
        .await
        .map_err(|error| {
            if error.is::<thelxinoe_playback::MediaAccessDenied>() {
                ApiError(
                    axum::http::StatusCode::CONFLICT,
                    "network_blocked",
                    "The media provider denied access from this server's network; try again later"
                        .into(),
                )
            } else {
                ApiError::conflict("Could not inspect public media codecs")
            }
        })
}
fn select(metadata: &Value, proxy: Option<String>) -> Result<Prepared> {
    let live = metadata["is_live"].as_bool().unwrap_or(false);
    let duration = metadata["duration"].as_f64().unwrap_or(0.0);
    if !live && (!duration.is_finite() || duration <= 0.0) {
        return Err(ApiError::conflict("This video is not ready for playback"));
    }
    let formats = formats::extract(metadata, proxy)?;
    let best = formats.last().unwrap();
    Ok(Prepared {
        source: best.source.clone(),
        native: best.native,
        duration: if live { 0.0 } else { duration },
        expires: now() + 300,
        formats,
        remux: false,
        quality: None,
        probe: json!({}),
        decision: None,
        options: None,
    })
}

pub(crate) async fn info(state: &AppState, p: &Principal, video: &str) -> Result<Value> {
    if live::domain(video) {
        live::authorize(state, p, video).await?;
    } else {
        downloads::authorize(state, p, video).await?;
    }
    let prepared = prepare(state, video).await?;
    let owner = p.user.id.clone();
    let id = video.to_owned();
    let position = storage::info(owner, id, &state.db).await?;
    Ok(
        json!({"sources":[{"id":video,"edition":"public","duration":prepared.duration,"video":true,"tracks":[],"probe":prepared.probe,"size":0}],"progress":[{"edition":"public","position":position,"duration":prepared.duration}],"watched":false,"live":prepared.source.live}),
    )
}

async fn select_for_player(
    state: &AppState,
    mut prepared: Prepared,
    options: &Options,
) -> Result<Prepared> {
    prepared.quality = None;
    if options.quality.ends_with("mbps") {
        let max = thelxinoe_playback::bitrate(&options.quality)?.unwrap();
        for format in prepared
            .formats
            .iter()
            .rev()
            .filter(|format| format.supported(&options.capabilities))
        {
            let probe = if prepared.source.video == format.source.video
                && prepared.source.audio == format.source.audio
            {
                prepared.probe.clone()
            } else {
                inspect_codecs(state, &format.source).await?
            };
            if probe["format"]["bit_rate"]
                .as_str()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|rate| rate > 0 && rate <= max)
            {
                prepared.source = format.source.clone();
                prepared.native = format.native;
                prepared.quality = Some(format.quality());
                prepared.probe = probe;
                break;
            }
        }
    }
    if !prepared.formats.is_empty() {
        let format = if options.quality.ends_with("mbps") && prepared.quality.is_some() {
            prepared
                .formats
                .iter()
                .find(|f| {
                    f.source.video == prepared.source.video
                        && f.source.audio == prepared.source.audio
                })
                .unwrap()
        } else {
            formats::choose(&prepared.formats, &options.quality, &options.capabilities)?
        };
        if prepared.source.video != format.source.video
            || prepared.source.audio != format.source.audio
        {
            prepared.probe = inspect_codecs(state, &format.source).await?;
        }
        prepared.source = format.source.clone();
        prepared.native = format.native;
        prepared.quality = Some(format.quality());
    }
    Ok(prepared)
}
async fn prepare_for_player(state: &AppState, video: &str, options: &Options) -> Result<Prepared> {
    let prepared = prepare(state, video).await?;
    match select_for_player(state, prepared, options).await {
        Err(error) if error.1 == "network_blocked" && !live::domain(video) => {
            // A cached signature or a different rendition can fail after metadata inspection.
            state.online.streams.prepared.lock().await.remove(video);
            Ok(
                extract::with_metadata(state, video, |metadata, family| async move {
                    let proxy = state
                        .online
                        .network
                        .egress
                        .proxy(family)
                        .await
                        .map_err(|_| {
                            ApiError::conflict("Could not prepare the public video connection")
                        })?;
                    select_for_player(state, select(&metadata, Some(proxy))?, options).await
                })
                .await?
                .0,
            )
        }
        result => result,
    }
}

pub(crate) async fn create_with_delivery(
    state: &AppState,
    p: &Principal,
    video: &str,
    options: Options,
    position: Option<f64>,
    vod: bool,
) -> Result<Value> {
    if live::domain(video) {
        live::authorize(state, p, video).await?;
    } else {
        downloads::authorize(state, p, video).await?;
    }
    if options.subtitle.as_deref().is_some_and(|s| s != "off") || options.audio.is_some() {
        return Err(ApiError::bad(
            "Track selection is not available for this public stream",
        ));
    }
    if position.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(ApiError::bad("Invalid stream position"));
    }
    let live_title = if live::domain(video) {
        Some(live::authorize(state, p, video).await?.1)
    } else {
        None
    };
    let mut prepared = prepare_for_player(state, video, &options).await?;
    let qualities = formats::choices(&prepared.formats, &options.capabilities);
    let mut input = options.clone();
    // Provider resolution choices select an existing rendition; only a bitrate
    // ceiling that the selected rendition exceeds needs local conversion.
    if input.quality.ends_with('p') || prepared.quality.as_deref() == Some(input.quality.as_str()) {
        input.quality = "auto".into();
    }
    if !(prepared.native && options.capabilities.native_remote && !vod) {
        input.capabilities.containers.clear();
    }
    let mut decision = thelxinoe_playback::decision(&prepared.media(), &input)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let policy = state.transcoding.read().await;
    policy.check(decision.encoding)?;
    let native = prepared.native
        && options.capabilities.native_remote
        && !vod
        && !decision.encoding.video
        && !decision.encoding.audio;
    if native {
        decision.mode = "direct";
    }
    prepared.remux = !decision.encoding.video && !decision.encoding.audio && !native;
    prepared.decision = Some(decision.clone());
    prepared.options = Some(input.clone());
    if !native {
        crate::tools::require(state, &["ffmpeg"]).await?;
    }
    let sid = thelxinoe_core::id();
    let _lifecycle = state.playback_lifecycle.write(&[&sid]).await;
    let mode = decision.mode;
    let key = sid.clone();
    let owner = p.user.id.clone();
    let auth = p.session_id.clone();
    let video = video.to_owned();
    let duration = prepared.duration;
    let live = prepared.source.live;
    let start = storage::create_with_delivery_write_twitch_streams(
        &state.db,
        storage::StreamSession {
            live_title,
            mode,
            key,
            owner,
            auth,
            video,
            input: options.clone(),
            duration,
            live,
            position,
        },
    )
    .await?;
    if !native && !decision.encoding.video && !live {
        let selected = formats::choose(&prepared.formats, &options.quality, &options.capabilities)?;
        if let Some(source) = segmented(state, &prepared.formats, selected, start, vod).await {
            if source.video == prepared.source.video && source.audio == prepared.source.audio {
                prepared.source = source;
            } else {
                match inspect_codecs(state, &source).await {
                    Ok(probe) => {
                        prepared.probe = probe;
                        prepared.source = source;
                        prepared.native = false;
                    }
                    // Keep the verified file rendition when its segmented alternative is denied.
                    Err(error) => {
                        tracing::debug!(code = error.1, "Keeping verified public stream input")
                    }
                }
            }
        }
    }
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .insert(sid.clone(), prepared.clone());
    let prepared_pipeline = if native {
        Ok((String::new(), 0.0))
    } else if vod && !live {
        let result = state
            .playback
            .start_remote_vod(&sid, &prepared.source, &prepared.media(), &input, &decision)
            .await;
        if result.as_ref().is_err_and(|error| {
            error
                .chain()
                .any(|cause| cause.to_string() == "Source keyframe index is unavailable")
        }) && !decision.encoding.video
            && input.quality != "original"
            && input.capabilities.allow_conversion != Some(false)
            && input.capabilities.conversion.is_some()
        {
            if let Err(error) = policy.check(thelxinoe_playback::Encoding {
                video: true,
                audio: decision.encoding.audio,
            }) {
                state.playback.stop(&sid).await;
                state.online.streams.sessions.lock().await.remove(&sid);
                storage::create_with_delivery_write_playback_sessions(sid.clone(), &state.db)
                    .await?;
                return Err(error);
            }
            decision.encoding.video = true;
            decision.mode = "transcode";
            decision.conversion_reason =
                Some("The source has no usable index for seekable original playback".into());
            prepared.quality = None;
            prepared.remux = false;
            prepared.decision = Some(decision.clone());
            state
                .online
                .streams
                .sessions
                .lock()
                .await
                .insert(sid.clone(), prepared.clone());
            storage::playback_mode(&state.db, sid.clone(), decision.mode.into()).await?;
            state
                .playback
                .start_remote_vod(&sid, &prepared.source, &prepared.media(), &input, &decision)
                .await
        } else {
            result
        }
    } else {
        state
            .playback
            .start_remote(&sid, &prepared.source, &input, &decision, start)
            .await
    };
    let (revision, offset) = match prepared_pipeline {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(reason = %error, "Public stream preparation failed");
            state.online.streams.sessions.lock().await.remove(&sid);
            let id = sid.clone();
            storage::create_with_delivery_write_playback_sessions(id, &state.db).await?;
            return Err(ApiError::conflict(
                "The server could not start the public stream; try again later",
            ));
        }
    };
    let key = sid.clone();
    let valid = storage::create_with_delivery_read_playback_sessions(key, &state.db).await?;
    if !valid {
        state.playback.stop(&sid).await;
        state.online.streams.sessions.lock().await.remove(&sid);
        return Err(ApiError::not_found());
    }
    let grant = grants::issue(state, p, &format!("playback:{sid}"), 120).await?;
    let url = if native {
        format!("/api/v1/playback/{sid}/remote/video?grant={grant}")
    } else {
        format!("/api/v1/playback/{sid}/hls/{revision}/index.m3u8?grant={grant}")
    };
    let external_audio = (native && prepared.source.audio.is_some())
        .then(|| format!("/api/v1/playback/{sid}/remote/audio?grant={grant}"));
    Ok(
        json!({"id":sid,"url":url,"external_audio":external_audio,"grant":grant,"mode":decision.mode,"encoding":decision.encoding,"conversion_reason":decision.conversion_reason,"position":start,"duration":prepared.duration,"timeline_start":offset,"video":true,"live":live,"tracks":[],"subtitles":[],"selected_subtitle":null,"options":options,"qualities":qualities,"quality":prepared.quality,"probe":prepared.probe,"replay_gain":"off"}),
    )
}
/// YouTube's segmented formats fill a browser's remux buffer much faster than
/// an open-ended file request. Each segment opens with a keyframe, so its
/// playlist provides seek points; without them, the file and its index remain.
async fn segmented(
    state: &AppState,
    formats: &[formats::Format],
    selected: &formats::Format,
    position: f64,
    index: bool,
) -> Option<RemoteSource> {
    let mut source = formats::segmented(formats, selected)?.source.clone();
    if position > 0.0 || index {
        source.segments = segments(state, &source).await?;
    }
    Some(source)
}
async fn segments(state: &AppState, source: &RemoteSource) -> Option<Vec<f64>> {
    source.validate().ok()?;
    let client = state
        .online
        .streams
        .relay
        .client(source.proxy.as_ref())
        .ok()?;
    let playlist = tokio::time::timeout(Duration::from_secs(5), async {
        let mut response = client.get(&source.video).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            // Signed segment addresses make long playlists several megabytes.
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return None;
            }
            bytes.extend(chunk);
        }
        String::from_utf8(bytes).ok()
    })
    .await
    .ok()??;
    formats::segment_starts(&playlist)
}
pub(crate) async fn validate(state: &AppState, id: &str) -> Result<()> {
    if !state.online.streams.sessions.lock().await.contains_key(id) {
        return Err(ApiError::conflict("Stream expired; start playback again"));
    }
    Ok(())
}
pub(super) async fn remote_source(state: &AppState, id: &str) -> Result<RemoteSource> {
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .get(id)
        .filter(|p| p.native)
        .map(|p| p.source.clone())
        .ok_or_else(ApiError::not_found)
}
pub(crate) async fn seek(
    state: &AppState,
    id: &str,
    options: &Options,
    position: f64,
    policy: &crate::transcoding::Policy,
) -> Result<(String, f64)> {
    let source = state
        .online
        .streams
        .sessions
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| ApiError::conflict("Stream expired; start playback again"))?;
    if source.source.live {
        return Err(ApiError::bad("Seeking is not available for live streams"));
    }
    let decision = source
        .decision
        .as_ref()
        .ok_or_else(|| ApiError::conflict("Stream expired"))?;
    policy.check(decision.encoding)?;
    let input = if source.remux && !source.formats.is_empty() {
        let selected = formats::choose(&source.formats, &options.quality, &options.capabilities)?;
        match segmented(state, &source.formats, selected, position, false).await {
            Some(segmented) if inspect_codecs(state, &segmented).await.is_ok() => segmented,
            None => selected.source.clone(),
            Some(_) => selected.source.clone(),
        }
    } else {
        source.source.clone()
    };
    state
        .playback
        .start_remote(
            id,
            &input,
            source.options.as_ref().unwrap_or(options),
            decision,
            position,
        )
        .await
        .map_err(|_| {
            ApiError::conflict("Could not seek the public stream; reopen it to refresh its address")
        })
}

#[cfg(all(test, unix))]
#[path = "../../tests/product/online_transcoding.rs"]
mod transcoding_tests;
pub(crate) async fn sessions(state: &AppState) -> Vec<String> {
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .keys()
        .cloned()
        .collect()
}
pub(crate) async fn maintain(state: &AppState, candidates: &[String], active: &[String]) {
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .retain(|id, _| !candidates.contains(id) || active.contains(id));
    state
        .online
        .streams
        .prepared
        .lock()
        .await
        .retain(|_, p| p.expires > now());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{call, fixture};
    use axum::http::StatusCode;
    fn metadata(live: bool) -> Value {
        json!({"is_live":live,"duration":100,"formats":[{"vcodec":"avc1.640028","acodec":"none","height":1080,"protocol":"https","ext":"mp4","url":"https://r1.googlevideo.com/video?signature=private"},{"vcodec":"none","acodec":"mp4a.40.2","abr":128,"protocol":"https","ext":"m4a","url":"https://r1.googlevideo.com/audio?signature=private"}]})
    }
    #[test]
    fn selection_handles_separate_streams_and_live_without_exposing_urls() {
        let vod = select(&metadata(false), None).unwrap();
        assert_eq!(vod.duration, 100.0);
        assert!(vod.source.audio.is_some());
        assert!(vod.native);
        assert!(!select(&metadata(true), None).unwrap().native);
        assert_eq!(select(&metadata(true), None).unwrap().duration, 0.0);
        let mut live = metadata(true);
        live["formats"][1]["acodec"] = Value::Null;
        live["formats"][1]["protocol"] = json!("m3u8_native");
        assert!(select(&live, None).unwrap().source.audio.is_some());
        let mut manifest = metadata(false);
        manifest["formats"][0]["protocol"] = json!("m3u8_native");
        assert!(!select(&manifest, None).unwrap().native);
        let mut audio_manifest = metadata(false);
        audio_manifest["formats"][1]["protocol"] = json!("m3u8_native");
        assert!(!select(&audio_manifest, None).unwrap().native);
        let mut malformed = metadata(false);
        malformed["formats"][0]["url"] = json!("https://localhost/private");
        assert!(select(&malformed, None).is_err());
    }
    #[tokio::test]
    async fn native_files_keep_sources_private_and_enforce_playback_grants() {
        let (_temp, state, cookie) = fixture().await;
        let video = "abcdefghijk";
        state
            .online
            .streams
            .prepared
            .lock()
            .await
            .insert(video.into(), select(&metadata(false), None).unwrap());
        let input = json!({"media_id":"youtube:abcdefghijk","position":20,"options":{"quality":"auto","audio":null,"subtitle":null,"capabilities":{"video":["h264"],"audio":["aac"],"containers":[],"hls":true,"native_remote":true}}});
        assert_eq!(
            call(&state, "/api/v1/playback", "POST", input.clone(), &cookie)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        state.db.write("test.fixture", move|db| { db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice',?1,'Public fixture')",[video])?; Ok(()) }).await.unwrap();
        let created = call(&state, "/api/v1/playback", "POST", input.clone(), &cookie).await;
        assert_eq!(created.0, StatusCode::OK);
        let data = created.2;
        assert_eq!(data["mode"], "direct");
        assert_eq!(data["position"].as_f64(), Some(20.0));
        assert_eq!(data["timeline_start"].as_f64(), Some(0.0));
        assert!(!data.to_string().contains("googlevideo"));
        assert!(!data.to_string().contains("signature"));
        assert!(
            data["url"]
                .as_str()
                .unwrap()
                .contains("/remote/video?grant=")
        );
        assert!(
            data["external_audio"]
                .as_str()
                .unwrap()
                .contains("/remote/audio?grant=")
        );
        let id = data["id"].as_str().unwrap();
        let grant = data["grant"].as_str().unwrap();
        // A valid session still cannot select an arbitrary upstream address.
        assert_eq!(
            call(
                &state,
                &format!("/api/v1/playback/{id}/remote/other?grant={grant}"),
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                &state,
                &format!("/api/v1/playback/{id}/remote/video?grant=invalid"),
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let other = call(&state, "/api/v1/playback", "POST", input, &cookie)
            .await
            .2;
        assert_eq!(
            call(
                &state,
                &format!(
                    "/api/v1/playback/{}/remote/video?grant={grant}",
                    other["id"].as_str().unwrap()
                ),
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let seek = call(
            &state,
            &format!("/api/v1/playback/{id}/seek"),
            "POST",
            json!({"position":60}),
            &cookie,
        )
        .await;
        assert_eq!(seek.0, StatusCode::OK);
        assert!(
            seek.2["url"]
                .as_str()
                .unwrap()
                .contains("/remote/video?grant=")
        );
        assert_eq!(
            call(
                &state,
                &format!("/api/v1/playback/{id}"),
                "DELETE",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &state,
                data["url"].as_str().unwrap(),
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        state
            .db
            .write("test.fixture", move |db| {
                db.execute(
                    "DELETE FROM youtube_videos WHERE user_id='alice' AND video_id=?1",
                    [video],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            call(
                &state,
                other["url"].as_str().unwrap(),
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn cached_public_stream_still_requires_user_visibility_and_live_does_not_mark_watched() {
        let (_temp, state, cookie) = fixture().await;
        let video = "abcdefghijk";
        state
            .online
            .streams
            .prepared
            .lock()
            .await
            .insert(video.into(), select(&metadata(true), None).unwrap());
        assert_eq!(
            call(
                &state,
                "/api/v1/catalog/youtube:abcdefghijk/playback",
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        state.db.write("test.fixture", move|db|{db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice',?1,'Live fixture')",[video])?;db.execute("INSERT INTO youtube_media VALUES (?1)",[video])?;db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id,streaming) SELECT 'live','alice',id,'live','public','playing','transcode','{}',0,?1,?1,?2,1 FROM sessions WHERE user_id='alice' LIMIT 1",params![now(),video])?;Ok(())}).await.unwrap();
        let info = call(
            &state,
            "/api/v1/catalog/youtube:abcdefghijk/playback",
            "GET",
            json!({}),
            &cookie,
        )
        .await;
        assert_eq!(info.0, StatusCode::OK);
        assert_eq!(info.2["live"], true);
        assert!(!info.2.to_string().contains("googlevideo"));
        assert!(!info.2.to_string().contains("signature"));
        let report = call(
            &state,
            "/api/v1/playback/live/progress",
            "POST",
            json!({"sequence":1,"position":300,"state":"playing"}),
            &cookie,
        )
        .await;
        assert_eq!(report.0, StatusCode::OK);
        let row = state
            .db
            .write("test.fixture", |db| {
                Ok(db.query_row(
                    "SELECT watched,position FROM youtube_state WHERE user_id='alice'",
                    [],
                    |r| Ok((r.get::<_, bool>(0)?, r.get::<_, f64>(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(row, (false, 0.0));
        let history = call(
            &state,
            "/api/v1/me/history?platform=youtube&range=all&user=bob",
            "GET",
            json!({}),
            &cookie,
        )
        .await;
        assert_eq!(history.0, StatusCode::OK);
        assert_eq!(history.2["items"].as_array().unwrap().len(), 1);
        assert_eq!(history.2["items"][0]["title"], "Live fixture");
        assert_eq!(
            call(
                &state,
                "/api/v1/admin/history?platform=youtube&range=all",
                "GET",
                json!({}),
                &cookie
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let token = crate::test_support::issue_session(
            &state.db,
            "bob".into(),
            "web".into(),
            "Bob browser".into(),
        )
        .await
        .unwrap();
        let bob = call(
            &state,
            "/api/v1/me/history?platform=youtube&range=all&user=alice",
            "GET",
            json!({}),
            &format!("thelxinoe_session={token}"),
        )
        .await;
        assert_eq!(bob.0, StatusCode::OK);
        assert_eq!(bob.2["items"].as_array().unwrap().len(), 0);
    }
}
