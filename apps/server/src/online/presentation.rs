use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Result<Json<Value>> {
    if !["youtube", "twitch"].contains(&provider.as_str()) {
        return Err(ApiError::not_found());
    }
    let p = security::principal(&state, &headers).await?;
    state.db.call(move|db|{
        if provider=="youtube" {db.execute("DELETE FROM oauth_attempts WHERE user_id=?1 AND session_id=?2 AND provider='youtube'",params![p.user.id,p.session_id])?;}
        else {db.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND session_id=?2",params![p.user.id,p.session_id])?;}
        Ok(())
    }).await?;
    Ok(Json(json!({"cancelled":true})))
}
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Result<Json<Value>> {
    if !["twitch", "kick"].contains(&provider.as_str()) {
        return Err(ApiError::not_found());
    }
    let p = security::principal(&state, &headers).await?;
    let changed=state.db.call(move|db|{
        if provider=="twitch" {Ok(db.execute("UPDATE twitch_sync SET next_run=?1 WHERE user_id=?2 AND failures=0 AND (last_complete IS NULL OR last_complete<?1-30) AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=twitch_sync.user_id AND a.provider='twitch' AND a.status='connected' AND a.generation=twitch_sync.generation)",params![now(),p.user.id])?)}
        else {Ok(db.execute("UPDATE kick_channels SET next_run=?1 WHERE user_id=?2 AND failures=0 AND updated_at<?1-30 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=kick_channels.user_id AND a.provider='kick' AND a.status='connected')",params![now(),p.user.id])?)}
    }).await?;
    if changed == 0 {
        return Err(ApiError::conflict(
            "Connect or track a channel first, or wait for the current retry delay and 30-second refresh cooldown",
        ));
    }
    Ok(Json(json!({"queued":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolve {
    video_id: String,
}
pub async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Resolve>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let id = super::feed::video_id(&input.video_id)
        .ok_or_else(|| ApiError::bad("Enter a YouTube video URL or ID"))?;
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count=tx.query_row("SELECT COUNT(*) FROM youtube_videos WHERE user_id=?1 AND metadata_at=0",[&p.user.id],|r|r.get::<_,i64>(0))?;
        if count>=1000{return Ok(false);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,?2,?2) ON CONFLICT DO NOTHING",params![p.user.id,id])?;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),p.user.id])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !result {
        return Err(ApiError::conflict(
            "Too many videos are waiting for metadata",
        ));
    }
    Ok(Json(json!({"ready":true})))
}
