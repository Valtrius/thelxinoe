//! Public live extraction and private statistics, using the shared playback lifecycle.
use crate::{
    AppState,
    error::{ApiError, Result},
};
use rusqlite::{OptionalExtension, params};
use thelxinoe_core::{Principal, now};
use thelxinoe_playback::RemoteSource;

pub(crate) fn domain(media: &str) -> bool {
    media.starts_with("twitch:") || media.starts_with("kick:")
}
pub(crate) async fn authorize(
    state: &AppState,
    p: &Principal,
    media: &str,
) -> Result<(String, String)> {
    if let Some(channel) = media.strip_prefix("kick:") {
        let channel = super::kick::slug(channel)?;
        let user = p.user.id.clone();
        return state.db.call(move|db|Ok(db.query_row("SELECT slug,slug||' - '||title FROM kick_channels WHERE user_id=?1 AND slug=?2",params![user,channel],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)).await?.ok_or_else(ApiError::not_found);
    }
    let channel = media
        .strip_prefix("twitch:")
        .ok_or_else(ApiError::not_found)?
        .to_owned();
    if channel.is_empty() || channel.len() > 64 || !channel.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::not_found());
    }
    let user = p.user.id.clone();
    state.db.call(move|db|Ok(db.query_row("SELECT login,display_name||' - '||title FROM twitch_streams WHERE user_id=?1 AND channel_id=?2 AND active=1",params![user,channel],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)).await?.ok_or_else(ApiError::not_found)
}
pub(crate) async fn extract(state: &AppState, media: &str) -> Result<RemoteSource> {
    let kick = media.starts_with("kick:");
    let login: String = if let Some(channel) = media.strip_prefix("kick:") {
        super::kick::slug(channel)?
    } else {
        let channel = media
            .strip_prefix("twitch:")
            .ok_or_else(ApiError::not_found)?
            .to_owned();
        state
            .db
            .call(move |db| {
                Ok(db
                    .query_row(
                        "SELECT login FROM twitch_streams WHERE channel_id=?1 AND active=1 LIMIT 1",
                        [channel],
                        |r| r.get(0),
                    )
                    .optional()?)
            })
            .await?
            .ok_or_else(ApiError::not_found)?
    };
    if login.is_empty()
        || login.len() > 100
        || !login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (kick && b == b'-'))
    {
        return Err(ApiError::bad("Invalid channel identity"));
    }
    let _permit = state
        .online
        .extraction
        .acquire()
        .await
        .map_err(|_| ApiError::conflict("Extraction is unavailable"))?;
    let address = state
        .online
        .streamlink
        .resolve(if kick { "kick" } else { "twitch" }, &login)
        .await
        .map_err(|_| {
            ApiError::conflict(
                "This channel is offline or public extraction is unavailable; retry later",
            )
        })?;
    let source = RemoteSource {
        video: address.trim().to_owned(),
        audio: None,
        live: true,
    };
    source
        .validate()
        .map_err(|_| ApiError::conflict("Unsupported public live media address"))?;
    let host = url::Url::parse(&source.video)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let domain = if kick { "live-video.net" } else { "ttvnw.net" };
    if host != domain && !host.ends_with(&format!(".{domain}")) {
        return Err(ApiError::conflict("Unsupported live media address"));
    }
    Ok(source)
}
pub(crate) fn record(
    tx: &rusqlite::Transaction<'_>,
    playback: &str,
    position: f64,
    status: &str,
    seconds: f64,
) -> anyhow::Result<()> {
    tx.execute("INSERT INTO live_history(playback_id,user_id,media_id,title,device_name,started_at,updated_at,position,played_seconds,state) SELECT p.id,p.user_id,p.live_media_id,m.title,s.name,?2,?2,?3,?4,?5 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id JOIN live_media m ON m.id=p.live_media_id WHERE p.id=?1 ON CONFLICT(playback_id) DO UPDATE SET updated_at=excluded.updated_at,position=excluded.position,played_seconds=played_seconds+?4,state=excluded.state",params![playback,now(),position,seconds,status])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::oauth::tests::{call, fixture};
    use axum::http::StatusCode;
    use serde_json::{Value, json};
    #[tokio::test]
    async fn live_progress_is_private_and_data_deletion_stops_only_own_sessions() {
        let (_temp, state, alice) = fixture().await;
        let bob =
            thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        let bob = format!("thelxinoe_session={bob}");
        state.db.call(|db|{
            db.execute("INSERT INTO live_media VALUES ('twitch:42','Live fixture')",[])?;
            for user in ["alice","bob"] {
                db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES (?1,'42','fixture','Fixture','Live fixture','Science',10,'today','s',1)",[user])?;
                db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,live_media_id,streaming) SELECT ?1,user_id,id,'g','live','playing','transcode','{}',0,?2,?2,'twitch:42',1 FROM sessions WHERE user_id=?1 LIMIT 1",params![user,now()])?;
            }Ok(())
        }).await.unwrap();
        assert_eq!(
            call(
                &state,
                "/api/v1/playback/alice/progress",
                "POST",
                json!({"state":"playing","position":10,"sequence":1}),
                &bob
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/playback/alice/progress",
                "POST",
                json!({"state":"playing","position":10,"sequence":1}),
                &alice
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/history?domain=twitch",
                "GET",
                Value::Null,
                &alice
            )
            .await
            .2["stats"]["plays"],
            1
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/history?domain=twitch&user=alice",
                "GET",
                Value::Null,
                &bob
            )
            .await
            .2["stats"]["plays"],
            0
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/online/twitch/data",
                "DELETE",
                json!({}),
                &alice
            )
            .await
            .0,
            StatusCode::OK
        );
        let rows = state
            .db
            .call(|db| {
                Ok(db
                    .prepare("SELECT id,state FROM playback_sessions ORDER BY id")?
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("alice".into(), "stopped".into()),
                ("bob".into(), "playing".into())
            ]
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/history?domain=twitch",
                "GET",
                Value::Null,
                &alice
            )
            .await
            .2["stats"]["plays"],
            0
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/catalog/twitch:42/playback",
                "GET",
                Value::Null,
                &alice
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
}
