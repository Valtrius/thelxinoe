//! Per-user UI preferences; partial updates avoid overwriting other client changes.

#[path = "storage/appearance.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    provider_preferences: std::collections::BTreeMap<String, String>,
    audio_volume: f64,
    player_height: Option<f64>,
    youtube_card_shortcuts: Vec<String>,
    theme: String,
    sidebar_collapsed: bool,
    card_columns: u8,
    fade_watched: bool,
    thumbnail_fit: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            provider_preferences: Default::default(),
            audio_volume: 1.0,
            player_height: None,
            youtube_card_shortcuts: vec![],
            theme: "system".into(),
            sidebar_collapsed: false,
            card_columns: 6,
            fade_watched: true,
            thumbnail_fit: "contain".into(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    provider_preferences: Option<std::collections::BTreeMap<String, String>>,
    audio_volume: Option<f64>,
    #[serde(default, deserialize_with = "optional_player_height")]
    player_height: Option<Option<f64>>,
    youtube_card_shortcuts: Option<Vec<String>>,
    theme: Option<String>,
    sidebar_collapsed: Option<bool>,
    card_columns: Option<u8>,
    fade_watched: Option<bool>,
    thumbnail_fit: Option<String>,
}
fn optional_player_height<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<f64>::deserialize(deserializer).map(Some)
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Appearance>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(storage::get(&state.db, p).await?))
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(change): Json<Change>,
) -> Result<Json<Appearance>> {
    let p = security::principal(&state, &headers).await?;
    if change
        .audio_volume
        .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        || change
            .player_height
            .flatten()
            .is_some_and(|v| !v.is_finite() || v < 210.0)
        || change.provider_preferences.as_ref().is_some_and(|v| {
            v.iter().any(|(key, value)| {
                ![
                    "youtube-feed-layout",
                    "youtube-watchlist-sidebar-open",
                    "youtube-selected-watchlist-id",
                ]
                .contains(&key.as_str())
                    || value.len() > 8192
            })
        })
    {
        return Err(ApiError::bad("Invalid player or provider preferences"));
    }
    if change.youtube_card_shortcuts.as_ref().is_some_and(|v| {
        v.len() > 3
            || v.iter().collect::<std::collections::HashSet<_>>().len() != v.len()
            || v.iter().any(|s| {
                ![
                    "play_from_beginning",
                    "download",
                    "watch_toggle",
                    "copy_url",
                    "open_browser",
                    "pin_download",
                    "delete_download",
                    "add_watch_later",
                ]
                .contains(&s.as_str())
            })
    }) {
        return Err(ApiError::bad("Choose up to three supported shortcuts"));
    }
    if change
        .theme
        .as_deref()
        .is_some_and(|v| !["light", "system", "dark"].contains(&v))
        || change.card_columns.is_some_and(|v| !(3..=12).contains(&v))
        || change
            .thumbnail_fit
            .as_deref()
            .is_some_and(|v| !["contain", "cover"].contains(&v))
    {
        return Err(ApiError::bad(
            "Choose a supported theme, thumbnail fit and 3 to 12 card columns",
        ));
    }
    let value = storage::update(&state.db, change, p).await?;
    let _ = state.events.send(());
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use crate::online::oauth::tests::{call, fixture};
    use serde_json::{Value, json};
    #[tokio::test]
    async fn player_and_provider_changes_merge_and_emit_only_to_the_owner() {
        let (_temp, state, alice) = fixture().await;
        let (volume, filter) = tokio::join!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"audio_volume":0.27}),
                &alice
            ),
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"provider_preferences":{"youtube-feed-layout":"{\"showLive\":true}"}}),
                &alice
            )
        );
        assert_eq!(volume.0, axum::http::StatusCode::OK);
        assert_eq!(filter.0, axum::http::StatusCode::OK);
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"provider_preferences":{"youtube-watchlist-sidebar-open":"true"}}),
            &alice,
        )
        .await
        .2;
        assert_eq!(saved["audio_volume"], 0.27);
        assert_eq!(saved["player_height"], Value::Null);
        assert_eq!(
            saved["provider_preferences"]["youtube-feed-layout"],
            "{\"showLive\":true}"
        );
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"player_height":420}),
            &alice,
        )
        .await
        .2;
        assert_eq!(saved["player_height"], 420.0);
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"player_height":209}),
                &alice
            )
            .await
            .0,
            axum::http::StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"player_height":null}),
                &alice
            )
            .await
            .2["player_height"],
            Value::Null
        );
        state.db.write("test.fixture", |db| {
            assert_eq!(db.query_row("SELECT COUNT(*) FROM events WHERE kind='appearance.changed' AND user_id='alice'", [], |r|r.get::<_,i64>(0))?, 5);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM events WHERE kind='appearance.changed' AND (user_id IS NULL OR user_id<>'alice')", [], |r|r.get::<_,i64>(0))?, 0);
            Ok(())
        }).await.unwrap();
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"audio_volume":1.1}),
                &alice
            )
            .await
            .0,
            axum::http::StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn partial_preferences_preserve_theme_and_stay_private() {
        let (_temp, state, alice) = fixture().await;
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"theme":"dark"}),
                &alice
            )
            .await
            .2["theme"],
            "dark"
        );
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"card_columns":9}),
            &alice,
        )
        .await;
        assert_eq!(saved.2["theme"], "dark");
        assert_eq!(saved.2["card_columns"], 9);
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"theme":"invalid"}),
                &alice
            )
            .await
            .0,
            axum::http::StatusCode::BAD_REQUEST
        );
        let token =
            thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "GET",
                Value::Null,
                &format!("thelxinoe_session={token}")
            )
            .await
            .2["theme"],
            "system"
        );
    }
}
