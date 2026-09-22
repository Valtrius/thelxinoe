use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Fresh schema identity, also used by signed releases and backups.
pub const SCHEMA_VERSION: u32 = 1;
const APPLICATION_ID: u32 = 0x544c584e; // TLXN
const SCHEMA: &str = include_str!("../schema.sql");

/// Inspect a quiesced database without changing it or creating missing files.
pub fn verify_snapshot(path: &Path) -> Result<u32> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let checks = conn
        .prepare("PRAGMA integrity_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(checks == ["ok"], "Database integrity check failed");
    anyhow::ensure!(
        !conn.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Database has invalid references"
    );
    let application: u32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let schema: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(
        application == APPLICATION_ID && schema > 0,
        "Not a Thelxinoe database"
    );
    Ok(schema)
}

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
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let application: u32 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let empty = !tx
            .prepare("SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")?
            .exists([])?;
        if empty && application == 0 && version == 0 {
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else {
            anyhow::ensure!(
                !empty && application == APPLICATION_ID && version == SCHEMA_VERSION,
                "Incompatible database. Use the matching release, or stop Thelxinoe and reset its local database to start fresh."
            );
        }
        tx.commit()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
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
            anyhow::ensure!(!destination.exists(), "Backup destination already exists");
            conn.backup(rusqlite::MAIN_DB, &destination, None)?;
            verify_snapshot(&destination)?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests;
