#[path = "storage/playlists.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
    user_media::{card, validate_tracks},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, id, now};

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items = storage::list(&state.db, p).await?;
    Ok(Json(json!({"items":items})))
}

pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let result = storage::detail(&state.db, id, p).await?;
    Ok(Json(result.ok_or_else(ApiError::not_found)?))
}
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
    if input.name.trim().is_empty()
        || input.name.len() > 160
        || input.description.len() > 2000
        || input.items.len() > 500
    {
        return Err(ApiError::bad(
            "Enter a name, a description up to 2,000 characters and up to 500 tracks",
        ));
    }
    let creating = key.is_none();
    let key = key.unwrap_or_else(id);
    let playlist = key.clone();
    let result = storage::save(creating, key, &state.db, p, input).await?;
    match result {
        400 => {
            return Err(ApiError::bad(
                "Use catalog music tracks and at most 100 owned playlists",
            ));
        }
        403 => return Err(ApiError::forbidden()),
        404 => return Err(ApiError::not_found()),
        409 => {
            return Err(ApiError::conflict(
                "This playlist changed; reload before editing",
            ));
        }
        _ => {}
    }
    state
        .emit(None, "playlists.changed", json!({"id":playlist}))
        .await?;
    Ok(json!({"id":playlist}))
}
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let key = id.clone();
    let code = storage::remove(&state.db, p, key).await?;
    if code == 404 {
        return Err(ApiError::not_found());
    }
    if code == 403 {
        return Err(ApiError::forbidden());
    }
    state
        .emit(None, "playlists.changed", json!({"id":id}))
        .await?;
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
