//! Database operations for appearance.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn get(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<Appearance> {
    db.read("appearance.get", move |db| read(db, &p.user.id))
        .await
}

pub(super) async fn update(
    db: &Database,
    change: Change,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Appearance> {
    db.write("appearance.update", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut value = read(&tx, &p.user.id)?;
        if let Some(v) = change.provider_preferences { value.provider_preferences.extend(v); }
        if let Some(v) = change.audio_volume { value.audio_volume = v; }
        if let Some(v) = change.player_height { value.player_height = v; }
        if let Some(v) = change.youtube_card_shortcuts { value.youtube_card_shortcuts = v; }
        if let Some(v) = change.theme { value.theme = v; }
        if let Some(v) = change.sidebar_collapsed { value.sidebar_collapsed = v; }
        if let Some(v) = change.card_columns { value.card_columns = v; }
        if let Some(v) = change.fade_watched { value.fade_watched = v; }
        if let Some(v) = change.thumbnail_fit { value.thumbnail_fit = v; }
        tx.execute("INSERT INTO ui_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value", params![p.user.id, serde_json::to_string(&value)?])?;
        tx.execute(
            "INSERT INTO events(user_id,kind,payload,created_at) VALUES (?1,'appearance.changed',?2,?3)",
            params![p.user.id, serde_json::json!({"appearance":&value}).to_string(), thelxinoe_core::now()],
        )?;
        tx.commit()?;
        Ok(value)
    }).await
}

pub(super) fn read(db: &rusqlite::Connection, user: &str) -> anyhow::Result<Appearance> {
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
