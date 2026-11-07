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

/// The mock servers refuse other keys too, so UI tests catch one missing here.
const PROVIDER_PREFERENCE_KEYS: &str = include_str!("provider_preference_keys.txt");

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    provider_preferences: std::collections::BTreeMap<String, String>,
    audio_volume: f64,
    player_height: Option<f64>,
    youtube_card_shortcuts: Vec<String>,
    theme: String,
    sidebar_collapsed: bool,
    poster_columns: u8,
    video_columns: u8,
    animation_duration_ms: u16,
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
            poster_columns: 9,
            video_columns: 6,
            animation_duration_ms: 200,
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
    poster_columns: Option<u8>,
    video_columns: Option<u8>,
    animation_duration_ms: Option<u16>,
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
    if change.animation_duration_ms.is_some_and(|v| v > 500) {
        return Err(ApiError::bad(
            "Choose an animation duration from 0 to 500 ms",
        ));
    }
    if change
        .audio_volume
        .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        || change
            .player_height
            .flatten()
            .is_some_and(|v| !v.is_finite() || v < 210.0)
        || change.provider_preferences.as_ref().is_some_and(|v| {
            v.iter().any(|(key, value)| {
                !PROVIDER_PREFERENCE_KEYS.lines().any(|known| known == key) || value.len() > 8192
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
        || change
            .poster_columns
            .is_some_and(|v| !(5..=18).contains(&v))
        || change.video_columns.is_some_and(|v| !(3..=12).contains(&v))
        || change
            .thumbnail_fit
            .as_deref()
            .is_some_and(|v| !["contain", "cover"].contains(&v))
    {
        return Err(ApiError::bad(
            "Choose a supported theme, thumbnail fit, 5 to 18 poster columns and 3 to 12 video columns",
        ));
    }
    let value = storage::update(&state.db, change, p).await?;
    let _ = state.events.send(());
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use crate::test_support::{call, fixture};
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
    async fn music_preferences_save_and_unknown_preferences_do_not() {
        let (_temp, state, alice) = fixture().await;
        for key in [
            "music-view",
            "music-sort",
            "music-selected-playlist-id",
            "music-playlist-sidebar-open",
        ] {
            let saved = call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"provider_preferences":{key:"value"}}),
                &alice,
            )
            .await;
            assert_eq!(saved.0, axum::http::StatusCode::OK, "{key}");
            assert_eq!(saved.2["provider_preferences"][key], "value");
        }
        assert_eq!(
            call(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"provider_preferences":{"unknown":"value"}}),
                &alice,
            )
            .await
            .0,
            axum::http::StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn partial_preferences_preserve_theme_and_stay_private() {
        let (_temp, state, alice) = fixture().await;
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"theme":"dark"}),
            &alice,
        )
        .await;
        assert_eq!(saved.2["theme"], "dark");
        assert_eq!(saved.2["poster_columns"], 9);
        assert_eq!(saved.2["video_columns"], 6);
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"poster_columns":12}),
            &alice,
        )
        .await;
        assert_eq!(saved.2["theme"], "dark");
        assert_eq!(saved.2["poster_columns"], 12);
        assert_eq!(saved.2["video_columns"], 6);
        let saved = call(
            &state,
            "/api/v1/me/appearance",
            "PATCH",
            json!({"video_columns":4}),
            &alice,
        )
        .await;
        assert_eq!(saved.2["poster_columns"], 12);
        assert_eq!(saved.2["video_columns"], 4);
        for (field, valid, invalid) in [
            ("poster_columns", [5, 18], [4, 19]),
            ("video_columns", [3, 12], [2, 13]),
        ] {
            for columns in valid {
                assert_eq!(
                    call(
                        &state,
                        "/api/v1/me/appearance",
                        "PATCH",
                        json!({field:columns}),
                        &alice,
                    )
                    .await
                    .2[field],
                    columns
                );
            }
            for columns in invalid {
                assert_eq!(
                    call(
                        &state,
                        "/api/v1/me/appearance",
                        "PATCH",
                        json!({field:columns}),
                        &alice,
                    )
                    .await
                    .0,
                    axum::http::StatusCode::BAD_REQUEST
                );
            }
        }
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
            crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
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
