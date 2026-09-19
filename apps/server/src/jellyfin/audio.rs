//! Universal audio URLs do not carry a playback id in several Jellyfin clients.
//! Keep the association private to the authenticated device and logical track.
use super::{Query, canonical};
use crate::{
    AppState,
    error::{ApiError, Result},
    playback as core,
};
use axum::{extract::Request, response::Response};
use rusqlite::{OptionalExtension, params};
use thelxinoe_core::Principal;
use thelxinoe_playback::{Capabilities, Options};

pub async fn session(state: &AppState, p: &Principal, media: &str) -> Result<Option<String>> {
    let auth = p.session_id.clone();
    let user = p.user.id.clone();
    let media = media.to_owned();
    Ok(state.db.call(move|db|Ok(db.query_row("SELECT a.playback_id FROM compat_audio_playbacks a JOIN playback_sessions s ON s.id=a.playback_id JOIN media m ON m.id=a.media_id WHERE a.auth_session_id=?1 AND a.media_id=?2 AND s.user_id=?3 AND m.kind='track'",params![auth,media,user],|r|r.get(0)).optional()?)).await?)
}
pub async fn stream(
    state: AppState,
    p: &Principal,
    media: &str,
    mut q: Query,
    request: Request,
) -> Result<Response> {
    let guard = state.compatibility_audio.lock().await;
    let file = q.get("mediasourceid").map(|v| canonical(v));
    let source = core::source(&state, media, file.as_deref()).await?;
    let mid = media.to_owned();
    let track = state
        .db
        .call(move |db| {
            Ok(
                db.query_row("SELECT kind='track' FROM media WHERE id=?1", [mid], |r| {
                    r.get::<_, bool>(0)
                })?,
            )
        })
        .await?;
    if !track || source.video_codec().is_some() {
        return Err(ApiError::bad("Universal audio requires an audio track"));
    }
    let audio = source
        .tracks()
        .await?
        .into_iter()
        .find(|t| t.kind == "audio")
        .ok_or_else(ApiError::not_found)?;
    let container = source
        .path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let supported = q.get("container").is_none_or(|list| {
        list.split(',').any(|entry| {
            let (format, codec) = entry.split_once('|').unwrap_or((entry, ""));
            (format.eq_ignore_ascii_case(&container) || format.eq_ignore_ascii_case(&audio.codec))
                && (codec.is_empty() || codec.eq_ignore_ascii_case(&audio.codec))
        })
    });
    let bitrate = source.probe["format"]["bit_rate"]
        .as_str()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(source.size as f64 * 8.0 / source.duration());
    let channels = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["index"].as_i64() == audio.index)
        .and_then(|s| s["channels"].as_u64())
        .unwrap_or(2);
    if !supported
        || q.get("maxstreamingbitrate")
            .and_then(|v| v.parse::<f64>().ok())
            .is_some_and(|max| max > 0.0 && bitrate > max)
        || q.get("maxaudiochannels")
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|max| channels > max)
    {
        return Err(ApiError::bad(
            "This audio format requires conversion through PlaybackInfo",
        ));
    }
    let mut id = session(&state, p, media).await?;
    if let Some(existing) = &id {
        let key = existing.clone();
        let current = state
            .db
            .call(move |db| {
                Ok(db.query_row(
                    "SELECT state,file_id FROM playback_sessions WHERE id=?1",
                    [key],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?)
            })
            .await?;
        if !["ready", "playing", "paused"].contains(&current.0.as_str()) {
            id = None;
        } else if current.1 != source.id {
            return Err(ApiError::conflict(
                "Stop the current track before changing its source",
            ));
        }
    }
    let id = if let Some(id) = id {
        id
    } else {
        let created = core::create_with_delivery(
            &state,
            p,
            core::Create {
                queue: None,
                media_id: media.into(),
                file_id: Some(source.id),
                position: Some(0.0),
                options: Options {
                    quality: "original".into(),
                    audio: audio.index,
                    subtitle: Some("off".into()),
                    capabilities: Capabilities {
                        containers: vec![container],
                        audio: vec![audio.codec],
                        native_tracks: true,
                        ..Default::default()
                    },
                },
            },
            true,
        )
        .await?;
        let id = created["id"].as_str().unwrap().to_owned();
        let key = id.clone();
        let auth = p.session_id.clone();
        let media = media.to_owned();
        state.db.call(move|db| {
            let tx=db.transaction()?;
            tx.execute("INSERT INTO compat_playbacks(playback_id) VALUES (?1)",[&key])?;
            tx.execute("INSERT INTO compat_audio_playbacks VALUES (?1,?2,?3) ON CONFLICT(auth_session_id,media_id) DO UPDATE SET playback_id=excluded.playback_id",params![auth,media,key])?;
            tx.commit()?;Ok(())
        }).await?;
        id
    };
    q.insert("playsessionid".into(), id);
    drop(guard);
    super::playback::stream(state, p, media, &q, request).await
}
