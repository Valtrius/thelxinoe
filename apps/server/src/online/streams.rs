#[path = "../storage/online/streams.rs"]
mod storage;

#[cfg(test)]
#[path = "../../tests/product/online_quality.rs"]
mod compatibility_tests;
#[path = "stream_formats.rs"]
mod formats;
#[path = "stream_index.rs"]
mod index;
#[cfg(test)]
use crate::test_support::tool_runtime;

use super::{downloads, extract, live, network::Family};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::HashMap, fmt::Write, sync::Arc, time::Duration};
use thelxinoe_core::{Principal, now};
use thelxinoe_playback::{Options, RemoteSource, Source};
use tokio::sync::Mutex;

/// The revision of byte-range HLS playlists, which the server writes on request.
const INDEXED: &str = "source";

/// A rendition's video and audio addresses.
type Rendition = (String, Option<String>);

#[derive(Default)]
pub(crate) struct Runtime {
    prepared: Mutex<HashMap<String, Prepared>>,
    /// Concurrent requests for one video share its extraction.
    pending: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Fragment indexes by signed address, which changes with each extraction.
    indexes: Mutex<HashMap<String, (i64, Arc<index::Index>)>>,
    /// Codec probes of renditions by their video and audio addresses.
    probes: Mutex<HashMap<Rendition, (i64, Value)>>,
    sessions: Mutex<HashMap<String, Session>>,
    pub(super) relay: super::relay::Relay,
}
#[derive(Clone)]
struct Prepared {
    source: RemoteSource,
    duration: f64,
    expires: i64,
    formats: Vec<formats::Format>,
    quality: Option<String>,
    probe: Value,
}
impl Prepared {
    fn media(&self) -> Source {
        media(&self.probe, self.duration, self.source.live)
    }
    fn rendition(&self) -> &formats::Format {
        self.formats
            .iter()
            .find(|f| f.source.video == self.source.video && f.source.audio == self.source.audio)
            .expect("a prepared source is one of its renditions")
    }
}
fn media(probe: &Value, duration: f64, live: bool) -> Source {
    let mut probe = probe.clone();
    probe["format"]["duration"] = json!(duration.to_string());
    Source {
        id: String::new(),
        media_id: String::new(),
        generation: String::new(),
        edition: "public".into(),
        path: if live { "public.ts" } else { "public.mp4" }.into(),
        root: std::path::PathBuf::new(),
        size: 0,
        modified: String::new(),
        probe,
    }
}
#[derive(Clone)]
struct Session {
    source: RemoteSource,
    /// The provider refuses its media before this time.
    available_at: i64,
    delivery: Delivery,
    /// The client's playback address, without its grant.
    path: String,
}
#[derive(Clone)]
enum Delivery {
    /// The client reads the original files through the relay.
    Relay,
    /// Byte-range HLS of the original files, read through the relay.
    Indexed(Arc<Manifest>),
    /// FFmpeg output: live streams, and recorded videos for clients without
    /// fragmented MP4 HLS.
    Pipeline,
}
struct Manifest {
    video: Arc<index::Index>,
    audio: Arc<index::Index>,
    format: formats::Format,
}
impl Manifest {
    fn master(&self, grant: &str) -> String {
        let (video_peak, video_average) = self.video.bandwidth();
        let (audio_peak, audio_average) = self.audio.bandwidth();
        let format = &self.format;
        let mut variant = format!(
            "BANDWIDTH={},AVERAGE-BANDWIDTH={},CODECS=\"{},{}\"",
            video_peak + audio_peak,
            video_average + audio_average,
            format.codecs[0],
            format.codecs[1]
        );
        if let Some(width) = format.width {
            let _ = write!(variant, ",RESOLUTION={width}x{}", format.height);
        }
        if let Some(rate) = format.frame_rate {
            let _ = write!(variant, ",FRAME-RATE={rate:.3}");
        }
        format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Original\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8?grant={grant}\"\n#EXT-X-STREAM-INF:{variant},AUDIO=\"audio\"\nvideo.m3u8?grant={grant}\n"
        )
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Relay,
    Indexed,
    Pipeline,
}

/// The rendition a client plays: the largest picture, up to the selected one, that it takes
/// without converting video, preferring the selected rendition at that picture. A client
/// that takes none converts the selected or best rendition.
pub(crate) async fn negotiate_source(
    state: &AppState,
    video: &str,
    codecs: &[String],
    max_bitrate: Option<i64>,
    selected: Option<&Value>,
    negotiate: impl Fn(&Source) -> thelxinoe_playback::Capabilities,
) -> Result<(Source, thelxinoe_playback::Capabilities, Option<String>)> {
    let prepared = prepare(state, video).await?;
    let mut base = prepared.clone();
    let playable = |format: &&formats::Format| format.source.live || format.indexed;
    let selected = selected
        .map(|selected| {
            prepared
                .formats
                .iter()
                .rev()
                .filter(playable)
                .find(|format| {
                    format.quality() == selected["quality"]
                        && format.video == selected["video"]
                        && format.audio == selected["audio"]
                })
                .ok_or_else(ApiError::not_found)
        })
        .transpose()?;
    if let Some(format) = selected {
        base.probe = describe(state, &prepared, format).await?;
        base.source = format.source.clone();
        base.quality = Some(format.quality());
    }
    let picture = |format: &formats::Format| (format.height, format.fps);
    let largest = selected.map(picture);
    let others = prepared
        .formats
        .iter()
        .rev()
        .filter(playable)
        .filter(|format| {
            largest.is_none_or(|largest| picture(format) <= largest)
                && selected.is_none_or(|selected| !std::ptr::eq(*format, selected))
        });
    let candidates = selected.into_iter().chain(others);
    let mut partial: Option<(Source, thelxinoe_playback::Capabilities, &formats::Format)> = None;
    for format in candidates.filter(|format| codecs.iter().any(|codec| codec == &format.video)) {
        // Converting only audio keeps the picture, which a smaller rendition would not.
        if partial
            .as_ref()
            .is_some_and(|(.., found)| picture(found) != picture(format))
        {
            break;
        }
        let mut candidate = base.clone();
        candidate.probe = describe(state, &prepared, format).await?;
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
                partial = Some((source, capabilities, format));
            }
        }
    }
    if let Some((source, capabilities, format)) = partial {
        return Ok((source, capabilities, Some(format.quality())));
    }
    let source = base.media();
    let capabilities = negotiate(&source);
    Ok((source, capabilities, base.quality))
}
/// Recorded renditions are described by the provider's metadata; live
/// playlists are inspected.
async fn describe(
    state: &AppState,
    prepared: &Prepared,
    format: &formats::Format,
) -> Result<Value> {
    if !format.source.live {
        Ok(format.probe(prepared.duration))
    } else if prepared.source.video == format.source.video
        && prepared.source.audio == format.source.audio
    {
        Ok(prepared.probe.clone())
    } else {
        inspect_codecs(state, format).await
    }
}
pub(crate) async fn variants(state: &AppState, p: &Principal, video: &str) -> Result<Vec<Value>> {
    if live::domain(video) {
        live::authorize(state, p, video).await?;
    } else {
        downloads::authorize(state, p, video).await?;
    }
    let prepared = prepare(state, video).await?;
    let mut variants = Vec::new();
    // Recorded videos offer the renditions every delivery can play.
    for format in prepared
        .formats
        .iter()
        .rev()
        .filter(|f| f.source.live || f.indexed)
    {
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

async fn cached(state: &AppState, video: &str) -> Option<Prepared> {
    let mut cached = state.online.streams.prepared.lock().await;
    cached.retain(|_, p| p.expires > now());
    cached.get(video).cloned()
}
async fn remember(state: &AppState, video: &str, prepared: &Prepared) {
    let mut cached = state.online.streams.prepared.lock().await;
    if cached.len() >= 64 {
        cached.clear();
    }
    cached.insert(video.into(), prepared.clone());
}
async fn prepare(state: &AppState, video: &str) -> Result<Prepared> {
    if let Some(prepared) = cached(state, video).await {
        return Ok(prepared);
    }
    let turn = state
        .online
        .streams
        .pending
        .lock()
        .unwrap()
        .entry(video.into())
        .or_default()
        .clone();
    let result = async {
        let _turn = turn.lock().await;
        if let Some(prepared) = cached(state, video).await {
            return Ok(prepared);
        }
        let prepared = if live::domain(video) {
            let mut selected = select(&live::extract(state, video).await?, None, &[])?;
            selected.probe = inspect_codecs(state, selected.rendition()).await?;
            selected
        } else {
            let excluded = &super::excluded_codecs(state).await?;
            extract::with_metadata(state, video, |metadata, family| async move {
                let mut selected = select(&metadata, Some(proxy(state, family).await?), excluded)?;
                if selected.source.live {
                    selected.probe = inspect_codecs(state, selected.rendition()).await?;
                }
                Ok(selected)
            })
            .await?
            .0
        };
        remember(state, video, &prepared).await;
        Ok(prepared)
    }
    .await;
    let mut pending = state.online.streams.pending.lock().unwrap();
    if Arc::strong_count(&turn) <= 2 {
        pending.remove(video);
    }
    result
}
/// Drops prepared renditions, so the next playback applies changed settings.
pub(crate) async fn forget(state: &AppState) {
    state.online.streams.prepared.lock().await.clear();
}
async fn proxy(state: &AppState, family: Family) -> Result<String> {
    state
        .online
        .network
        .egress
        .proxy(family)
        .await
        .map_err(|_| ApiError::conflict("Could not prepare the public video connection"))
}
fn denied() -> ApiError {
    ApiError(
        axum::http::StatusCode::CONFLICT,
        "network_blocked",
        "The media provider denied access from this server's network; try again later".into(),
    )
}
/// A rendition's codecs, probed once while its address lasts: choosing a live rendition
/// compares several, and playback start considers them again.
async fn inspect_codecs(state: &AppState, format: &formats::Format) -> Result<Value> {
    let source = &format.source;
    let key = (source.video.clone(), source.audio.clone());
    {
        let mut probes = state.online.streams.probes.lock().await;
        probes.retain(|_, (expires, _)| *expires > now());
        if let Some((_, probe)) = probes.get(&key) {
            return Ok(probe.clone());
        }
    }
    crate::tools::require(state, &["ffmpeg"]).await?;
    let mut probe = source
        .probe(&state.tools.runtime.media()?)
        .await
        .map_err(|error| {
            if error.is::<thelxinoe_playback::MediaAccessDenied>() {
                denied()
            } else {
                ApiError::conflict("Could not inspect public media codecs")
            }
        })?;
    // FFprobe cannot measure a live playlist's bitrate; without the provider's declared
    // bandwidth, every client bitrate limit would convert the stream.
    if probe["format"]["bit_rate"].is_null()
        && let Some(rate) = format.bitrate
    {
        probe["format"]["bit_rate"] = json!(rate.to_string());
    }
    let mut probes = state.online.streams.probes.lock().await;
    if probes.len() >= 128 {
        probes.clear();
    }
    let expires = expiry(&source.video).map_or(now() + 300, |at| at - 600);
    probes.insert(key, (expires, probe.clone()));
    Ok(probe)
}
/// When a signed address expires, from its `expire` parameter.
fn expiry(address: &str) -> Option<i64> {
    reqwest::Url::parse(address)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "expire")?
        .1
        .parse()
        .ok()
}
fn select(metadata: &Value, proxy: Option<String>, excluded: &[&str]) -> Result<Prepared> {
    let live = metadata["is_live"].as_bool().unwrap_or(false);
    let duration = metadata["duration"].as_f64().unwrap_or(0.0);
    if !live && (!duration.is_finite() || duration <= 0.0) {
        return Err(ApiError::conflict("This video is not ready for playback"));
    }
    let formats = formats::extract(metadata, proxy, excluded)?;
    let best = formats
        .iter()
        .rev()
        .find(|f| f.indexed)
        .unwrap_or(formats.last().unwrap());
    let duration = if live { 0.0 } else { duration };
    // Signed addresses last hours; reuse them until shortly before they expire.
    let lifetime = if live {
        60
    } else {
        expiry(&best.source.video).map_or(300, |at| (at - now() - 600).clamp(60, 3600))
    };
    Ok(Prepared {
        source: best.source.clone(),
        duration,
        expires: now() + lifetime,
        probe: if live {
            json!({})
        } else {
            best.probe(duration)
        },
        formats,
        quality: None,
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
                inspect_codecs(state, format).await?
            };
            if probe["format"]["bit_rate"]
                .as_str()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|rate| rate > 0 && rate <= max)
            {
                prepared.source = format.source.clone();
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
            prepared.probe = inspect_codecs(state, format).await?;
        }
        prepared.source = format.source.clone();
        prepared.quality = Some(format.quality());
    }
    Ok(prepared)
}

pub(crate) async fn create_with_delivery(
    state: &AppState,
    p: &Principal,
    video: &str,
    options: Options,
    position: Option<f64>,
    vod: bool,
) -> Result<Value> {
    let live_title = if live::domain(video) {
        Some(live::authorize(state, p, video).await?.1)
    } else {
        downloads::authorize(state, p, video).await?;
        None
    };
    if options.subtitle.as_deref().is_some_and(|s| s != "off") || options.audio.is_some() {
        return Err(ApiError::bad(
            "Track selection is not available for this public stream",
        ));
    }
    if position.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(ApiError::bad("Invalid stream position"));
    }
    let prepared = prepare(state, video).await?;
    if prepared.source.live {
        create_live(state, p, video, live_title, prepared, options).await
    } else {
        create_recorded(state, p, video, prepared, options, position, vod).await
    }
}
/// Original renditions for a client, and how it receives the chosen one:
/// separate files, byte-range HLS or, for players with neither, FFmpeg.
fn plan(
    prepared: &Prepared,
    options: &Options,
    vod: bool,
) -> Result<(Kind, formats::Format, Vec<Value>)> {
    let caps = &options.capabilities;
    let relay: Vec<_> = prepared
        .formats
        .iter()
        .filter(|f| caps.native_remote && !vod && f.native && f.supported(caps))
        .collect();
    let indexed: Vec<_> = prepared
        .formats
        .iter()
        .filter(|f| f.indexed_for(caps))
        .collect();
    let (kind, eligible) = if !relay.is_empty() {
        (Kind::Relay, relay)
    } else if !indexed.is_empty() {
        (Kind::Indexed, indexed)
    } else if vod {
        let files: Vec<_> = prepared
            .formats
            .iter()
            .filter(|f| f.indexed)
            .cloned()
            .collect();
        let format = formats::choose(&files, &options.quality, caps)?.clone();
        let supported: Vec<_> = files.iter().filter(|f| f.supported(caps)).collect();
        return Ok((Kind::Pipeline, format, formats::choices(supported)));
    } else {
        return Err(ApiError::conflict(
            "This player cannot play any original format of this video",
        ));
    };
    let format = formats::pick(&eligible, &options.quality)?.clone();
    Ok((kind, format, formats::choices(eligible)))
}
type Indexes = (Arc<index::Index>, Option<Arc<index::Index>>);
async fn load_index(
    state: &AppState,
    address: &str,
    source: &RemoteSource,
) -> Result<Arc<index::Index>> {
    {
        let mut indexes = state.online.streams.indexes.lock().await;
        indexes.retain(|_, (expires, _)| *expires > now());
        if let Some((_, index)) = indexes.get(address) {
            return Ok(index.clone());
        }
    }
    let client = state.online.streams.relay.client(source.proxy.as_ref())?;
    let value = Arc::new(index::load(&client, address).await.map_err(|error| {
        if error.is::<index::Denied>() {
            denied()
        } else {
            ApiError::conflict("Could not read the public video's index")
        }
    })?);
    let mut indexes = state.online.streams.indexes.lock().await;
    if indexes.len() >= 128 {
        indexes.clear();
    }
    let expires = expiry(address).map_or(now() + 300, |at| at - 600);
    indexes.insert(address.into(), (expires, value.clone()));
    Ok(value)
}
async fn indexes(state: &AppState, format: &formats::Format, audio: bool) -> Result<Indexes> {
    wait_until(format.available_at).await;
    let source = &format.source;
    let video = load_index(state, &source.video, source);
    match source.audio.as_deref().filter(|_| audio) {
        Some(address) => {
            let (video, audio) = tokio::try_join!(video, load_index(state, address, source))?;
            Ok((video, Some(audio)))
        }
        None => Ok((video.await?, None)),
    }
}
/// Provider media requested before its availability time is refused.
pub(super) async fn wait_until(at: i64) {
    let wait = at - now();
    if (1..=60).contains(&wait) {
        tokio::time::sleep(Duration::from_secs(wait as u64)).await;
    }
}
async fn create_recorded(
    state: &AppState,
    p: &Principal,
    video: &str,
    prepared: Prepared,
    options: Options,
    position: Option<f64>,
    vod: bool,
) -> Result<Value> {
    let (mut kind, mut format, mut qualities) = plan(&prepared, &options, vod)?;
    let mut prepared = prepared;
    let indexes = if kind == Kind::Relay {
        None
    } else {
        match indexes(state, &format, kind == Kind::Indexed).await {
            Ok(value) => Some(value),
            Err(error) if error.1 == "network_blocked" => {
                // A cached signed address can be refused after extraction. Extract
                // again, moving to the other address family if it is blocked.
                state.online.streams.prepared.lock().await.remove(video);
                let excluded = &super::excluded_codecs(state).await?;
                let (fresh, _) = extract::with_metadata(state, video, |metadata, family| {
                    let options = &options;
                    async move {
                        let prepared =
                            select(&metadata, Some(proxy(state, family).await?), excluded)?;
                        let (kind, format, qualities) = plan(&prepared, options, vod)?;
                        let value = indexes(state, &format, kind == Kind::Indexed).await?;
                        Ok((prepared, kind, format, qualities, value))
                    }
                })
                .await?;
                remember(state, video, &fresh.0).await;
                (prepared, kind, format, qualities) = (fresh.0, fresh.1, fresh.2, fresh.3);
                Some(fresh.4)
            }
            Err(error) => return Err(error),
        }
    };
    let mut probe = format.probe(prepared.duration);
    if let Some((video, _)) = &indexes {
        let start = json!(format!("{:.6}", video.start));
        probe["format"]["start_time"] = start.clone();
        probe["streams"][0]["start_time"] = start;
    }
    let mut input = options.clone();
    // Provider resolution choices select an existing rendition; only a bitrate
    // ceiling that the selected rendition exceeds needs local conversion.
    if input.quality.ends_with('p') || format.quality() == input.quality {
        input.quality = "auto".into();
    }
    input.capabilities.containers.clear();
    let mut source = format.source.clone();
    let decision = if kind == Kind::Pipeline {
        // The provider's fragment index supplies the keyframes for seeking.
        source.segments = indexes.as_ref().unwrap().0.starts();
        let decision =
            thelxinoe_playback::decision(&media(&probe, prepared.duration, false), &input)
                .map_err(|e| ApiError::bad(e.to_string()))?;
        state.transcoding.read().await.check(decision.encoding)?;
        crate::tools::require(state, &["ffmpeg"]).await?;
        Some(decision)
    } else {
        None
    };
    let mode = decision.as_ref().map_or("direct", |d| d.mode);
    let sid = thelxinoe_core::id();
    let _lifecycle = state.playback_lifecycle.write(&[&sid]).await;
    let start = storage::create_with_delivery_write_twitch_streams(
        &state.db,
        storage::StreamSession {
            live_title: None,
            mode,
            key: sid.clone(),
            owner: p.user.id.clone(),
            auth: p.session_id.clone(),
            video: video.to_owned(),
            input: options.clone(),
            duration: prepared.duration,
            live: false,
            position,
        },
    )
    .await?;
    let (delivery, path) = match (kind, indexes) {
        (Kind::Relay, _) => (
            Delivery::Relay,
            format!("/api/v1/playback/{sid}/remote/video"),
        ),
        (Kind::Indexed, Some((video, Some(audio)))) => (
            Delivery::Indexed(Arc::new(Manifest {
                video,
                audio,
                format: format.clone(),
            })),
            format!("/api/v1/playback/{sid}/hls/{INDEXED}/index.m3u8"),
        ),
        _ => (Delivery::Pipeline, String::new()),
    };
    let mut session = Session {
        source: source.clone(),
        available_at: format.available_at,
        delivery,
        path,
    };
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .insert(sid.clone(), session.clone());
    if let Some(decision) = &decision {
        let started = state
            .playback
            .start_remote_vod(
                &sid,
                &source,
                &media(&probe, prepared.duration, false),
                &input,
                decision,
            )
            .await;
        match started {
            Ok((revision, _)) => {
                session.path = format!("/api/v1/playback/{sid}/hls/{revision}/index.m3u8");
                state
                    .online
                    .streams
                    .sessions
                    .lock()
                    .await
                    .insert(sid.clone(), session.clone());
            }
            Err(error) => {
                tracing::warn!(reason = %error, "Public stream preparation failed");
                state.online.streams.sessions.lock().await.remove(&sid);
                storage::create_with_delivery_write_playback_sessions(sid.clone(), &state.db)
                    .await?;
                return Err(ApiError::conflict(
                    "The server could not start the public stream; try again later",
                ));
            }
        }
    }
    let valid =
        storage::create_with_delivery_read_playback_sessions(sid.clone(), &state.db).await?;
    if !valid {
        state.playback.stop(&sid).await;
        state.online.streams.sessions.lock().await.remove(&sid);
        return Err(ApiError::not_found());
    }
    let grant = grants::issue(state, p, &format!("playback:{sid}"), 120).await?;
    let external_audio = (kind == Kind::Relay && source.audio.is_some())
        .then(|| format!("/api/v1/playback/{sid}/remote/audio?grant={grant}"));
    let encoding = decision.as_ref().map(|d| d.encoding).unwrap_or_default();
    Ok(
        json!({"id":sid,"url":format!("{}?grant={grant}",session.path),"external_audio":external_audio,"grant":grant,"mode":mode,"encoding":encoding,"conversion_reason":decision.as_ref().and_then(|d| d.conversion_reason.clone()),"position":start,"duration":prepared.duration,"timeline_start":0.0,"video":true,"live":false,"tracks":[],"subtitles":[],"selected_subtitle":null,"options":options,"qualities":qualities,"quality":(!encoding.video).then(|| format.quality()),"probe":probe,"replay_gain":"off"}),
    )
}
async fn create_live(
    state: &AppState,
    p: &Principal,
    video: &str,
    live_title: Option<String>,
    prepared: Prepared,
    options: Options,
) -> Result<Value> {
    let prepared = select_for_player(state, prepared, &options).await?;
    let qualities = formats::choices(
        prepared
            .formats
            .iter()
            .filter(|f| f.supported(&options.capabilities)),
    );
    let mut input = options.clone();
    if input.quality.ends_with('p') || prepared.quality.as_deref() == Some(input.quality.as_str()) {
        input.quality = "auto".into();
    }
    input.capabilities.containers.clear();
    let decision = thelxinoe_playback::decision(&prepared.media(), &input)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    state.transcoding.read().await.check(decision.encoding)?;
    crate::tools::require(state, &["ffmpeg"]).await?;
    let sid = thelxinoe_core::id();
    let _lifecycle = state.playback_lifecycle.write(&[&sid]).await;
    storage::create_with_delivery_write_twitch_streams(
        &state.db,
        storage::StreamSession {
            live_title,
            mode: decision.mode,
            key: sid.clone(),
            owner: p.user.id.clone(),
            auth: p.session_id.clone(),
            video: video.to_owned(),
            input: options.clone(),
            duration: 0.0,
            live: true,
            position: None,
        },
    )
    .await?;
    let mut session = Session {
        source: prepared.source.clone(),
        available_at: 0,
        delivery: Delivery::Pipeline,
        path: String::new(),
    };
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .insert(sid.clone(), session.clone());
    let revision = match state
        .playback
        .start_remote(&sid, &prepared.source, &prepared.probe, &input, &decision)
        .await
    {
        Ok((revision, _)) => revision,
        Err(error) => {
            tracing::warn!(reason = %error, "Public stream preparation failed");
            state.online.streams.sessions.lock().await.remove(&sid);
            storage::create_with_delivery_write_playback_sessions(sid.clone(), &state.db).await?;
            return Err(ApiError::conflict(
                "The server could not start the public stream; try again later",
            ));
        }
    };
    session.path = format!("/api/v1/playback/{sid}/hls/{revision}/index.m3u8");
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .insert(sid.clone(), session.clone());
    let valid =
        storage::create_with_delivery_read_playback_sessions(sid.clone(), &state.db).await?;
    if !valid {
        state.playback.stop(&sid).await;
        state.online.streams.sessions.lock().await.remove(&sid);
        return Err(ApiError::not_found());
    }
    let grant = grants::issue(state, p, &format!("playback:{sid}"), 120).await?;
    Ok(
        json!({"id":sid,"url":format!("{}?grant={grant}",session.path),"external_audio":null,"grant":grant,"mode":decision.mode,"encoding":decision.encoding,"conversion_reason":decision.conversion_reason,"position":0.0,"duration":0.0,"timeline_start":0.0,"video":true,"live":true,"tracks":[],"subtitles":[],"selected_subtitle":null,"options":options,"qualities":qualities,"quality":prepared.quality,"probe":prepared.probe,"replay_gain":"off"}),
    )
}
pub(crate) async fn validate(state: &AppState, id: &str) -> Result<()> {
    if !state.online.streams.sessions.lock().await.contains_key(id) {
        return Err(ApiError::conflict("Stream expired; start playback again"));
    }
    Ok(())
}
/// The original files of a relayed session, and when they become available.
pub(super) async fn remote_source(state: &AppState, id: &str) -> Result<(RemoteSource, i64)> {
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .get(id)
        .filter(|s| matches!(s.delivery, Delivery::Relay | Delivery::Indexed(_)))
        .map(|s| (s.source.clone(), s.available_at))
        .ok_or_else(ApiError::not_found)
}
async fn manifest(state: &AppState, id: &str) -> Option<Arc<Manifest>> {
    match &state.online.streams.sessions.lock().await.get(id)?.delivery {
        Delivery::Indexed(manifest) => Some(manifest.clone()),
        _ => None,
    }
}
/// A byte-range HLS playlist of an indexed session. Fragments and
/// initialization segments resolve to the session's relay.
pub(crate) async fn playlist(
    state: &AppState,
    id: &str,
    revision: &str,
    name: &str,
    grant: &str,
) -> Option<String> {
    if revision != INDEXED {
        return None;
    }
    let manifest = manifest(state, id).await?;
    let media = |track: &str, index: &index::Index| {
        index.playlist(
            &format!("../../remote/{track}?grant={grant}"),
            &format!("../../remote/{track}-init?grant={grant}"),
        )
    };
    match name {
        "index.m3u8" => Some(manifest.master(grant)),
        "video.m3u8" => Some(media("video", &manifest.video)),
        "audio.m3u8" => Some(media("audio", &manifest.audio)),
        _ => None,
    }
}
/// An indexed track's initialization segment, read with its index.
pub(super) async fn initialization(
    state: &AppState,
    id: &str,
    track: &str,
) -> Option<(&'static str, axum::body::Bytes)> {
    let manifest = manifest(state, id).await?;
    let (kind, index) = match track {
        "video" => ("video/mp4", &manifest.video),
        "audio" => ("audio/mp4", &manifest.audio),
        _ => return None,
    };
    Some((kind, axum::body::Bytes::copy_from_slice(&index.init)))
}
/// Recorded videos are complete timelines that clients seek within, so a
/// seek only renews the session's address.
pub(crate) async fn seek(state: &AppState, id: &str) -> Result<(String, f64)> {
    let session = state
        .online
        .streams
        .sessions
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| ApiError::conflict("Stream expired; start playback again"))?;
    if session.source.live {
        return Err(ApiError::bad("Seeking is not available for live streams"));
    }
    Ok((session.path, 0.0))
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
    state
        .online
        .streams
        .indexes
        .lock()
        .await
        .retain(|_, (expires, _)| *expires > now());
    state
        .online
        .streams
        .probes
        .lock()
        .await
        .retain(|_, (expires, _)| *expires > now());
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
        let vod = select(&metadata(false), None, &[]).unwrap();
        assert_eq!(vod.duration, 100.0);
        assert!(vod.source.audio.is_some());
        assert!(vod.formats[0].native && vod.formats[0].indexed);
        assert_eq!(vod.probe["streams"][0]["height"], 1080);
        assert!(!vod.probe.to_string().contains("googlevideo"));
        let live = select(&metadata(true), None, &[]).unwrap();
        assert!(!live.formats[0].native && !live.formats[0].indexed);
        assert_eq!(live.duration, 0.0);
        let mut live = metadata(true);
        live["formats"][1]["acodec"] = Value::Null;
        live["formats"][1]["protocol"] = json!("m3u8_native");
        assert!(select(&live, None, &[]).unwrap().source.audio.is_some());
        // Recorded videos play from their separate files, never a playlist.
        for track in [0, 1] {
            let mut playlist = metadata(false);
            playlist["formats"][track]["protocol"] = json!("m3u8_native");
            assert!(select(&playlist, None, &[]).is_err());
        }
        let mut malformed = metadata(false);
        malformed["formats"][0]["url"] = json!("https://localhost/private");
        assert!(select(&malformed, None, &[]).is_err());
    }
    #[test]
    fn signed_addresses_are_reused_until_shortly_before_they_expire() {
        let mut metadata = metadata(false);
        metadata["formats"][0]["url"] = json!(format!(
            "https://r1.googlevideo.com/video?expire={}&signature=private",
            now() + 6 * 3600
        ));
        assert!(select(&metadata, None, &[]).unwrap().expires >= now() + 3599);
        metadata["formats"][0]["url"] = json!(format!(
            "https://r1.googlevideo.com/video?expire={}",
            now() + 900
        ));
        let expires = select(&metadata, None, &[]).unwrap().expires;
        assert!((now() + 250..=now() + 300).contains(&expires));
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
            .insert(video.into(), select(&metadata(false), None, &[]).unwrap());
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
        // A valid session still cannot select an arbitrary upstream address,
        // and relayed files have no in-memory initialization segment.
        for track in ["other", "video-init"] {
            assert_eq!(
                call(
                    &state,
                    &format!("/api/v1/playback/{id}/remote/{track}?grant={grant}"),
                    "GET",
                    json!({}),
                    &cookie
                )
                .await
                .0,
                StatusCode::NOT_FOUND
            );
        }
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
    async fn concurrent_requests_share_one_extraction() {
        let (_temp, state, _) = fixture().await;
        let gate = state
            .online
            .streams
            .pending
            .lock()
            .unwrap()
            .entry("abcdefghijk".into())
            .or_default()
            .clone();
        let held = gate.lock_owned().await;
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { prepare(&state, "abcdefghijk").await.map(|p| p.duration) }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The first request's result reaches every request waiting for it.
        remember(
            &state,
            "abcdefghijk",
            &select(&metadata(false), None, &[]).unwrap(),
        )
        .await;
        drop(held);
        assert_eq!(waiting.await.unwrap().unwrap(), 100.0);
        assert!(state.online.streams.pending.lock().unwrap().is_empty());
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
            .insert(video.into(), select(&metadata(true), None, &[]).unwrap());
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
