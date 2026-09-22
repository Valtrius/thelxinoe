#[path = "../storage/online/presentation.rs"]
mod storage;

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
    storage::cancel(&state.db, provider, p).await?;
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
    let changed = storage::refresh(&state.db, provider, p).await?;
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
    let result = storage::resolve(&state.db, p, id).await?;
    if !result {
        return Err(ApiError::conflict(
            "Too many videos are waiting for metadata",
        ));
    }
    Ok(Json(json!({"ready":true})))
}
