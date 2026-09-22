#[path = "storage/playback.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use thelxinoe_core::{Principal, id, now};
use thelxinoe_playback::{Options, Preferences, Source};
use tower::ServiceExt;

pub async fn source(state: &AppState, media: &str, file: Option<&str>) -> Result<Source> {
    let media = media.to_owned();
    let file = file.map(str::to_owned);
    storage::source(media, file, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}
async fn user_preferences(state: &AppState, p: &Principal) -> Result<Preferences> {
    let user = p.user.id.clone();
    Ok(storage::user_preferences(user, &state.db)
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}
pub async fn preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Preferences>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(user_preferences(&state, &p).await?))
}
pub async fn save_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(value): Json<Preferences>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    value.validate().map_err(|e| ApiError::bad(e.to_string()))?;
    storage::save_preferences(&state.db, value, p).await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn media_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if crate::online::live::domain(&media) {
        let mut info = crate::online::streams::info(&state, &p, &media).await?;
        info["preferences"] = serde_json::to_value(user_preferences(&state, &p).await?)
            .expect("preference serialization");
        return Ok(Json(info));
    }
    if let Some(video) = media.strip_prefix("youtube:") {
        crate::online::downloads::authorize(&state, &p, video).await?;
        let source = match crate::online::downloads::source(&state, video).await {
            Ok(value) => value,
            Err(error) if error.0 == axum::http::StatusCode::CONFLICT => {
                let mut info = crate::online::streams::info(&state, &p, video).await?;
                info["preferences"] = serde_json::to_value(user_preferences(&state, &p).await?)
                    .map_err(anyhow::Error::from)?;
                return Ok(Json(info));
            }
            Err(error) => return Err(error),
        };
        let video = video.to_owned();
        let owner = p.user.id.clone();
        let (position, watched) =
            storage::media_info_read_youtube_state(&state.db, video, owner).await?;
        return Ok(Json(
            json!({"sources":[{"id":source.id,"edition":source.edition,"duration":source.duration(),"video":source.video_codec().is_some(),"tracks":source.tracks().await?,"probe":source.probe,"size":source.size}],"progress":[{"edition":"public","position":position,"duration":source.duration()}],"watched":watched,"preferences":user_preferences(&state,&p).await?}),
        ));
    }
    let user = p.user.id.clone();
    let mid = media.clone();
    let (files, progress, watched) =
        storage::media_info_read_media_sources(&state.db, user, mid).await?;
    let mut sources = Vec::new();
    for file in files {
        let source = source(&state, &media, Some(&file)).await?;
        sources.push(json!({"id":source.id,"edition":source.edition,"duration":source.duration(),"video":source.video_codec().is_some(),"tracks":source.tracks().await?,"probe":source.probe,"size":source.size}));
    }
    Ok(Json(
        json!({"sources":sources,"progress":progress,"watched":watched,"preferences":user_preferences(&state,&p).await?}),
    ))
}
#[derive(Deserialize)]
pub struct Create {
    pub queue: Option<crate::user_media::QueueContext>,
    pub media_id: String,
    pub file_id: Option<String>,
    pub position: Option<f64>,
    pub options: Options,
}
pub async fn create(
    State(state): State<AppState>,
    axum::Extension(context): axum::Extension<security::RequestContext>,
    headers: HeaderMap,
    Json(mut input): Json<Create>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if context.remote() && input.options.quality == "auto" && input.options.capabilities.hls {
        let video = crate::online::live::domain(&input.media_id)
            || input.media_id.starts_with("youtube:")
            || source(&state, &input.media_id, input.file_id.as_deref())
                .await?
                .video_codec()
                .is_some();
        if video {
            input.options.quality = "4mbps".into();
        }
    }
    create_for(&state, &p, input).await.map(Json)
}
pub async fn create_for(state: &AppState, p: &Principal, input: Create) -> Result<Value> {
    create_with_delivery(state, p, input, false).await
}
pub(crate) async fn create_with_delivery(
    state: &AppState,
    p: &Principal,
    mut input: Create,
    vod: bool,
) -> Result<Value> {
    let _lease = state.media_operations.read().await;
    if let Some(queue) = &input.queue {
        if !crate::user_media::valid_client(&queue.client_id) || queue.index >= 500 {
            return Err(ApiError::bad("Invalid queue context"));
        }
        let user = p.clone();
        let queue = queue.clone();
        let media = input.media_id.clone();
        let matches = storage::queue_matches(user, queue, media, &state.db).await?;
        if !matches {
            return Err(ApiError::conflict(
                "The music queue changed; reload it before playing",
            ));
        }
    }
    if crate::online::live::domain(&input.media_id) {
        if input.queue.is_some() {
            return Err(ApiError::bad(
                "Live channels are not part of the local library",
            ));
        }
        return crate::online::streams::create_with_delivery(
            state,
            p,
            &input.media_id,
            input.options,
            input.position,
            vod,
        )
        .await;
    }
    let online_video = input.media_id.strip_prefix("youtube:").map(str::to_owned);
    let source = if let Some(video) = &online_video {
        crate::online::downloads::authorize(state, p, video).await?;
        if input.queue.is_some() {
            return Err(ApiError::bad(
                "Online media is not part of the local library",
            ));
        }
        match crate::online::downloads::source(state, video).await {
            Ok(value) => value,
            Err(error) if error.0 == axum::http::StatusCode::CONFLICT => {
                return crate::online::streams::create_with_delivery(
                    state,
                    p,
                    video,
                    input.options,
                    input.position,
                    vod,
                )
                .await;
            }
            Err(error) => return Err(error),
        }
    } else {
        source(state, &input.media_id, input.file_id.as_deref()).await?
    };
    source
        .validate()
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    let prefs = user_preferences(state, p).await?;
    let tracks = source.tracks().await?;
    if input.options.audio.is_none() {
        input.options.audio = tracks
            .iter()
            .find(|t| {
                t.kind == "audio"
                    && !prefs.audio_language.is_empty()
                    && t.language == prefs.audio_language
            })
            .and_then(|t| t.index);
    }
    if input.options.subtitle.is_none() && prefs.subtitles {
        input.options.subtitle = tracks
            .iter()
            .find(|t| t.kind == "subtitle" && t.supported && t.language == prefs.subtitle_language)
            .map(|t| t.id.clone());
    }
    if let Some(track) = &input.options.subtitle
        && track != "off"
        && !tracks
            .iter()
            .any(|t| t.kind == "subtitle" && t.supported && &t.id == track)
    {
        return Err(ApiError::bad("Unsupported subtitle track"));
    }
    let mode = thelxinoe_playback::plan(&source, &input.options)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let duration = source.duration();
    if duration <= 0.0 || !duration.is_finite() {
        return Err(ApiError::conflict(
            "Media duration is unavailable; rescan the library",
        ));
    }
    let sid = id();
    let key = sid.clone();
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let src = source.clone();
    let options = input.options.clone();
    let position = storage::create_session(
        &state.db,
        storage::PlaybackSession {
            online_video,
            mode,
            duration,
            key,
            user,
            auth,
            src,
            options,
            position: input.position,
            queue: input.queue.clone(),
        },
    )
    .await?;
    let grant = grants::issue(state, p, &format!("playback:{sid}"), 120).await?;
    let mut timeline_start = 0.0;
    let url = if mode == "direct" {
        format!("/api/v1/playback/{sid}/stream?grant={grant}")
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
                format!("/api/v1/playback/{sid}/hls/{revision}/index.m3u8?grant={grant}")
            }
            Err(error) => {
                fail(state, &sid).await?;
                return Err(ApiError::conflict(error.to_string()));
            }
        }
    };
    let subtitles:Vec<_>=tracks.iter().filter(|t|t.kind=="subtitle" && t.supported).map(|t|json!({"id":t.id,"language":t.language,"title":t.title,"url":format!("/api/v1/playback/{sid}/subtitles/{}?grant={grant}",t.id)})).collect();
    Ok(
        json!({"id":sid,"file_id":source.id,"generation":source.generation,"url":url,"grant":grant,"mode":mode,"position":position,"duration":duration,"timeline_start":timeline_start,"video":source.video_codec().is_some(),"tracks":tracks,"subtitles":subtitles,"selected_subtitle":input.options.subtitle,"options":input.options,"probe":source.probe,"replay_gain":prefs.replay_gain}),
    )
}
#[derive(Clone)]
struct Session {
    media: String,
    file: String,
    generation: String,
    mode: String,
    options: Options,
    duration: f64,
    streaming: bool,
}
async fn session(state: &AppState, p: &Principal, id: &str) -> Result<Session> {
    let id = id.to_owned();
    let p = p.clone();
    storage::session(id, p, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}
async fn current_source(state: &AppState, session: &Session) -> Result<Source> {
    let src = if let Some(video) = session.media.strip_prefix("youtube:") {
        crate::online::downloads::source(state, video).await?
    } else {
        source(state, &session.media, Some(&session.file)).await?
    };
    if src.generation != session.generation {
        return Err(ApiError::conflict(
            "Media was replaced; start a new playback session",
        ));
    }
    src.validate()
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    Ok(src)
}
#[derive(Deserialize)]
pub struct Grant {
    pub(crate) grant: String,
}
async fn from_grant(state: &AppState, id: &str, grant: &str) -> Result<(Principal, Session)> {
    let p = grants::resolve(state, grant, &format!("playback:{id}"), false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let session = session(state, &p, id).await?;
    Ok((p, session))
}
pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(grant): Query<Grant>,
    request: Request,
) -> Result<Response> {
    let (_, session) = from_grant(&state, &id, &grant.grant).await?;
    if session.streaming {
        return Err(ApiError::not_found());
    }
    let src = current_source(&state, &session).await?;
    let mut response = tower_http::services::ServeFile::new(&src.path)
        .oneshot(request)
        .await
        .unwrap()
        .map(Body::new);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        thelxinoe_playback::media_type(&src.path).parse().unwrap(),
    );
    Ok(response)
}
pub async fn remote(
    State(state): State<AppState>,
    Path((id, track)): Path<(String, String)>,
    Query(grant): Query<Grant>,
    request: Request,
) -> Result<Response> {
    let (p, session) = from_grant(&state, &id, &grant.grant).await?;
    if !session.streaming || session.mode != "direct" || !session.options.capabilities.native_remote
    {
        return Err(ApiError::not_found());
    }
    let video = session
        .media
        .strip_prefix("youtube:")
        .ok_or_else(ApiError::not_found)?;
    crate::online::downloads::authorize(&state, &p, video).await?;
    crate::online::relay::stream(&state, &id, &track, request).await
}
pub async fn hls(
    State(state): State<AppState>,
    Path((id, revision, name)): Path<(String, String, String)>,
    Query(grant): Query<Grant>,
    request: Request,
) -> Result<Response> {
    let (_, session) = from_grant(&state, &id, &grant.grant).await?;
    if session.streaming {
        crate::online::streams::validate(&state, &id).await?;
    } else {
        current_source(&state, &session).await?;
    }
    let path = state
        .playback
        .file(&id, &revision, &name)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if name == "index.m3u8" {
        let playlist = tokio::fs::read_to_string(path)
            .await
            .map_err(|_| ApiError::not_found())?;
        let playlist = playlist
            .lines()
            .map(|line| {
                if line.starts_with("segment-") {
                    format!("{line}?grant={}", grant.grant)
                } else {
                    line.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok((
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
            playlist + "\n",
        )
            .into_response());
    }
    Ok(tower_http::services::ServeFile::new(path)
        .oneshot(request)
        .await
        .unwrap()
        .map(Body::new))
}
#[derive(Deserialize)]
pub struct Position {
    position: f64,
}
pub async fn keepalive(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    session(&state, &p, &id).await?;
    storage::keepalive(&state.db, id).await?;
    Ok(Json(json!({"active":true})))
}
pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    session(&state, &p, &id).await?;
    let key = id.clone();
    storage::cancel(&state.db, key).await?;
    state.playback.stop(&id).await;
    Ok(Json(json!({"stopped":true})))
}
pub async fn seek(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Position>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let session = session(&state, &p, &id).await?;
    if !input.position.is_finite() || input.position < 0.0 || input.position >= session.duration {
        return Err(ApiError::bad("Seek outside media duration"));
    }
    let grant = grants::issue(&state, &p, &format!("playback:{id}"), 120).await?;
    let mut timeline_start = 0.0;
    let url = if session.streaming && session.mode == "direct" {
        crate::online::streams::validate(&state, &id).await?;
        format!("/api/v1/playback/{id}/remote/video?grant={grant}")
    } else if session.streaming {
        let (revision, offset) =
            crate::online::streams::seek(&state, &id, &session.options, input.position).await?;
        timeline_start = offset;
        format!("/api/v1/playback/{id}/hls/{revision}/index.m3u8?grant={grant}")
    } else if session.mode == "direct" {
        current_source(&state, &session).await?;
        format!("/api/v1/playback/{id}/stream?grant={grant}")
    } else {
        let src = current_source(&state, &session).await?;
        let (revision, offset) = state
            .playback
            .start(&id, &src, &session.options, &session.mode, input.position)
            .await
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        timeline_start = offset;
        format!("/api/v1/playback/{id}/hls/{revision}/index.m3u8?grant={grant}")
    };
    Ok(Json(
        json!({"url":url,"position":input.position,"timeline_start":timeline_start}),
    ))
}
#[derive(Deserialize)]
pub struct Progress {
    pub sequence: i64,
    pub position: f64,
    pub state: String,
    /// Cumulative active wall seconds. Compatibility clients omit this field.
    pub active_seconds: Option<f64>,
}
pub async fn progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Progress>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    report(&state, &p, &id, input).await.map(Json)
}
pub async fn report(state: &AppState, p: &Principal, id: &str, input: Progress) -> Result<Value> {
    let _lease = state.media_operations.read().await;
    if input.sequence < 0
        || input
            .active_seconds
            .is_some_and(|value| !value.is_finite() || !(0.0..=1e12).contains(&value))
        || !input.position.is_finite()
        || input.position < 0.0
        || !["playing", "paused", "stopped"].contains(&input.state.as_str())
    {
        return Err(ApiError::bad("Invalid playback progress"));
    }
    // Authorize even a duplicate terminal report; it must never reopen a stopped session.
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let key = id.to_owned();
    let stopped = input.state == "stopped";
    let updated = storage::report(user, auth, key, stopped, &state.db, input)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if updated {
        if stopped {
            state.playback.stop(id).await;
        }
        state.notify_events();
    }
    Ok(json!({"accepted":updated}))
}
#[derive(Deserialize)]
pub struct SubtitleQuery {
    grant: String,
    format: Option<String>,
}
pub async fn subtitle(
    State(state): State<AppState>,
    Path((id, track)): Path<(String, String)>,
    Query(grant): Query<SubtitleQuery>,
) -> Result<Response> {
    let (format, content_type) = match grant.format.as_deref().unwrap_or("vtt") {
        "vtt" => ("webvtt", "text/vtt; charset=utf-8"),
        "srt" => ("srt", "application/x-subrip; charset=utf-8"),
        _ => return Err(ApiError::bad("Unsupported subtitle format")),
    };
    let (_, session) = from_grant(&state, &id, &grant.grant).await?;
    let source = current_source(&state, &session).await?;
    let track = source
        .tracks()
        .await?
        .into_iter()
        .find(|t| t.id == track && t.kind == "subtitle" && t.supported)
        .ok_or_else(ApiError::not_found)?;
    let _slot = state
        .subtitle_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(track.path.as_ref().unwrap_or(&source.path));
    if let Some(index) = track.index {
        command.args(["-map", &format!("0:{index}")]);
    }
    command
        .args(["-f", format, "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(anyhow::Error::from)?;
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 8 * 1024 * 1024 {
            anyhow::bail!("Subtitle exceeds size limit");
        }
        if !child.wait().await?.success() {
            anyhow::bail!("Subtitle conversion failed");
        }
        Ok::<_, anyhow::Error>(bytes)
    })
    .await
    .map_err(|_| ApiError::conflict("Subtitle conversion timed out"))??;
    Ok(([(header::CONTENT_TYPE, content_type)], output).into_response())
}
async fn fail(state: &AppState, id: &str) -> anyhow::Result<()> {
    let id = id.to_owned();
    storage::fail(id, &state.db).await
}
pub async fn maintain(state: AppState) -> anyhow::Result<()> {
    // Process state cannot survive a server restart, but resume records do.
    storage::maintain_write_playback_sessions(&state.db).await?;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let active = storage::expire_sessions(&state.db).await?;
        for id in state.playback.maintain(&active).await? {
            fail(&state, &id).await?;
        }
        crate::online::streams::maintain(&state, &active).await;
    }
}
