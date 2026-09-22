//! Database operations for product.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn setting(key: String, db: &Database) -> anyhow::Result<Option<Value>> {
    db.read("product.setting", move |db| {
        Ok(db
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()?)
    })
    .await
}

pub(super) async fn save(key: String, db: &Database, value: Value) -> anyhow::Result<()> {
    db.write("product.save", move|db|{db.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()])?;Ok(())}).await
}

pub(super) async fn audit(
    action: String,
    target: String,
    db: &Database,
    user: Option<String>,
) -> anyhow::Result<()> {
    db.write("product.audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![user, action, target, now()],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn idle(db: &Database) -> anyhow::Result<bool> {
    db.read("product.idle", |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running') OR EXISTS(SELECT 1 FROM media_operations WHERE state='executing')",[now()],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn server_zone(db: &Database) -> anyhow::Result<chrono_tz::Tz> {
    db.read("product.server_zone", crate::timezones::server_zone)
        .await
}
