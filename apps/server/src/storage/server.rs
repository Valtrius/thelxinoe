//! Database operations for server.
use thelxinoe_database::Database;

pub(super) async fn open_write_media_operations(db: &Database) -> anyhow::Result<()> {
    db.write("server.open_write_media_operations", |db| {db.execute("UPDATE media_operations SET state='uncertain',error='Server stopped during execution; reconcile before preparing another operation' WHERE state='executing'",[])?;Ok(())}).await
}

pub(super) async fn open_write_settings(db: &Database) -> anyhow::Result<String> {
    db.write("server.open_write_settings", |db| {
        db.execute(
            "INSERT INTO settings(key,value) VALUES ('server_id',?1) ON CONFLICT(key) DO NOTHING",
            [thelxinoe_core::id()],
        )?;
        Ok(db.query_row(
            "SELECT value FROM settings WHERE key='server_id'",
            [],
            |r| r.get::<_, String>(0),
        )?)
    })
    .await
}

pub(super) async fn open_read_secrets(db: &Database) -> anyhow::Result<i64> {
    db
        .read("server.open_read_secrets", |c| {
            Ok(c.query_row("SELECT (SELECT COUNT(*) FROM secrets)+(SELECT COUNT(*) FROM manager_services)+(SELECT COUNT(*) FROM support_services)+(SELECT COUNT(*) FROM stack_provisions)+(SELECT COUNT(*) FROM online_accounts WHERE credential IS NOT NULL)", [], |r| r.get::<_, i64>(0))?)
        })
        .await
}

pub(super) async fn emit(
    db: &Database,
    kind: String,
    user_id: Option<String>,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    db.write("server.emit", move |c| {
        record_event(c, user_id.as_deref(), &kind, &payload)
    })
    .await
}

/// Call within the domain transaction when an event describes a database change.
pub(crate) fn record_event(
    db: &rusqlite::Connection,
    user: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO events(user_id,kind,payload,created_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![user, kind, payload.to_string(), thelxinoe_core::now()],
    )?;
    Ok(())
}

pub(super) async fn run_jobs(
    error: String,
    root: thelxinoe_catalog::Root,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("server.run_jobs", move |db| {
        db.execute(
            "UPDATE library_roots SET scan_error=?1 WHERE id=?2",
            rusqlite::params![error, root.id],
        )?;
        Ok(())
    })
    .await
}
