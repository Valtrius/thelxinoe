//! Per-user UI preferences; partial updates avoid overwriting other client changes.
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
    youtube_card_shortcuts: Option<Vec<String>>,
    theme: Option<String>,
    sidebar_collapsed: Option<bool>,
    card_columns: Option<u8>,
    fade_watched: Option<bool>,
    thumbnail_fit: Option<String>,
}
fn read(db: &rusqlite::Connection, user: &str) -> anyhow::Result<Appearance> {
    Ok(db
        .query_row(
            "SELECT value FROM ui_preferences WHERE user_id=?1",
            [user],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default())
}
pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Appearance>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(state.db.call(move |db| read(db, &p.user.id)).await?))
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(change): Json<Change>,
) -> Result<Json<Appearance>> {
    let p = security::principal(&state, &headers).await?;
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
    let value = state.db.call(move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut value = read(&tx, &p.user.id)?;
        if let Some(v) = change.youtube_card_shortcuts { value.youtube_card_shortcuts = v; }
        if let Some(v) = change.theme { value.theme = v; }
        if let Some(v) = change.sidebar_collapsed { value.sidebar_collapsed = v; }
        if let Some(v) = change.card_columns { value.card_columns = v; }
        if let Some(v) = change.fade_watched { value.fade_watched = v; }
        if let Some(v) = change.thumbnail_fit { value.thumbnail_fit = v; }
        tx.execute("INSERT INTO ui_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value", params![p.user.id, serde_json::to_string(&value)?])?;
        tx.commit()?;
        Ok(value)
    }).await?;
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use crate::online::oauth::tests::{call, fixture};
    use serde_json::{Value, json};
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
