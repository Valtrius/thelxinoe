use super::Availability;
use rusqlite::{OptionalExtension, params};
use thelxinoe_database::Database;

pub(crate) fn enabled_on(db: &rusqlite::Connection, provider: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!("online.{provider}.enabled")],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_none_or(|value| value != "false"))
}

pub(super) async fn enabled(db: &Database, provider: String) -> anyhow::Result<bool> {
    db.read("online.provider.enabled", move |db| {
        enabled_on(db, &provider)
    })
    .await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Availability> {
    db.read("online.providers", |db| {
        Ok(Availability {
            youtube: enabled_on(db, "youtube")?,
            twitch: enabled_on(db, "twitch")?,
            kick: enabled_on(db, "kick")?,
        })
    })
    .await
}

pub(super) async fn save(
    db: &Database,
    provider: String,
    enabled: bool,
    actor: String,
) -> anyhow::Result<Vec<String>> {
    db.write("online.provider.save", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("online.{provider}.enabled"), enabled.to_string()])?;
        let mut stopped = Vec::new();
        if !enabled {
            tx.execute("DELETE FROM oauth_attempts WHERE provider=?1", [&provider])?;
            if provider == "twitch" { tx.execute("DELETE FROM twitch_attempts", [])?; }
            let prefix = format!("{provider}:%");
            stopped = tx.prepare("SELECT id FROM playback_sessions WHERE COALESCE('youtube:'||youtube_video_id,live_media_id,'') LIKE ?1 AND state NOT IN ('stopped','failed')")?.query_map([&prefix], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            tx.execute("UPDATE playback_sessions SET state='stopped',generation=?1,updated_at=?2 WHERE COALESCE('youtube:'||youtube_video_id,live_media_id,'') LIKE ?3 AND state NOT IN ('stopped','failed')", params![thelxinoe_core::id(), thelxinoe_core::now(), prefix])?;
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)", params![actor, if enabled { "online.enable" } else { "online.disable" }, provider, thelxinoe_core::now()])?;
        tx.commit()?;
        Ok(stopped)
    }).await
}
