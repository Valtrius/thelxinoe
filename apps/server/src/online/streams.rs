use super::{downloads, extract, live};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use thelxinoe_core::{Principal, now};
use thelxinoe_playback::{Options, RemoteSource};
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
        Prepared {
            source: live::extract(state, video).await?,
            duration: 0.0,
            expires: now() + 60,
            native: false,
        }
    } else {
        let metadata = extract::metadata(state, video).await?;
        select(&metadata)?
    };
    let mut cached = state.online.streams.prepared.lock().await;
    if cached.len() >= 64 {
        cached.clear();
    }
    cached.insert(video.into(), prepared.clone());
    Ok(prepared)
}
fn select(metadata: &Value) -> Result<Prepared> {
    let live = metadata["is_live"].as_bool().unwrap_or(false);
    let duration = metadata["duration"].as_f64().unwrap_or(0.0);
    if !live && (!duration.is_finite() || duration <= 0.0) {
        return Err(ApiError::conflict("This video is not ready for playback"));
    }
    let formats = metadata["formats"]
        .as_array()
        .ok_or_else(|| ApiError::conflict("No public media formats are available"))?;
    let videos = formats.iter().filter(|f| {
        f["vcodec"]
            .as_str()
            .is_some_and(|v| v.starts_with("avc1") || v == "h264")
            && f["height"].as_u64().is_some_and(|h| h <= 1080)
            && f["url"].is_string()
            && matches!(
                f["protocol"].as_str(),
                Some("https" | "m3u8_native" | "m3u8")
            )
    });
    let video = videos
        .max_by_key(|f| (f["height"].as_u64().unwrap_or(0), f["protocol"] == "https"))
        .ok_or_else(|| ApiError::conflict("No supported public video stream is available"))?;
    let audio = if video["acodec"].as_str().is_some_and(|v| v != "none") {
        None
    } else {
        Some(
            formats
                .iter()
                .filter(|f| {
                    f["vcodec"] == "none"
                        // Live audio HLS often omits the codec. FFmpeg probes
                        // the audio-only input and converts it to AAC.
                        && f["acodec"] != "none"
                        && matches!(
                            f["protocol"].as_str(),
                            Some("https" | "m3u8_native" | "m3u8")
                        )
                })
                .max_by(|a, b| {
                    a["abr"]
                        .as_f64()
                        .unwrap_or(0.0)
                        .total_cmp(&b["abr"].as_f64().unwrap_or(0.0))
                })
                .filter(|f| f["url"].is_string())
                .ok_or_else(|| {
                    ApiError::conflict("No supported public audio stream is available")
                })?,
        )
    };
    // Only finite, independently seekable files can be relayed to MPV. HLS
    // manifests and live sources retain the server's conversion pipeline.
    let native = !live
        && video["protocol"] == "https"
        && video["ext"] == "mp4"
        && audio.is_none_or(|f| {
            f["protocol"] == "https"
                && matches!(f["ext"].as_str(), Some("m4a" | "mp4" | "webm" | "opus"))
        });
    let source = RemoteSource {
        video: video["url"].as_str().unwrap().into(),
        audio: audio.map(|f| f["url"].as_str().unwrap().to_owned()),
        live,
    };
    source
        .validate()
        .map_err(|_| ApiError::conflict("The extractor returned an unsupported media address"))?;
    Ok(Prepared {
        source,
        duration: if live { 0.0 } else { duration },
        expires: now() + 300,
        native,
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
    let position = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",
                    params![owner, id],
                    |r| r.get::<_, f64>(0),
                )
                .optional()?
                .unwrap_or(0.0))
        })
        .await?;
    Ok(
        json!({"sources":[{"id":video,"edition":"public","duration":prepared.duration,"video":true,"tracks":[],"probe":{},"size":0}],"progress":[{"edition":"public","position":position,"duration":prepared.duration}],"watched":false,"live":prepared.source.live}),
    )
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
    if !options.capabilities.hls
        || !options.capabilities.video.iter().any(|v| v == "h264")
        || !options.capabilities.audio.iter().any(|v| v == "aac")
    {
        return Err(ApiError::conflict(
            "Streaming requires HLS with H.264 and AAC support",
        ));
    }
    if options.quality == "original" {
        return Err(ApiError::conflict(
            "Original quality requires a completed download; use Auto for streaming",
        ));
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
    let prepared = prepare(state, video).await?;
    let sid = thelxinoe_core::id();
    let native =
        prepared.native && options.capabilities.native_remote && options.quality == "auto" && !vod;
    let mode = if native { "direct" } else { "transcode" };
    let key = sid.clone();
    let owner = p.user.id.clone();
    let auth = p.session_id.clone();
    let video = video.to_owned();
    let input = options.clone();
    let duration = prepared.duration;
    let live = prepared.source.live;
    let start=state.db.call(move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(title)=live_title {
            anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM twitch_streams WHERE user_id=?1 AND 'twitch:'||channel_id=?2 AND active=1) OR EXISTS(SELECT 1 FROM kick_channels WHERE user_id=?1 AND 'kick:'||slug=?2)",params![owner,video],|r|r.get::<_,bool>(0))?,"Channel was removed before playback");
            tx.execute("INSERT INTO live_media VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET title=excluded.title",params![video,title])?;
            tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,live_media_id,streaming) VALUES (?1,?2,?3,?1,'live','ready','transcode',?4,0,0,?5,?5,?6,1)",params![key,owner,auth,serde_json::to_string(&input)?,now(),video])?;
            tx.commit()?;return Ok(0.0)
        }
        anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",params![owner,video],|r|r.get::<_,bool>(0))?,"Video was removed before playback");
        let saved=tx.query_row("SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",params![owner,video],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0);
        let start=if live {0.0}else {position.unwrap_or(if saved>=duration*0.9 {0.0}else{saved}).clamp(0.0,(duration-0.1).max(0.0))};
        tx.execute("INSERT INTO youtube_media(video_id) VALUES (?1) ON CONFLICT DO NOTHING",[&video])?;
        tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,youtube_video_id,streaming) VALUES (?1,?2,?3,?1,'public','ready',?9,?4,?5,?6,?7,?7,?8,1)",params![key,owner,auth,serde_json::to_string(&input)?,duration,start,now(),video,mode])?;
        tx.commit()?;Ok(start)
    }).await?;
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
        state
            .playback
            .start_remote_vod(&sid, &prepared.source, prepared.duration, &options)
            .await
    } else {
        state
            .playback
            .start_remote(&sid, &prepared.source, &options, start)
            .await
    };
    let (revision, offset) = match prepared_pipeline {
        Ok(value) => value,
        Err(_) => {
            state.online.streams.sessions.lock().await.remove(&sid);
            let id = sid.clone();
            state
                .db
                .call(move |db| {
                    db.execute(
                        "UPDATE playback_sessions SET state='failed' WHERE id=?1",
                        [id],
                    )?;
                    Ok(())
                })
                .await?;
            return Err(ApiError::conflict(
                "The server could not start the public stream; try again later",
            ));
        }
    };
    let key = sid.clone();
    let valid=state.db.call(move |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=?1 AND p.state='ready' AND s.expires_at>?2 AND (EXISTS(SELECT 1 FROM youtube_videos v WHERE v.user_id=p.user_id AND v.video_id=p.youtube_video_id) OR EXISTS(SELECT 1 FROM twitch_streams t WHERE t.user_id=p.user_id AND 'twitch:'||t.channel_id=p.live_media_id AND t.active=1) OR EXISTS(SELECT 1 FROM kick_channels k WHERE k.user_id=p.user_id AND 'kick:'||k.slug=p.live_media_id)))",params![key,now()],|r|r.get::<_,bool>(0))?)).await?;
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
        json!({"id":sid,"url":url,"external_audio":external_audio,"grant":grant,"mode":mode,"position":start,"duration":prepared.duration,"timeline_start":offset,"video":true,"live":live,"tracks":[],"subtitles":[],"selected_subtitle":null,"options":options,"probe":{},"replay_gain":"off"}),
    )
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
    state
        .playback
        .start_remote(id, &source.source, options, position)
        .await
        .map_err(|_| {
            ApiError::conflict("Could not seek the public stream; reopen it to refresh its address")
        })
}
pub(crate) async fn maintain(state: &AppState, active: &[String]) {
    state
        .online
        .streams
        .sessions
        .lock()
        .await
        .retain(|id, _| active.contains(id));
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
    use crate::online::oauth::tests::{call, fixture};
    use axum::http::StatusCode;
    fn metadata(live: bool) -> Value {
        json!({"is_live":live,"duration":100,"formats":[{"vcodec":"avc1.640028","acodec":"none","height":1080,"protocol":"https","ext":"mp4","url":"https://r1.googlevideo.com/video?signature=private"},{"vcodec":"none","acodec":"mp4a.40.2","abr":128,"protocol":"https","ext":"m4a","url":"https://r1.googlevideo.com/audio?signature=private"}]})
    }
    #[test]
    fn selection_handles_separate_streams_and_live_without_exposing_urls() {
        let vod = select(&metadata(false)).unwrap();
        assert_eq!(vod.duration, 100.0);
        assert!(vod.source.audio.is_some());
        assert!(vod.native);
        assert!(!select(&metadata(true)).unwrap().native);
        assert_eq!(select(&metadata(true)).unwrap().duration, 0.0);
        let mut live = metadata(true);
        live["formats"][1]["acodec"] = Value::Null;
        live["formats"][1]["protocol"] = json!("m3u8_native");
        assert!(select(&live).unwrap().source.audio.is_some());
        let mut manifest = metadata(false);
        manifest["formats"][0]["protocol"] = json!("m3u8_native");
        assert!(!select(&manifest).unwrap().native);
        let mut audio_manifest = metadata(false);
        audio_manifest["formats"][1]["protocol"] = json!("m3u8_native");
        assert!(!select(&audio_manifest).unwrap().native);
        let mut malformed = metadata(false);
        malformed["formats"][0]["url"] = json!("https://localhost/private");
        assert!(select(&malformed).is_err());
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
            .insert(video.into(), select(&metadata(false)).unwrap());
        let input = json!({"media_id":"youtube:abcdefghijk","position":20,"options":{"quality":"auto","audio":null,"subtitle":null,"capabilities":{"video":["h264"],"audio":["aac"],"containers":[],"hls":true,"native_remote":true}}});
        assert_eq!(
            call(&state, "/api/v1/playback", "POST", input.clone(), &cookie)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        state.db.call(move|db| { db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice',?1,'Public fixture')",[video])?; Ok(()) }).await.unwrap();
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
            .call(move |db| {
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
            .insert(video.into(), select(&metadata(true)).unwrap());
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
        state.db.call(move|db|{db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice',?1,'Live fixture')",[video])?;db.execute("INSERT INTO youtube_media VALUES (?1)",[video])?;db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id,streaming) SELECT 'live','alice',id,'live','public','playing','transcode','{}',0,?1,?1,?2,1 FROM sessions WHERE user_id='alice' LIMIT 1",params![now(),video])?;Ok(())}).await.unwrap();
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
            .call(|db| {
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
        let token = thelxinoe_auth::issue_session(
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
