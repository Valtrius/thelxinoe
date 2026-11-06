#[path = "storage/playlists.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    library, security,
    user_media::validate_tracks,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, id, now};

/// Why a playlist change was refused.
enum Refusal {
    Invalid,
    Forbidden,
    Missing,
    Stale,
    NameTaken,
}
impl From<Refusal> for ApiError {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Invalid => {
                ApiError::bad("Use catalog music tracks and at most 100 owned playlists")
            }
            Refusal::Forbidden => ApiError::forbidden(),
            Refusal::Missing => ApiError::not_found(),
            Refusal::Stale => ApiError::conflict("This playlist changed; reload before editing"),
            Refusal::NameTaken => ApiError::conflict("You already have a playlist with this name"),
        }
    }
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 160
}
async fn changed(state: &AppState, id: &str) -> Result<()> {
    state
        .emit(None, "playlists.changed", json!({"id":id}))
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct Listing {
    /// Also says whether each playlist holds this track.
    track: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Listing>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items = storage::list(&state.db, p, input.track).await?;
    Ok(Json(json!({"items":items})))
}

pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let mut playlist = storage::detail(&state.db, id.clone(), p.user.id.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    playlist["items"] = json!(library::playlist_tracks(&state, &p, id).await?);
    Ok(Json(playlist))
}
/// A whole playlist, as released desktops save it.
#[derive(Deserialize)]
pub struct Save {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    items: Vec<String>,
}
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Save>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    save(&state, p, None, input).await.map(Json)
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Save>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    save(&state, p, Some(id), input).await.map(Json)
}
async fn save(state: &AppState, p: Principal, key: Option<String>, input: Save) -> Result<Value> {
    if !name_valid(&input.name) || input.description.len() > 2000 {
        return Err(ApiError::bad(
            "Enter a name up to 160 characters and a description up to 2,000",
        ));
    }
    let creating = key.is_none();
    let key = key.unwrap_or_else(id);
    let playlist = key.clone();
    storage::save(creating, key, &state.db, p, input).await??;
    changed(state, &playlist).await?;
    Ok(json!({"id":playlist}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    name: Option<String>,
    access: Option<String>,
    sort_mode: Option<String>,
    sort_direction: Option<String>,
}
pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Change>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if input.name.as_deref().is_some_and(|name| !name_valid(name))
        || input
            .access
            .as_deref()
            .is_some_and(|access| !["private", "read", "add", "edit"].contains(&access))
        || input
            .sort_mode
            .as_deref()
            .is_some_and(|mode| !["manual", "added", "title", "artist"].contains(&mode))
        || input
            .sort_direction
            .as_deref()
            .is_some_and(|direction| !["asc", "desc"].contains(&direction))
    {
        return Err(ApiError::bad("Invalid playlist settings"));
    }
    let revision = storage::change(&state.db, id.clone(), p.user.id, input).await??;
    changed(&state, &id).await?;
    Ok(Json(json!({"revision":revision})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tracks {
    items: Vec<String>,
}
pub async fn add_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Tracks>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let (added, revision) =
        storage::add_items(&state.db, id.clone(), p.user.id, input.items).await??;
    if added > 0 {
        changed(&state, &id).await?;
    }
    Ok(Json(json!({"added":added,"revision":revision})))
}
pub async fn remove_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, media)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let revision = storage::remove_item(&state.db, id.clone(), p.user.id, media).await??;
    changed(&state, &id).await?;
    Ok(Json(json!({"revision":revision})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Move {
    /// The track it now comes before; none moves it to the end.
    before: Option<String>,
}
pub async fn move_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, media)): Path<(String, String)>,
    Json(input): Json<Move>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let revision =
        storage::move_item(&state.db, id.clone(), p.user.id, media, input.before).await??;
    changed(&state, &id).await?;
    Ok(Json(json!({"revision":revision})))
}
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    storage::remove(&state.db, p, id.clone()).await??;
    changed(&state, &id).await?;
    Ok(Json(json!({"deleted":true})))
}
#[derive(Deserialize)]
pub struct Favorite {
    favorite: bool,
}
pub async fn favorite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Favorite>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    favorite_for(&state, &p, &id, input.favorite)
        .await
        .map(Json)
}
pub(crate) async fn favorite_for(
    state: &AppState,
    p: &Principal,
    id: &str,
    favorite: bool,
) -> Result<Value> {
    let user = p.user.id.clone();
    let key = id.to_owned();
    let found = storage::favorite_for(user, key, &state.db, favorite).await?;
    if !found {
        return Err(ApiError::not_found());
    }
    state
        .emit(
            Some(p.user.id.clone()),
            "playlists.changed",
            json!({"id":id}),
        )
        .await?;
    Ok(json!({"favorite":favorite}))
}
