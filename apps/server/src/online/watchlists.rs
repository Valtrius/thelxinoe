//! Private named lists. Membership determines aggregate retention.

#[path = "../storage/online/watchlists.rs"]
mod storage;

use super::{browse, downloads, feed};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub(crate) use storage::default_list;
async fn authorize(state: &AppState, user: &str, id: i64) -> Result<()> {
    let user = user.to_owned();
    if !storage::authorize(user, &state.db, id).await? {
        return Err(ApiError::not_found());
    }
    Ok(())
}
async fn changed(state: &AppState) -> Result<Json<Value>> {
    state.notify_events();
    Ok(Json(json!({"saved":true})))
}
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    let lists = storage::list(&state.db, p, grant).await?;
    Ok(Json(json!(lists)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub name: String,
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
}
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Create>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    create_for(&state, &p, input).await
}
pub(crate) async fn create_for(
    state: &AppState,
    p: &Principal,
    input: Create,
) -> Result<Json<Value>> {
    if !name_valid(&input.name) {
        return Err(ApiError::bad("Enter a watchlist name of 1–80 characters"));
    }
    let user = p.user.id.clone();
    let id = storage::create_for(user, &state.db, input)
        .await?
        .ok_or_else(|| ApiError::conflict("You can create up to 50 watchlists"))?;
    let _ = changed(state).await?;
    Ok(Json(json!(id)))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    name: String,
    auto_download: bool,
    auto_remove_watched: bool,
    sort_mode: String,
    sort_direction: String,
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Update>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    if !name_valid(&input.name)
        || !["manual", "date"].contains(&input.sort_mode.as_str())
        || !["asc", "desc"].contains(&input.sort_direction.as_str())
    {
        return Err(ApiError::bad("Invalid watchlist settings"));
    }
    if input.auto_download && !downloads::enabled(&state).await? {
        return Err(ApiError::conflict(
            "YouTube downloads are disabled by the administrator",
        ));
    }
    let user = p.user.id.clone();
    storage::update(&state.db, id, input, user).await?;
    changed(&state).await
}
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    let user = p.user.id.clone();
    let removed = storage::delete(&state.db, id, user).await?;
    if !removed {
        return Err(ApiError::conflict("Watch Later cannot be deleted"));
    }
    changed(&state).await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Add {
    pub video_id: String,
    pub manual_position: f64,
}
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Add>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    add_for(&state, &p, id, input).await
}
pub(crate) async fn add_for(
    state: &AppState,
    p: &Principal,
    id: i64,
    input: Add,
) -> Result<Json<Value>> {
    authorize(state, &p.user.id, id).await?;
    let video = feed::video_id(&input.video_id)
        .ok_or_else(|| ApiError::bad("Enter a YouTube video URL or ID"))?;
    if !input.manual_position.is_finite() || input.manual_position.abs() > 1e15 {
        return Err(ApiError::bad("Invalid watchlist position"));
    }
    let grant = grants::issue(state, p, "youtube-artwork", 300).await?;
    let user = p.user.id.clone();
    let result = video.clone();
    let added = storage::add_for_write_youtube_watchlists(video, grant, user, &state.db, id, input)
        .await?
        .ok_or_else(|| {
            ApiError::conflict(
                "The list no longer exists or your 1,000 saved video limit was reached",
            )
        })?;
    let auto_download = storage::add_for_read_youtube_watchlists(&state.db, id).await?;
    let download_error = if auto_download {
        downloads::request_for(state,p,result).await.err().map(|_|json!({"category":"api","code":"download_pending","message":"Saved. Automatic download will retry when server tools and video metadata are ready.","technical":""}))
    } else {
        None
    };
    let _ = changed(state).await?;
    Ok(Json(
        json!({"added":added.0,"video":added.1,"autoDownloadError":download_error}),
    ))
}
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, video)): Path<(i64, String)>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    remove_for(&state, &p, id, video).await
}
pub(crate) async fn remove_for(
    state: &AppState,
    p: &Principal,
    id: i64,
    video: String,
) -> Result<Json<Value>> {
    authorize(state, &p.user.id, id).await?;
    let user = p.user.id.clone();
    storage::remove_for(user, &state.db, id, video).await?;
    changed(state).await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reorder {
    video_ids: Vec<String>,
}
pub async fn reorder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Reorder>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    if input.video_ids.len() > 1000
        || input
            .video_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.video_ids.len()
    {
        return Err(ApiError::bad("Invalid watchlist order"));
    }
    let user = p.user.id.clone();
    storage::reorder(&state.db, id, input, user).await?;
    changed(&state).await
}
