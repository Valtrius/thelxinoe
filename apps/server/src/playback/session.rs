//! Starting and restarting a playback session.
use super::{Create, current_source, fail, source, storage, user_preferences};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants,
};
use serde::Serialize;
use serde_json::Value;
use thelxinoe_core::{Principal, id};
use thelxinoe_playback::{Chapter, Encoding, Options, Orientation, Preferences, Track};

/// A started playback session. The web API sends it as it is.
#[derive(Debug, Serialize)]
pub struct Started {
    pub id: String,
    /// The media, through `grant`.
    pub url: String,
    pub grant: String,
    pub mode: &'static str,
    pub encoding: Encoding,
    pub conversion_reason: Option<String>,
    pub position: f64,
    pub duration: f64,
    /// The media time at which the delivered stream starts.
    pub timeline_start: f64,
    pub video: bool,
    pub tracks: Vec<Track>,
    pub subtitles: Vec<Subtitle>,
    pub selected_subtitle: Option<String>,
    /// The client's options, with the tracks chosen for it.
    pub options: Options,
    pub chapters: Vec<Chapter>,
    pub probe: Value,
    pub replay_gain: String,
    #[serde(flatten)]
    pub origin: Origin,
}
impl Started {
    pub fn direct(&self) -> bool {
        self.mode == "direct"
    }
    /// Whether an online stream is live; a file does not say.
    pub fn live(&self) -> Option<bool> {
        match self.origin {
            Origin::Online { live, .. } => Some(live),
            Origin::File { .. } => None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Subtitle {
    pub id: String,
    pub language: String,
    pub title: String,
    pub codec: String,
    pub forced: bool,
    /// A picture subtitle has no file: a transcode burns it in.
    pub url: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Origin {
    /// A media file, including a downloaded YouTube video.
    File {
        file_id: String,
        generation: String,
        fonts: Vec<String>,
        trickplay: Value,
    },
    /// A YouTube video or live stream played from its provider.
    Online {
        external_audio: Option<String>,
        live: bool,
        qualities: Vec<Value>,
        quality: Option<String>,
        /// Only a live stream reports the orientation it delivers.
        #[serde(skip_serializing_if = "Option::is_none")]
        orientation: Option<Option<Orientation>>,
    },
}
/// Where a restarted playback session continues.
#[derive(Debug, Serialize)]
pub struct Restarted {
    pub url: String,
    pub position: f64,
    pub timeline_start: f64,
}

/// Starts a playback session. `vod` prepares a complete playlist before returning, for
/// clients that cannot follow a growing one.
pub(crate) async fn start(
    state: &AppState,
    p: &Principal,
    mut input: Create,
    vod: bool,
) -> Result<Started> {
    if crate::online::live::domain(&input.media_id) {
        let _lease = state.media_operations.read().await;
        return crate::online::streams::start(
            state,
            p,
            &input.media_id,
            input.options,
            input.position,
            vod,
            input.orientation,
        )
        .await;
    }
    let online_video = input.media_id.strip_prefix("youtube:").map(str::to_owned);
    let source = if let Some(video) = &online_video {
        crate::online::downloads::authorize(state, p, video).await?;
        match crate::online::downloads::source(state, video).await {
            Ok(value) => value,
            Err(error) if error.0 == axum::http::StatusCode::CONFLICT => {
                let _lease = state.media_operations.read().await;
                return crate::online::streams::start(
                    state,
                    p,
                    video,
                    input.options,
                    input.position,
                    vod,
                    input.orientation,
                )
                .await;
            }
            Err(error) => return Err(error),
        }
    } else {
        source(state, &input.media_id, input.file_id.as_deref()).await?
    };
    let file_key = format!("file:{}", source.id);
    let _file = state.media_resources.read(&[&file_key]).await;
    let _lease = state.media_operations.read().await;
    source
        .validate()
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    let prefs = user_preferences(state, p).await?;
    let tracks = source.tracks().await?;
    select_tracks(&tracks, &prefs, &mut input.options)?;
    let decision = thelxinoe_playback::decision(&source, &input.options)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let policy = state.transcoding.read().await;
    policy.check(decision.encoding)?;
    let mode = decision.mode;
    let duration = source.duration();
    if duration <= 0.0 || !duration.is_finite() {
        return Err(ApiError::conflict(
            "Media duration is unavailable; rescan the library",
        ));
    }
    if mode != "direct" {
        crate::tools::require(state, &["ffmpeg"]).await?;
    }
    let sid = id();
    let _lifecycle = state.playback_lifecycle.write(&[&sid]).await;
    let position = storage::create_session(
        &state.db,
        storage::PlaybackSession {
            online_video,
            mode,
            duration,
            key: sid.clone(),
            user: p.user.id.clone(),
            auth: p.session_id.clone(),
            src: source.clone(),
            options: input.options.clone(),
            position: input.position,
        },
    )
    .await?;
    let grant = grants::issue(state, p, &format!("playback:{sid}"), 120).await?;
    let mut timeline_start = 0.0;
    let url = if mode == "direct" {
        file_url(&sid, &grant)
    } else {
        let prepared = if vod {
            state
                .playback
                .start_vod(&sid, &source, &input.options, mode)
                .await
        } else {
            state
                .playback
                .start(&sid, &source, &input.options, mode, position)
                .await
        };
        match prepared {
            Ok((revision, offset)) => {
                timeline_start = offset;
                hls_url(&sid, &revision, &grant)
            }
            Err(error) => {
                fail(state, &sid).await?;
                return Err(ApiError::conflict(error.to_string()));
            }
        }
    };
    let subtitles = subtitles(
        &tracks,
        input.options.capabilities.burn_subtitles,
        &sid,
        &grant,
    );
    let fonts = thelxinoe_playback::font_attachments(&source.probe)
        .into_iter()
        .map(|index| format!("/api/v1/playback/{sid}/fonts/{index}?grant={grant}"))
        .collect();
    let trickplay = crate::trickplay::web(state, p, &source).await?;
    Ok(Started {
        url,
        mode,
        encoding: decision.encoding,
        conversion_reason: decision.conversion_reason,
        position,
        duration,
        timeline_start,
        video: source.video_codec().is_some(),
        tracks,
        subtitles,
        selected_subtitle: input.options.subtitle.clone(),
        options: input.options,
        chapters: thelxinoe_playback::chapters(&source.probe),
        replay_gain: prefs.replay_gain,
        origin: Origin::File {
            file_id: source.id,
            generation: source.generation,
            fonts,
            trickplay,
        },
        probe: source.probe,
        id: sid,
        grant,
    })
}
/// Restarts playback session `id` of `p` at `position`, with a new grant.
pub(crate) async fn restart(
    state: &AppState,
    p: &Principal,
    id: &str,
    position: f64,
) -> Result<Restarted> {
    let policy = state.transcoding.read().await;
    let _lifecycle = state.playback_lifecycle.write(&[id]).await;
    let session = super::session(state, p, id).await?;
    if !position.is_finite() || position < 0.0 || position >= session.duration {
        return Err(ApiError::bad("Seek outside media duration"));
    }
    let grant = grants::issue(state, p, &format!("playback:{id}"), 120).await?;
    let mut timeline_start = 0.0;
    let url = if session.streaming {
        let (path, offset) = crate::online::streams::seek(state, id).await?;
        timeline_start = offset;
        format!("{path}?grant={grant}")
    } else if session.mode == "direct" {
        current_source(state, &session).await?;
        file_url(id, &grant)
    } else {
        let src = current_source(state, &session).await?;
        policy.check(
            thelxinoe_playback::decision(&src, &session.options)
                .map_err(|e| ApiError::bad(e.to_string()))?
                .encoding,
        )?;
        let (revision, offset) = state
            .playback
            .start(id, &src, &session.options, &session.mode, position)
            .await
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        timeline_start = offset;
        hls_url(id, &revision, &grant)
    };
    Ok(Restarted {
        url,
        position,
        timeline_start,
    })
}
fn file_url(id: &str, grant: &str) -> String {
    format!("/api/v1/playback/{id}/stream?grant={grant}")
}
fn hls_url(id: &str, revision: &str, grant: &str) -> String {
    format!("/api/v1/playback/{id}/hls/{revision}/index.m3u8?grant={grant}")
}
/// Whether the client can show subtitle `track`: a picture one only when a transcode burns
/// it in.
fn shown(track: &Track, burn: bool) -> bool {
    track.kind == "subtitle" && (track.supported || burn && track.picture())
}
/// Chooses the audio and subtitle tracks the client left unset from the viewer's preferences,
/// and refuses a subtitle the client cannot show.
fn select_tracks(tracks: &[Track], prefs: &Preferences, options: &mut Options) -> Result<()> {
    if options.audio.is_none() {
        options.audio = tracks
            .iter()
            .find(|t| {
                t.kind == "audio"
                    && !prefs.audio_language.is_empty()
                    && t.language == prefs.audio_language
            })
            .and_then(|t| t.index);
    }
    // Only text subtitles show by default: a picture one needs a transcode, which the viewer
    // asks for by choosing it.
    if options.subtitle.is_none() {
        let preferred = tracks.iter().find(|t| {
            prefs.subtitles
                && t.kind == "subtitle"
                && t.supported
                && t.language == prefs.subtitle_language
        });
        // A client that selects embedded tracks itself also selects their forced subtitles.
        let forced = || {
            (!options.capabilities.native_tracks)
                .then(|| thelxinoe_playback::forced_subtitle(tracks, options.audio))
                .flatten()
        };
        options.subtitle = preferred.or_else(forced).map(|t| t.id.clone());
    }
    let burn = options.capabilities.burn_subtitles;
    if let Some(track) = &options.subtitle
        && track != "off"
        && !tracks.iter().any(|t| shown(t, burn) && &t.id == track)
    {
        return Err(ApiError::bad("Unsupported subtitle track"));
    }
    Ok(())
}
fn subtitles(tracks: &[Track], burn: bool, id: &str, grant: &str) -> Vec<Subtitle> {
    tracks
        .iter()
        .filter(|t| shown(t, burn))
        .map(|t| Subtitle {
            id: t.id.clone(),
            language: t.language.clone(),
            title: t.title.clone(),
            codec: t.codec.clone(),
            forced: t.forced,
            url: t
                .supported
                .then(|| format!("/api/v1/playback/{id}/subtitles/{}?grant={grant}", t.id)),
        })
        .collect()
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
