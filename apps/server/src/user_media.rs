#[path = "storage/user_media.rs"]
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
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub(crate) use storage::card;
pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    get_for(&state, &p, &media).await.map(Json)
}
pub(crate) async fn get_for(state: &AppState, p: &Principal, media: &str) -> Result<Value> {
    let p = p.clone();
    let media = media.to_owned();
    let result = storage::get_for(p, media, &state.db).await?;
    result.ok_or_else(ApiError::not_found)
}
#[derive(Deserialize)]
pub struct Change {
    pub(crate) favorite: Option<bool>,
    pub(crate) watch_later: Option<bool>,
    pub(crate) watched: Option<bool>,
}
pub async fn set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Json(input): Json<Change>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    set_for(&state, &p, &media, input).await.map(Json)
}
pub(crate) async fn set_for(
    state: &AppState,
    p: &Principal,
    media: &str,
    input: Change,
) -> Result<Value> {
    let _lease = state.media_operations.read().await;
    let user = p.user.id.clone();
    let mid = media.to_owned();
    let result = storage::set_for(user, mid, &state.db, input).await?;
    match result {
        404 => return Err(ApiError::not_found()),
        400 => {
            return Err(ApiError::bad(
                "This state is not supported for this media type",
            ));
        }
        _ => {}
    }
    state
        .emit(
            Some(p.user.id.clone()),
            "media-state.changed",
            json!({"media_id":media}),
        )
        .await?;
    get_for(state, p, media).await
}
pub async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let mut home = home_for(&state, &p).await?;
    let mut items = home
        .as_object_mut()
        .into_iter()
        .flat_map(|shelves| shelves.values_mut())
        .filter_map(Value::as_array_mut)
        .flat_map(|items| items.iter_mut())
        .collect::<Vec<_>>();
    crate::library::decorate_cards(&state, &p, &mut items).await?;
    Ok(Json(home))
}
pub(crate) async fn home_for(state: &AppState, principal: &Principal) -> Result<Value> {
    let p = principal.clone();
    let result = storage::home_for(p, &state.db).await?;
    Ok(result)
}
#[derive(Clone, Deserialize, Serialize)]
pub struct QueueContext {
    pub client_id: String,
    pub revision: i64,
    pub index: usize,
}
pub(crate) fn valid_client(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()
}
pub(crate) use storage::queue_value;
pub(crate) use storage::validate_tracks;
pub async fn queue_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if !valid_client(&client) {
        return Err(ApiError::bad("Invalid client identifier"));
    }
    Ok(Json(storage::queue_get(&state.db, client, p).await?))
}
#[derive(Deserialize)]
pub struct SaveQueue {
    revision: i64,
    items: Vec<String>,
    current_index: usize,
}
pub async fn queue_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client): Path<String>,
    Json(input): Json<SaveQueue>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if !valid_client(&client)
        || input.items.len() > 500
        || input.current_index >= input.items.len().max(1)
    {
        return Err(ApiError::bad("Invalid music queue"));
    }
    let principal = p.clone();
    let result = storage::queue_put(&state.db, client, input, principal).await?;
    match result.0 {
        409 => Err(ApiError::conflict(
            "This queue changed in another window; reload it before editing",
        )),
        400 => Err(ApiError::bad(
            "Choose up to 500 catalog tracks and at most 100 client queues",
        )),
        _ => Ok(Json(result.1)),
    }
}
