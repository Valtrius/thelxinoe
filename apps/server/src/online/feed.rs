#[path = "../storage/online/feed.rs"]
mod storage;

use super::sync;
use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

#[derive(Default, Deserialize)]
pub struct Filter {
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    watchlist: bool,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    hide_shorts: bool,
    #[serde(default)]
    unwatched: bool,
    #[serde(default)]
    search: String,
    #[serde(default)]
    channel: String,
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if filter.offset > 100000
        || filter.search.len() > 200
        || (!filter.channel.is_empty() && !sync::identifier(&filter.channel, 24))
    {
        return Err(ApiError::bad("Invalid feed filter"));
    }
    let owner = p.user.id.clone();
    let (mut items, total, sync) = storage::list(&state.db, filter, owner).await?;
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    for item in &mut items {
        if item["privacy"] == "public" {
            item["artwork_url"] = json!(format!(
                "/api/v1/online/youtube/videos/{}/artwork?grant={grant}",
                item["id"].as_str().unwrap()
            ));
        }
    }
    Ok(Json(json!({"items":items,"total":total,"sync":sync})))
}
#[derive(Deserialize)]
pub struct Add {
    url: String,
}
pub(super) fn video_id(value: &str) -> Option<String> {
    let value = value.trim();
    if sync::identifier(value, 11) {
        return Some(value.into());
    }
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let id = match url.host_str()? {
        "youtu.be" => url.path().trim_start_matches('/').to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            if url.path() == "/watch" {
                url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
            } else {
                let mut pieces = url.path().trim_start_matches('/').split('/');
                if !matches!(pieces.next()?, "shorts" | "live" | "embed") {
                    return None;
                }
                let id = pieces.next()?.to_owned();
                if pieces.next().is_some() {
                    return None;
                }
                id
            }
        }
        _ => return None,
    };
    sync::identifier(&id, 11).then_some(id)
}
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Add>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let video = video_id(&input.url)
        .ok_or_else(|| ApiError::bad("Enter a YouTube video URL or video ID"))?;
    let result = video.clone();
    let user = p.user.id.clone();
    let added = storage::add(&state.db, video, user).await?;
    if !added {
        return Err(ApiError::conflict(
            "Your YouTube watchlist and pins can retain at most 1,000 videos",
        ));
    }
    state
        .emit(Some(p.user.id), "youtube.changed", json!({}))
        .await?;
    Ok(Json(json!({"id":result,"watchlist":true})))
}
#[derive(Deserialize)]
pub struct Edit {
    pub watchlist: Option<bool>,
    pub pinned: Option<bool>,
    pub watched: Option<bool>,
}
pub async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    edit_for(&state, &p, video, input).await
}
pub(crate) async fn edit_for(
    state: &AppState,
    p: &thelxinoe_core::Principal,
    video: String,
    input: Edit,
) -> Result<Json<Value>> {
    let user = p.user.id.clone();
    let updated = storage::edit_for(user, &state.db, video, input).await?;
    if updated == 2 {
        return Err(ApiError::conflict(
            "Your YouTube watchlist and pins can retain at most 1,000 videos",
        ));
    }
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(p.user.id.clone()), "youtube.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn delete_data(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    storage::delete_data(&state.db, user).await?;
    state
        .emit(
            Some(p.user.id),
            "online.account.changed",
            json!({"provider":"youtube"}),
        )
        .await?;
    Ok(Json(json!({"deleted":true})))
}
#[derive(Deserialize)]
pub struct Artwork {
    grant: String,
}
pub async fn artwork(
    State(state): State<AppState>,
    Path(video): Path<String>,
    Query(query): Query<Artwork>,
) -> Result<Response> {
    let p = grants::resolve(&state, &query.grant, "youtube-artwork", false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if !sync::identifier(&video, 11) {
        return Err(ApiError::not_found());
    }
    let id = video.clone();
    let allowed = storage::artwork(&state.db, p, id).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    let (mime, bytes) = super::youtube_artwork_bytes(&state, &video).await?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=300"),
        ],
        bytes,
    )
        .into_response())
}
