use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub struct Database {
    path: Arc<PathBuf>,
    slots: Arc<tokio::sync::Semaphore>,
    // Keep WAL/SHM alive between calls rather than repeatedly closing the last connection.
    _anchor: Arc<std::sync::Mutex<Option<Connection>>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Self {
            path: Arc::new(path.as_ref().to_owned()),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
            _anchor: Arc::new(std::sync::Mutex::new(None)),
        };
        let mut conn = db.connect()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY);",
        )?;
        let version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        let migrations = [
            include_str!("../migrations/001.sql"),
            include_str!("../migrations/002.sql"),
            include_str!("../migrations/003.sql"),
            include_str!("../migrations/004.sql"),
            include_str!("../migrations/005.sql"),
            include_str!("../migrations/006.sql"),
            include_str!("../migrations/007.sql"),
        ];
        if version > migrations.len() as i64 {
            anyhow::bail!("Database is newer than this server; use the matching release");
        }
        for (index, migration) in migrations.iter().enumerate().skip(version as usize) {
            tx.execute_batch(migration)?;
            tx.execute(
                "INSERT INTO schema_migrations VALUES (?1)",
                [(index + 1) as i64],
            )?;
        }
        tx.commit()?;
        *db._anchor.lock().expect("new database anchor") = Some(conn);
        Ok(db)
    }
    pub fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(self.path.as_ref()).context("Open database")?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(conn)
    }
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        let permit = self.slots.clone().acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f(&mut db.connect()?)
        })
        .await?
    }
    pub async fn backup(&self, destination: PathBuf) -> Result<()> {
        self.call(move |conn| {
            conn.backup(rusqlite::MAIN_DB, &destination, None)?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn migration_and_online_backup_are_restart_safe() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("main.db"))?;
        db.call(|c| {
            c.execute("INSERT INTO settings VALUES ('timezone', '\"UTC\"')", [])?;
            Ok(())
        })
        .await?;
        db.backup(temp.path().join("snapshot.db")).await?;
        let copy = Database::open(temp.path().join("snapshot.db"))?;
        assert_eq!(
            copy.call(|c| Ok(c.query_row(
                "SELECT value FROM settings WHERE key='timezone'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await?,
            "\"UTC\""
        );
        Database::open(temp.path().join("main.db"))?;
        Ok(())
    }
}
