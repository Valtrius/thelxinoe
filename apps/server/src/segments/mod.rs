//! Shared file-generation-specific segments for first-party and Jellyfin clients.

#[path = "../storage/segments.rs"]
mod storage;

mod analysis;
mod fingerprint;
#[cfg(test)]
mod tests;
use crate::{
    AppState,
    error::{ApiError, Result},
    playback, security,
};
pub use analysis::run;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, Principal, id, now};
const KINDS: [&str; 4] = ["Intro", "Recap", "Credits", "Preview"];

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/catalog/{id}/segments", get(list).put(save))
        .route("/api/v1/catalog/{id}/segments/analyze", post(reanalyze))
        .route(
            "/api/v1/me/segments",
            get(preferences).put(save_preferences),
        )
        .route("/api/v1/admin/segments", get(status).put(configure))
}
#[derive(Serialize, Deserialize, Clone)]
pub(super) struct Segment {
    #[serde(default)]
    pub id: String,
    pub kind: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub confidence: f64,
}
#[derive(Deserialize, Default)]
struct Selection {
    file_id: Option<String>,
}
pub(crate) async fn resolved(
    state: &AppState,
    media: &str,
    file: Option<&str>,
) -> Result<(String, String, Vec<Segment>)> {
    let src = playback::source(state, media, file).await?;
    let media = media.to_owned();
    let file = src.id.clone();
    let generation = src.generation.clone();
    let items = storage::resolved(media, file, generation, &state.db).await?;
    Ok((src.id, src.generation, items))
}
async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Query(selection): Query<Selection>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let (file, generation, items) = resolved(&state, &media, selection.file_id.as_deref()).await?;
    Ok(Json(
        json!({"file_id":file,"generation":generation,"items":items,"preferences":preferences_for(&state,&p).await?}),
    ))
}
pub(crate) async fn for_jellyfin(
    state: &AppState,
    p: &Principal,
    media: &str,
) -> Result<Vec<Segment>> {
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let mid = media.to_owned();
    let (file, count) = storage::for_jellyfin(user, auth, mid, &state.db).await?;
    // The standard endpoint has no edition parameter. Never guess between editions.
    if file.is_none() && count > 1 {
        return Ok(vec![]);
    }
    Ok(resolved(state, media, file.as_deref()).await?.2)
}
pub(crate) async fn preferences_for(state: &AppState, p: &Principal) -> Result<Value> {
    let user = p.user.id.clone();
    Ok(storage::preferences_for(user, &state.db)
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({"Intro":"Ask","Recap":"Ask","Credits":"Ask","Preview":"Ask"})))
}
async fn preferences(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(preferences_for(&state, &p).await?))
}
async fn save_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if input.as_object().is_none_or(|o| o.len() != 4)
        || KINDS
            .iter()
            .any(|k| !matches!(input[*k].as_str(), Some("Auto" | "Ask" | "Ignore")))
    {
        return Err(ApiError::bad(
            "Choose Auto, Ask or Ignore for every segment type",
        ));
    }
    storage::save_preferences(&state.db, input, p).await?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Edit {
    file_id: String,
    generation: String,
    items: Vec<Segment>,
    #[serde(default)]
    reset: bool,
}
async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let _lease = state.media_operations.write().await;
    let src = playback::source(&state, &media, Some(&input.file_id)).await?;
    if src.generation != input.generation {
        return Err(ApiError::conflict(
            "Media was replaced; reload its segments",
        ));
    }
    if input.items.len() > 40 || input.items.iter().any(|s| !valid(s, src.duration())) {
        return Err(ApiError::bad(
            "Segments need a supported type and a valid start/end within this file",
        ));
    }
    let event_media = media.clone();
    storage::save(&state.db, media, input, p, src).await?;
    state
        .emit(None, "segments.changed", json!({"media_id":event_media}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
fn valid(s: &Segment, duration: f64) -> bool {
    KINDS.contains(&s.kind.as_str())
        && s.start.is_finite()
        && s.end.is_finite()
        && s.start >= 0.0
        && s.end > s.start
        && s.end <= duration
}
use storage::insert;
async fn reanalyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Query(selection): Query<Selection>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    let src = playback::source(&state, &media, selection.file_id.as_deref()).await?;
    let mid = media.clone();
    let episode = storage::reanalyze_read_media(&state.db, mid).await?;
    if !episode {
        return Err(ApiError::bad(
            "Automatic analysis is available for episodes",
        ));
    }
    storage::reanalyze_write_segment_analysis(&state.db, media, src).await?;
    Ok(Json(json!({"queued":true})))
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
    local: bool,
    external: bool,
}
async fn config(state: &AppState) -> Result<Config> {
    Ok(storage::config(&state.db).await?)
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Config>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    storage::configure(&state.db, input).await?;
    Ok(Json(json!({"saved":true})))
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let items = storage::status(&state.db).await?;
    Ok(Json(json!({"config":config(&state).await?,"items":items})))
}
