//! Database operations for product.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn discovered(
    db: &Database,
    release: Option<(Envelope, Manifest, bool)>,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("product.discovered", move |db| {
        let tx = db.transaction()?;
        let previous: Value = tx.query_row("SELECT value FROM settings WHERE key='product.observation'", [], |r| r.get::<_, String>(0))
            .optional()?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let failed = error.is_some();
        let failures = if failed { previous["failures"].as_u64().unwrap_or(0).saturating_add(1).min(6) } else { 0 };
        let retry = if failed { (60i64 * (1 << failures.saturating_sub(1))).min(1800) } else { 6 * 3600 };
        let mut candidate = Value::Null;
        if let Some((envelope, manifest, available)) = release {
            tx.execute("INSERT INTO settings VALUES ('product.envelope',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json!(envelope).to_string()])?;
            if available { candidate = json!(manifest); }
        }
        for (key, value) in [
            ("product.release", candidate),
            ("product.observation", json!({"checked_at":now(),"last_success":if failed {previous["last_success"].clone()} else {json!(now())},"next_check":now()+retry,"failures":failures,"error":error})),
        ] {
            tx.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }).await
}

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
