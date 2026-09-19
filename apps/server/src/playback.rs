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
    state.db.call(move|db| {
        Ok(db.query_row("SELECT f.id,f.generation,f.edition,f.path,r.path,f.size,f.modified,f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id JOIN library_roots r ON r.id=f.root_id WHERE s.media_id=?1 AND f.present=1 AND (?2 IS NULL OR f.id=?2) ORDER BY f.edition,f.id LIMIT 1",params![media,file],|r|Ok(Source {id:r.get(0)?,media_id:media.clone(),generation:r.get(1)?,edition:r.get(2)?,path:PathBuf::from(r.get::<_,String>(3)?),root:PathBuf::from(r.get::<_,String>(4)?),size:r.get::<_,i64>(5)? as u64,modified:r.get(6)?,probe:serde_json::from_str(&r.get::<_,String>(7)?).unwrap_or_default()})).optional()?)
    }).await?.ok_or_else(ApiError::not_found)
}
async fn user_preferences(state: &AppState, p: &Principal) -> Result<Preferences> {
    let user = p.user.id.clone();
    Ok(state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT value FROM playback_preferences WHERE user_id=?1",
                    [user],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
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
    state.db.call(move|db|{db.execute("INSERT INTO playback_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![p.user.id,serde_json::to_string(&value)?])?;Ok(())}).await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn media_info(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if let Some(video) = media.strip_prefix("youtube:") {
        crate::online::downloads::authorize(&state, &p, video).await?;
        let source = crate::online::downloads::source(&state, video).await?;
        let video = video.to_owned();
        let owner = p.user.id.clone();
        let (position,watched)=state.db.call(move |db| Ok(db.query_row("SELECT position,watched FROM youtube_state WHERE user_id=?1 AND video_id=?2",params![owner,video],|r|Ok((r.get::<_,f64>(0)?,r.get::<_,bool>(1)?))).optional()?.unwrap_or((0.0,false)))).await?;
        return Ok(Json(
            json!({"sources":[{"id":source.id,"edition":source.edition,"duration":source.duration(),"video":source.video_codec().is_some(),"tracks":source.tracks().await?,"probe":source.probe,"size":source.size}],"progress":[{"edition":"public","position":position,"duration":source.duration()}],"watched":watched,"preferences":user_preferences(&state,&p).await?}),
        ));
    }
    let user = p.user.id.clone();
    let mid = media.clone();
    let (files,progress,watched)=state.db.call(move|db| {
        let files=db.prepare("SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition,f.id")?.query_map([&mid],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let progress=db.prepare("SELECT edition,position,duration FROM edition_progress WHERE user_id=?1 AND media_id=?2")?.query_map(params![user,mid],|r|Ok(json!({"edition":r.get::<_,String>(0)?,"position":r.get::<_,f64>(1)?,"duration":r.get::<_,f64>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let watched=db.query_row("SELECT watched FROM media_state WHERE user_id=?1 AND media_id=?2",params![user,mid],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        Ok((files,progress,watched))
    }).await?;
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
    headers: HeaderMap,
    Json(input): Json<Create>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
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
    if let Some(queue) = &input.queue {
        if !crate::user_media::valid_client(&queue.client_id) || queue.index >= 500 {
            return Err(ApiError::bad("Invalid queue context"));
        }
        let user = p.clone();
        let queue = queue.clone();
        let media = input.media_id.clone();
        let matches = state
            .db
            .call(move |db| {
                let saved = crate::user_media::queue_value(db, &user, &queue.client_id)?;
                Ok(saved["revision"].as_i64() == Some(queue.revision)
                    && saved["items"][queue.index].as_str() == Some(&media))
            })
            .await?;
        if !matches {
            return Err(ApiError::conflict(
                "The music queue changed; reload it before playing",
            ));
        }
    }
    let online_video = input.media_id.strip_prefix("youtube:").map(str::to_owned);
    let source = if let Some(video) = &online_video {
        crate::online::downloads::authorize(state, p, video).await?;
        if input.queue.is_some() || vod {
            return Err(ApiError::bad(
                "Online media is not part of the local library",
            ));
        }
        crate::online::downloads::source(state, video).await?
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
    let position=state.db.call(move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved=if let Some(video)=&online_video {tx.query_row("SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",params![user,video],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0)} else {tx.query_row("SELECT position FROM edition_progress WHERE user_id=?1 AND media_id=?2 AND edition=?3",params![user,src.media_id,src.edition],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0)};
        let position=input.position.unwrap_or(if saved>=duration*0.9 {0.0} else {saved}).clamp(0.0,duration);
        if !position.is_finite() {anyhow::bail!("Invalid playback position");}
        if let Some(video)=&online_video {
            let available=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_downloads d JOIN youtube_videos v ON v.video_id=d.video_id WHERE d.video_id=?1 AND d.generation=?2 AND d.state='ready' AND v.user_id=?3)",params![video,src.generation,user],|r|r.get::<_,bool>(0))?;
            anyhow::ensure!(available,"Online media changed before playback");
        }
        tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,client_id,queue_revision,queue_index,youtube_video_id) VALUES (?1,?2,?3,?4,?5,?6,?7,'ready',?8,?9,?10,?11,?12,?12,?13,?14,?15,?16)",params![key,user,auth,online_video.is_none().then_some(&src.media_id),online_video.is_none().then_some(&src.id),src.generation,src.edition,mode,serde_json::to_string(&options)?,duration,position,now(),input.queue.as_ref().map(|q|&q.client_id),input.queue.as_ref().map(|q|q.revision),input.queue.as_ref().map(|q|q.index as i64),online_video])?;tx.commit()?;Ok(position)
    }).await?;
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
        json!({"id":sid,"url":url,"grant":grant,"mode":mode,"position":position,"duration":duration,"timeline_start":timeline_start,"video":source.video_codec().is_some(),"tracks":tracks,"subtitles":subtitles,"selected_subtitle":input.options.subtitle,"options":input.options,"probe":source.probe,"replay_gain":prefs.replay_gain}),
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
}
async fn session(state: &AppState, p: &Principal, id: &str) -> Result<Session> {
    let id = id.to_owned();
    let p = p.clone();
    state.db.call(move|db|Ok(db.query_row("SELECT COALESCE(media_id,'youtube:'||youtube_video_id),COALESCE(file_id,youtube_video_id),generation,mode,options,duration FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3 AND state IN ('ready','playing','paused') AND updated_at>?4",params![id,p.user.id,p.session_id,now()-120],|r|Ok(Session {media:r.get(0)?,file:r.get(1)?,generation:r.get(2)?,mode:r.get(3)?,options:serde_json::from_str(&r.get::<_,String>(4)?).unwrap(),duration:r.get(5)?})).optional()?)).await?.ok_or_else(ApiError::not_found)
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
pub async fn hls(
    State(state): State<AppState>,
    Path((id, revision, name)): Path<(String, String, String)>,
    Query(grant): Query<Grant>,
    request: Request,
) -> Result<Response> {
    let (_, session) = from_grant(&state, &id, &grant.grant).await?;
    current_source(&state, &session).await?;
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
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;tx.execute("UPDATE playback_sessions SET updated_at=?1 WHERE id=?2 AND state IN ('ready','playing','paused')",params![now(),id])?;tx.execute("UPDATE playback_grants SET expires_at=?1 WHERE resource=?2 AND expires_at>?3",params![now()+120,format!("playback:{id}"),now()])?;tx.commit()?;Ok(())}).await?;
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
    state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE playback_sessions SET state='stopped',updated_at=?1 WHERE id=?2",
                params![now(), key],
            )?;
            tx.execute(
                "DELETE FROM playback_grants WHERE resource=?1",
                [format!("playback:{key}")],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
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
    let src = current_source(&state, &session).await?;
    let grant = grants::issue(&state, &p, &format!("playback:{id}"), 120).await?;
    let mut timeline_start = 0.0;
    let url = if session.mode == "direct" {
        format!("/api/v1/playback/{id}/stream?grant={grant}")
    } else {
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
    if input.sequence < 0
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
    let event_state = input.state.clone();
    let updated=state.db.call(move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT COALESCE(media_id,'youtube:'||youtube_video_id),edition,duration,sequence,state FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3",params![key,user,auth],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,f64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))).optional()?;
        let Some((media,edition,duration,sequence,status))=row else {return Ok(None);};
        if input.sequence<=sequence || ["stopped","failed"].contains(&status.as_str()) {return Ok(Some(false));}
        let position=input.position.min(duration);
        tx.execute("UPDATE playback_sessions SET position=?1,sequence=?2,state=?3,updated_at=?4 WHERE id=?5",params![position,input.sequence,input.state,now(),key])?;
        if let Some(video)=media.strip_prefix("youtube:") {
            crate::online::downloads::record(&tx,&key,&user,video,position,duration,&input.state)?;
        } else {
        tx.execute("INSERT INTO edition_progress VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,media_id,edition) DO UPDATE SET position=excluded.position,duration=excluded.duration,updated_at=excluded.updated_at",params![user,media,edition,position,duration,now()])?;
        tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=MAX(watched,excluded.watched),updated_at=excluded.updated_at",params![user,media,position>=duration*0.9,now()])?;
        crate::history::record(&tx,&key,position,&input.state)?;
        }
        let resource=format!("playback:{key}");
        if stopped {tx.execute("DELETE FROM playback_grants WHERE resource=?1",[resource])?;} else {tx.execute("UPDATE playback_grants SET expires_at=?1 WHERE resource=?2 AND expires_at>?3",params![now()+120,resource,now()])?;}
        tx.commit()?;Ok(Some(true))
    }).await?.ok_or_else(ApiError::not_found)?;
    if updated {
        if stopped {
            state.playback.stop(id).await;
        }
        state
            .emit(
                Some(p.user.id.clone()),
                "playback.changed",
                json!({"id":id,"state":event_state}),
            )
            .await?;
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
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE playback_sessions SET state='failed',updated_at=?1 WHERE id=?2",
                params![now(), id],
            )?;
            db.execute(
                "DELETE FROM playback_grants WHERE resource=?1",
                [format!("playback:{id}")],
            )?;
            Ok(())
        })
        .await
}
pub async fn maintain(state: AppState) -> anyhow::Result<()> {
    // Process state cannot survive a server restart, but resume records do.
    state.db.call(|db|{db.execute("UPDATE playback_sessions SET state='stopped' WHERE state IN ('ready','playing','paused')",[])?;crate::history::finish_stale(db)?;Ok(())}).await?;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let active=state.db.call(|db| {
            db.execute("UPDATE playback_sessions SET state='stopped' WHERE state IN ('ready','playing','paused') AND updated_at<=?1",[now()-120])?;
            crate::history::finish_stale(db)?;
            Ok(db.prepare("SELECT p.id FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.state IN ('ready','playing','paused') AND s.expires_at>?1")?.query_map([now()],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
        }).await?;
        for id in state.playback.maintain(&active).await? {
            fail(&state, &id).await?;
        }
    }
}
