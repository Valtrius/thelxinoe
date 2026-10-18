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
            Ok(c.query_row("SELECT (SELECT COUNT(*) FROM secrets)+(SELECT COUNT(*) FROM auth_totp)+(SELECT COUNT(*) FROM auth_oidc_provider)+(SELECT COUNT(*) FROM auth_attempts)+(SELECT COUNT(*) FROM manager_services)+(SELECT COUNT(*) FROM support_services)+(SELECT COUNT(*) FROM stack_provisions)+(SELECT COUNT(*) FROM online_accounts WHERE credential IS NOT NULL)", [], |r| r.get::<_, i64>(0))?)
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

/// How long events stay available to replay, and how many inserts apart old ones are removed.
const EVENT_RETENTION_SECONDS: i64 = 86400;
const EVENT_PRUNE_EVERY: i64 = 1000;

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
    // Progress ticks add a row each, so old rows go as new ones arrive. IDs never repeat, and
    // a client whose cursor predates what is kept still recovers: every connection opens with
    // server.reconnected, which refreshes everything it shows.
    if db.last_insert_rowid() % EVENT_PRUNE_EVERY == 0 {
        db.execute(
            "DELETE FROM events WHERE created_at<?1",
            [thelxinoe_core::now() - EVENT_RETENTION_SECONDS],
        )?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn old_events_are_pruned_and_ids_keep_increasing() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::open(temp.path().join("db")).unwrap();
        db.write("test.fixture", |c| {
            c.execute(
                "INSERT INTO events(id,kind,payload,created_at) VALUES (1,'old','{}',1),(?1,'recent','{}',?2)",
                rusqlite::params![EVENT_PRUNE_EVERY - 1, thelxinoe_core::now() - 60],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let events = || {
            db.read("test.events", |c| {
                Ok(c.prepare("SELECT id,kind FROM events ORDER BY id")?
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
        };
        emit(&db, "new".into(), None, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            events().await.unwrap(),
            [
                (EVENT_PRUNE_EVERY - 1, "recent".to_owned()),
                (EVENT_PRUNE_EVERY, "new".to_owned())
            ]
        );
        // Removed IDs are never reused, so a saved cursor never skips a newer event.
        db.write("test.fixture", |c| {
            c.execute("DELETE FROM events", [])?;
            Ok(())
        })
        .await
        .unwrap();
        emit(&db, "next".into(), None, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            events().await.unwrap(),
            [(EVENT_PRUNE_EVERY + 1, "next".to_owned())]
        );
    }
}
