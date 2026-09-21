use crate::{
    AppState,
    error::{ApiError, Result},
    security,
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
    tx.execute("INSERT INTO playback_history(playback_id,user_id,media_id,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) SELECT p.id,p.user_id,p.media_id,p.edition,s.name,?2,?2,CASE WHEN ?4='stopped' THEN ?2 ELSE NULL END,?3,p.duration,?5,?4 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=?1 ON CONFLICT(playback_id) DO UPDATE SET updated_at=excluded.updated_at,ended_at=excluded.ended_at,position=excluded.position,played_seconds=played_seconds+?5,state=excluded.state",params![playback,now(),position,state,seconds])?;
    // A prefetched session does not advance the saved queue until its own first
    // progress report. A late stop from an earlier item cannot rewind it.
    tx.execute("UPDATE music_queues SET current_index=p.queue_index,position=?2,updated_at=?3,completed=(p.queue_index=json_array_length(music_queues.items)-1 AND ?2>=p.duration*0.99) FROM playback_sessions p WHERE p.id=?1 AND music_queues.user_id=p.user_id AND music_queues.client_id=p.client_id AND music_queues.revision=p.queue_revision AND music_queues.current_index<=p.queue_index",params![playback,position,now()])?;
    Ok(())
}
pub(crate) fn finish_stale(db: &rusqlite::Connection) -> anyhow::Result<()> {
    db.execute("UPDATE playback_history SET ended_at=updated_at,state='stopped' WHERE ended_at IS NULL AND NOT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=playback_id AND p.state IN ('playing','paused') AND s.expires_at>?1 AND p.updated_at>?2)",params![now(),now()-120])?;
    db.execute("UPDATE youtube_history SET state='stopped' WHERE state IN ('playing','paused') AND NOT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=playback_id AND p.state IN ('playing','paused') AND s.expires_at>?1 AND p.updated_at>?2)",params![now(),now()-120])?;
    db.execute("UPDATE live_history SET state='stopped' WHERE state IN ('playing','paused') AND NOT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=playback_id AND p.state IN ('playing','paused') AND s.expires_at>?1 AND p.updated_at>?2)",params![now(),now()-120])?;
    Ok(())
}
#[derive(Deserialize, Default)]
pub struct Filter {
    before: Option<i64>,
    since: Option<i64>,
    until: Option<i64>,
    user: Option<String>,
    domain: Option<String>,
}
pub async fn mine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    history(&state, Some(p.user.id), filter, false)
        .await
        .map(Json)
}
pub async fn admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::InspectHistory).await?;
    history(&state, filter.user.clone(), filter, true)
        .await
        .map(Json)
}
async fn history(
    state: &AppState,
    user: Option<String>,
    filter: Filter,
    admin: bool,
) -> Result<Value> {
    if filter.since.zip(filter.until).is_some_and(|(s, u)| s > u) {
        return Err(ApiError::bad("The start date must precede the end date"));
    }
    let source = match filter.domain.as_deref().unwrap_or("library") {
        "library" => {
            "(SELECT h.*,c.kind,c.title FROM playback_history h JOIN media_cards c ON c.id=h.media_id)"
        }
        "youtube" => {
            "(SELECT rowid AS id,playback_id,user_id,'youtube:'||video_id AS media_id,'youtube' AS kind,title,'public' AS edition,device_name,started_at,updated_at,CASE WHEN state='stopped' THEN updated_at ELSE NULL END AS ended_at,position,duration,played_seconds,state FROM youtube_history)"
        }
        "twitch" => {
            "(SELECT rowid AS id,playback_id,user_id,media_id,'twitch' AS kind,title,'live' AS edition,device_name,started_at,updated_at,CASE WHEN state='stopped' THEN updated_at ELSE NULL END AS ended_at,position,0 AS duration,played_seconds,state FROM live_history WHERE media_id LIKE 'twitch:%')"
        }
        "kick" => {
            "(SELECT rowid AS id,playback_id,user_id,media_id,'kick' AS kind,title,'live' AS edition,device_name,started_at,updated_at,CASE WHEN state='stopped' THEN updated_at ELSE NULL END AS ended_at,position,0 AS duration,played_seconds,state FROM live_history WHERE media_id LIKE 'kick:%')"
        }
        _ => return Err(ApiError::bad("Unknown history domain")),
    };
    Ok(state.db.call(move|db|{
        let items=db.prepare(&format!("SELECT h.id,h.media_id,h.kind,h.title,h.edition,u.id,u.username,h.device_name,h.started_at,h.updated_at,h.ended_at,h.position,h.duration,h.played_seconds,h.state FROM {source} h JOIN users u ON u.id=h.user_id WHERE (?1 IS NULL OR h.user_id=?1) AND (?2 IS NULL OR h.started_at>=?2) AND (?3 IS NULL OR h.started_at<?3) AND (?4 IS NULL OR h.id<?4) ORDER BY h.id DESC LIMIT 100"))?.query_map(params![user,filter.since,filter.until,filter.before],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"media_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"edition":r.get::<_,String>(4)?,"user_id":r.get::<_,String>(5)?,"username":r.get::<_,String>(6)?,"device":r.get::<_,String>(7)?,"started_at":r.get::<_,i64>(8)?,"updated_at":r.get::<_,i64>(9)?,"ended_at":r.get::<_,Option<i64>>(10)?,"position":r.get::<_,f64>(11)?,"duration":r.get::<_,f64>(12)?,"played_seconds":r.get::<_,f64>(13)?,"state":r.get::<_,String>(14)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let stats=db.query_row(&format!("SELECT COUNT(*),COALESCE(SUM(played_seconds),0),COUNT(DISTINCT media_id),COUNT(DISTINCT user_id) FROM {source} WHERE (?1 IS NULL OR user_id=?1) AND (?2 IS NULL OR started_at>=?2) AND (?3 IS NULL OR started_at<?3)"),params![user,filter.since,filter.until],|r|Ok(json!({"plays":r.get::<_,i64>(0)?,"played_seconds":r.get::<_,f64>(1)?,"media_count":r.get::<_,i64>(2)?,"user_count":r.get::<_,i64>(3)?})))?;
        let daily=db.prepare(&format!("SELECT strftime('%Y-%m-%d',started_at,'unixepoch'),COUNT(*),SUM(played_seconds) FROM {source} WHERE (?1 IS NULL OR user_id=?1) AND (?2 IS NULL OR started_at>=?2) AND (?3 IS NULL OR started_at<?3) GROUP BY 1 ORDER BY 1 DESC LIMIT 90"))?.query_map(params![user,filter.since,filter.until],|r|Ok(json!({"date":r.get::<_,String>(0)?,"plays":r.get::<_,i64>(1)?,"played_seconds":r.get::<_,f64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let top=db.prepare(&format!("SELECT title,COUNT(*),SUM(played_seconds) FROM {source} WHERE (?1 IS NULL OR user_id=?1) AND (?2 IS NULL OR started_at>=?2) AND (?3 IS NULL OR started_at<?3) GROUP BY media_id ORDER BY SUM(played_seconds) DESC LIMIT 10"))?.query_map(params![user,filter.since,filter.until],|r|Ok(json!({"title":r.get::<_,String>(0)?,"plays":r.get::<_,i64>(1)?,"played_seconds":r.get::<_,f64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let users=if admin{db.prepare(&format!("SELECT u.id,u.username,COUNT(*),SUM(h.played_seconds) FROM {source} h JOIN users u ON u.id=h.user_id WHERE (?1 IS NULL OR h.user_id=?1) AND (?2 IS NULL OR h.started_at>=?2) AND (?3 IS NULL OR h.started_at<?3) GROUP BY u.id ORDER BY SUM(h.played_seconds) DESC"))?.query_map(params![user,filter.since,filter.until],|r|Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"plays":r.get::<_,i64>(2)?,"played_seconds":r.get::<_,f64>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?}else{vec![]};
        Ok(json!({"items":items,"stats":stats,"users":users,"daily":daily,"top":top,"next_before":items.last().map(|i|i["id"].clone()).filter(|_|items.len()==100)}))
    }).await?)
}
pub async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
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
    async fn charts_follow_the_same_user_and_date_scope_as_history() {
        let (_temp, state, alice) = fixture().await;
        state.db.call(|db| {
            for (id,user,time,seconds) in [("a","alice",100,10),("b","alice",200,20),("c","bob",200,900)] {
                db.execute("INSERT INTO live_history VALUES (?1,?2,'twitch:42','Stream','Browser',?3,?3,?4,?4,'stopped')",rusqlite::params![id,user,time,seconds])?;
            }
            Ok(())
        }).await.unwrap();
        let response = call(
            &state,
            "/api/v1/me/history?domain=twitch&since=150&until=300&user=bob",
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(response.0, axum::http::StatusCode::OK);
        assert_eq!(response.2["stats"]["played_seconds"], 20.0);
        assert_eq!(response.2["daily"][0]["played_seconds"], 20.0);
        assert_eq!(response.2["top"][0]["plays"], 1);
        assert_eq!(response.2["users"], serde_json::json!([]));
    }
}
