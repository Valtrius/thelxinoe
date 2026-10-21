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
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub(crate) use storage::{card, validate_tracks};
/// Released desktops (API 1) save the music queue before they play a track. They get
/// an empty queue that is never stored, until API_MIN_VERSION drops them.
pub async fn legacy_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    Ok(Json(
        json!({"revision":0,"items":[],"current_index":0,"position":0}),
    ))
}
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
    let keys = crate::media_resources::for_media(&state.db, media).await?;
    let _files = state
        .media_resources
        .read(&keys.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
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
    let grant = crate::grants::issue(&state, &p, "artwork", 300).await?;
    crate::library::decorate_cards_with(&state, &p, &mut items, &grant).await?;
    // Home shelves are landscape: cards prefer their title's backdrop over a poster.
    crate::managers::status::decorate(&state, &p, &mut items, &grant).await?;
    Ok(Json(home))
}
pub(crate) async fn home_for(state: &AppState, principal: &Principal) -> Result<Value> {
    let p = principal.clone();
    let result = storage::home_for(p, &state.db).await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{call, fixture};
    use axum::http::StatusCode;
    use serde_json::json;

    #[tokio::test]
    async fn released_desktops_still_get_an_empty_queue_before_playing_music() {
        let (_temp, state, cookie) = fixture().await;
        let path = "/api/v1/me/queue/9f7a3a8e-1d2c-4b5e-8f6a-0c1d2e3f4a5b";
        for (method, body) in [
            ("GET", json!(null)),
            (
                "PUT",
                json!({"revision":0,"items":["track"],"current_index":0}),
            ),
        ] {
            let (status, _, queue) = call(&state, path, method, body, &cookie).await;
            assert_eq!(status, StatusCode::OK, "{method}");
            assert_eq!(queue["revision"], 0);
            assert_eq!(queue["items"], json!([]));
        }
        let (status, _, _) = call(&state, path, "GET", json!(null), "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
