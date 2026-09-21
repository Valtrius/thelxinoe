use crate::{
    AppState,
    error::{ApiError, Result},
    security, statistics,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Capability, now};

pub(crate) fn record(
    tx: &rusqlite::Transaction<'_>,
    playback: &str,
    position: f64,
    state: &str,
    seconds: f64,
) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state)
         SELECT p.id,p.user_id,
                CASE
                  WHEN p.media_id IS NOT NULL THEN CASE c.kind WHEN 'movie' THEN 'movies' WHEN 'episode' THEN 'shows' WHEN 'track' THEN 'music' END
                  WHEN p.youtube_video_id IS NOT NULL THEN 'youtube'
                  WHEN p.live_media_id LIKE 'twitch:%' THEN 'twitch'
                  WHEN p.live_media_id LIKE 'kick:%' THEN 'kick'
                END,
                COALESCE(p.media_id,'youtube:'||p.youtube_video_id,p.live_media_id),
                COALESCE(c.title,y.title,l.title),
                CASE
                  WHEN p.media_id IS NOT NULL THEN c.kind
                  WHEN p.youtube_video_id IS NOT NULL THEN
                    CASE
                      WHEN y.broadcast='live' THEN 'live'
                      WHEN y.broadcast='upcoming' THEN 'upcoming'
                      WHEN y.is_short=1 THEN 'short'
                      WHEN y.broadcast='replay' THEN 'live_replay'
                      ELSE 'upload'
                    END
                  ELSE 'live'
                END,
                p.edition,s.name,?2,?2,CASE WHEN ?4='stopped' THEN ?2 ELSE NULL END,?3,p.duration,?5,?4
         FROM playback_sessions p
         JOIN sessions s ON s.id=p.auth_session_id
         LEFT JOIN media_cards c ON c.id=p.media_id
         LEFT JOIN youtube_videos y ON y.user_id=p.user_id AND y.video_id=p.youtube_video_id
         LEFT JOIN live_media l ON l.id=p.live_media_id
         WHERE p.id=?1 AND COALESCE(c.title,y.title,l.title) IS NOT NULL
         ON CONFLICT(playback_id,platform) DO UPDATE SET
           updated_at=excluded.updated_at,
           ended_at=excluded.ended_at,
           position=excluded.position,
           played_seconds=playback_history.played_seconds+excluded.played_seconds,
           state=excluded.state",
        params![playback, now(), position, state, seconds],
    )?;
    // A prefetched session does not advance the saved queue until its own first
    // progress report. A late stop from an earlier item cannot rewind it.
    tx.execute("UPDATE music_queues SET current_index=p.queue_index,position=?2,updated_at=?3,completed=(p.queue_index=json_array_length(music_queues.items)-1 AND ?2>=p.duration*0.99) FROM playback_sessions p WHERE p.id=?1 AND music_queues.user_id=p.user_id AND music_queues.client_id=p.client_id AND music_queues.revision=p.queue_revision AND music_queues.current_index<=p.queue_index",params![playback,position,now()])?;
    Ok(())
}
pub(crate) fn finish_stale(db: &rusqlite::Connection) -> anyhow::Result<()> {
    db.execute(
        "UPDATE playback_history SET ended_at=updated_at,state='stopped'
         WHERE ended_at IS NULL
           AND NOT EXISTS(
             SELECT 1
             FROM playback_sessions p
             JOIN sessions s ON s.id=p.auth_session_id
             WHERE p.id=playback_history.playback_id
               AND p.state IN ('playing','paused')
               AND s.expires_at>?1
               AND p.updated_at>?2
               AND (
                 (playback_history.platform='youtube' AND 'youtube:'||p.youtube_video_id=playback_history.media_id)
                 OR (playback_history.platform IN ('twitch','kick') AND p.live_media_id=playback_history.media_id)
                 OR (playback_history.platform IN ('movies','shows','music') AND p.media_id=playback_history.media_id)
               )
           )",
        params![now(), now() - 120],
    )?;
    Ok(())
}
pub(crate) fn delete_provider(
    tx: &rusqlite::Transaction<'_>,
    user: &str,
    platform: &str,
) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM playback_history WHERE user_id=?1 AND platform=?2",
        params![user, platform],
    )?;
    Ok(())
}
#[derive(Deserialize, Default)]
pub struct Filter {
    before: Option<String>,
    #[serde(flatten)]
    scope: statistics::Filter,
}
#[derive(Deserialize, Default)]
pub struct AuditFilter {
    before: Option<i64>,
}
pub async fn mine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    history(&state, Some(p.user.id), &p.user.timezone, filter)
        .await
        .map(Json)
}
pub async fn admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::InspectHistory).await?;
    history(&state, filter.scope.user.clone(), &p.user.timezone, filter)
        .await
        .map(Json)
}
async fn history(
    state: &AppState,
    user: Option<String>,
    zone: &str,
    filter: Filter,
) -> Result<Value> {
    let range = filter.scope.range.unwrap_or_else(|| "30d".into());
    let platform = filter.scope.platform.unwrap_or_else(|| "all".into());
    statistics::validate_scope(&range, &platform)?;
    let zone = zone
        .parse()
        .map_err(|_| ApiError::bad("Unknown display timezone"))?;
    let now = chrono::Utc::now();
    let until = now.timestamp();
    let cutoff = statistics::range_cutoff(zone, &range, now);
    let before = filter
        .before
        .as_deref()
        .map(serde_json::from_str::<(i64, i64)>)
        .transpose()
        .map_err(|_| ApiError::bad("Invalid history cursor"))?;
    Ok(state.db.call(move|db|{
        let before_started=before.as_ref().map(|cursor|cursor.0);
        let before_id=before.as_ref().map(|cursor|cursor.1);
        let items=db.prepare("SELECT h.id,h.media_id,CASE h.platform WHEN 'movies' THEN 'movie' WHEN 'shows' THEN 'episode' WHEN 'music' THEN 'track' ELSE h.platform END,h.media_title,h.edition,u.id,u.username,h.device_name,h.started_at,h.updated_at,h.ended_at,h.position,h.duration,h.played_seconds,h.state FROM playback_history h JOIN users u ON u.id=h.user_id WHERE (?1 IS NULL OR h.user_id=?1) AND (?2='all' OR h.platform=?2) AND (?3 IS NULL OR h.started_at>=?3) AND h.started_at<=?4 AND (?5 IS NULL OR h.started_at<?5 OR (h.started_at=?5 AND h.id<?6)) ORDER BY h.started_at DESC,h.id DESC LIMIT 100")?.query_map(params![user,platform,cutoff,until,before_started,before_id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"media_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"edition":r.get::<_,String>(4)?,"user_id":r.get::<_,String>(5)?,"username":r.get::<_,String>(6)?,"device":r.get::<_,String>(7)?,"started_at":r.get::<_,i64>(8)?,"updated_at":r.get::<_,i64>(9)?,"ended_at":r.get::<_,Option<i64>>(10)?,"position":r.get::<_,f64>(11)?,"duration":r.get::<_,f64>(12)?,"played_seconds":r.get::<_,f64>(13)?,"state":r.get::<_,String>(14)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let next_before=items.last().filter(|_|items.len()==100).map(|item|json!([item["started_at"].clone(),item["id"].clone()]).to_string());
        Ok(json!({"items":items,"next_before":next_before}))
    }).await?)
}
pub async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<AuditFilter>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let items=state.db.call(move|db|Ok(db.prepare("SELECT a.id,a.action,a.target,a.created_at,u.username FROM audit a LEFT JOIN users u ON u.id=a.actor_id WHERE (?1 IS NULL OR a.id<?1) ORDER BY a.id DESC LIMIT 100")?.query_map([filter.before],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"action":r.get::<_,String>(1)?,"target":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"actor":r.get::<_,Option<String>>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?)).await?;
    Ok(Json(
        json!({"next_before":items.last().map(|i|i["id"].clone()).filter(|_|items.len()==100),"items":items}),
    ))
}

#[cfg(test)]
mod tests {
    use crate::online::oauth::tests::{call, fixture};
    use serde_json::Value;
    #[tokio::test]
    async fn history_uses_shared_platform_range_and_private_scope() {
        let (_temp, state, alice) = fixture().await;
        let now = thelxinoe_core::now();
        state.db.call(move |db| {
            for (id,user,platform,media,time) in [
                ("recent","alice","twitch","twitch:recent",now-60),
                ("old","alice","twitch","twitch:old",now-8*86_400),
                ("other-user","bob","twitch","twitch:other",now-60),
                ("kick","alice","kick","kick:channel",now-60),
            ] {
                db.execute("INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES (?1,?2,?3,?4,'Stream','live','live','Browser',?5,?5,?5,20,0,20,'stopped')",rusqlite::params![id,user,platform,media,time])?;
            }
            Ok(())
        }).await.unwrap();
        let response = call(
            &state,
            "/api/v1/me/history?platform=twitch&range=7d&user=bob",
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(response.0, axum::http::StatusCode::OK);
        assert_eq!(response.2["items"].as_array().unwrap().len(), 1);
        assert_eq!(response.2["items"][0]["media_id"], "twitch:recent");
        assert!(response.2.get("stats").is_none());
        assert!(response.2.get("users").is_none());
        assert!(response.2.get("daily").is_none());
        assert!(response.2.get("top").is_none());
    }
    #[tokio::test]
    async fn combined_history_merges_sources_and_uses_a_stable_cursor() {
        let (_temp, state, alice) = fixture().await;
        state
            .db
            .call(|db| {
                for i in 0..99 {
                    db.execute(
                        "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES (?1,'alice','youtube',?2,?3,'upload','public','Browser',?4,?4,?4,30,60,20,'stopped')",
                        rusqlite::params![
                            format!("youtube-{i:03}"),
                            format!("youtube:video-{i:03}"),
                            format!("Video {i:03}"),
                            2_000 + i
                        ],
                    )?;
                }
                db.execute(
                    "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES ('shared','alice','youtube','youtube:shared-video','Shared video','upload','public','Browser',1000,1000,1000,30,60,20,'stopped')",
                    [],
                )?;
                db.execute(
                    "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES ('shared','alice','twitch','twitch:shared','Shared stream','live','live','Browser',1000,1000,1000,20,0,20,'stopped')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let first = call(
            &state,
            "/api/v1/me/history?platform=all&range=all",
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(first.0, axum::http::StatusCode::OK);
        assert_eq!(first.2["items"].as_array().unwrap().len(), 100);
        assert_eq!(first.2["items"][0]["kind"], "youtube");
        let before = url::form_urlencoded::byte_serialize(
            first.2["next_before"].as_str().unwrap().as_bytes(),
        )
        .collect::<String>();
        let second = call(
            &state,
            &format!("/api/v1/me/history?platform=all&range=all&before={before}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(second.0, axum::http::StatusCode::OK);
        assert_eq!(second.2["items"].as_array().unwrap().len(), 1);
        assert_eq!(second.2["items"][0]["media_id"], "youtube:shared-video");
        assert!(second.2["next_before"].is_null());
    }
}
