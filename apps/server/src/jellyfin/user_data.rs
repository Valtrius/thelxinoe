#[path = "../storage/jellyfin/user_data.rs"]
mod storage;

use super::{Query, canonical, catalog, online};
use crate::{
    AppState,
    error::{ApiError, Result},
};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::Principal;

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub(super) struct Update {
    is_favorite: Option<bool>,
    played: Option<bool>,
    playback_position_ticks: Option<i64>,
    key: Option<String>,
    item_id: Option<String>,
    rating: Option<f64>,
    played_percentage: Option<f64>,
    unplayed_item_count: Option<i32>,
    play_count: Option<i32>,
    likes: Option<bool>,
    last_played_date: Option<String>,
}

pub(super) enum Target {
    Media(String),
    YouTube(String),
    Playlist(String),
    Online(String),
}

pub async fn handle(
    state: &AppState,
    p: &Principal,
    id: &str,
    input: Option<Value>,
) -> Result<Value> {
    let mut item = catalog::browse(state, p, &Query::new(), Some(id)).await?;
    if let Some(input) = input {
        let update: Update =
            serde_json::from_value(input).map_err(|_| ApiError::bad("Invalid user data"))?;
        if update.key.as_ref().is_some_and(|key| canonical(key) != id)
            || update
                .item_id
                .as_ref()
                .is_some_and(|key| canonical(key) != id)
            || update
                .playback_position_ticks
                .is_some_and(|ticks| ticks < 0)
        {
            return Err(ApiError::bad("Invalid item identity or playback position"));
        }
        if update.rating.is_some()
            || update.likes.is_some()
            || update.play_count.is_some()
            || update.last_played_date.is_some()
            || update.played_percentage.is_some()
            || update.unplayed_item_count.is_some()
        {
            return Err(ApiError::bad(
                "Only favorite, played, and playback position can be updated",
            ));
        }
        let target = if let Some(identity) = online::resolve(state, p, id).await? {
            if identity.kind == "youtube" {
                if item["IsLive"] == true
                    && (update.played.is_some() || update.playback_position_ticks.is_some())
                {
                    return Err(ApiError::bad(
                        "Live streams do not have watched or resume state",
                    ));
                }
                Target::YouTube(identity.key)
            } else {
                if update.played.is_some() || update.playback_position_ticks.is_some() {
                    return Err(ApiError::bad("This item only supports favorite state"));
                }
                Target::Online(identity.id)
            }
        } else if catalog::is_playlist(state, id).await? {
            if update.played.is_some() || update.playback_position_ticks.is_some() {
                return Err(ApiError::bad("Playlists only support favorite state"));
            }
            Target::Playlist(id.into())
        } else {
            if !["Movie", "Episode", "Audio"].contains(&item["Type"].as_str().unwrap_or(""))
                && (update.played.is_some() || update.playback_position_ticks.is_some())
            {
                return Err(ApiError::bad(
                    "This item does not have watched or resume state",
                ));
            }
            Target::Media(id.into())
        };
        let keys = crate::media_resources::for_media(&state.db, id).await?;
        let _files = state
            .media_resources
            .read(&keys.iter().map(String::as_str).collect::<Vec<_>>())
            .await;
        let _lease = state.media_operations.read().await;
        let event = match &target {
            Target::YouTube(_) | Target::Online(_) => "youtube.changed",
            Target::Playlist(_) => "playlists.changed",
            Target::Media(_) => "media-state.changed",
        };
        match storage::update(&state.db, p.user.id.clone(), target, update).await? {
            404 => return Err(ApiError::not_found()),
            400 => {
                return Err(ApiError::bad(
                    "No available edition can store this playback position",
                ));
            }
            409 => {
                return Err(ApiError::conflict(
                    "Your YouTube watchlist and pins can retain at most 1,000 videos",
                ));
            }
            _ => {}
        }
        state
            .emit(Some(p.user.id.clone()), event, json!({"media_id":id}))
            .await?;
        item = catalog::browse(state, p, &Query::new(), Some(id)).await?;
    }
    if item["UserData"].is_object() {
        Ok(item["UserData"].clone())
    } else {
        Ok(
            json!({"Key":id,"ItemId":id,"IsFavorite":false,"Played":false,"PlayCount":0,"PlaybackPositionTicks":0}),
        )
    }
}
