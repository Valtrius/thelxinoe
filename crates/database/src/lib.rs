mod runtime;
pub use runtime::{AccessError, Metrics, Options, WorkStats};

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

/// Shared database runtime. Domain storage modules own all SQL and call the
/// explicitly read-only or write path. Accepted operations run through commit,
/// even if the caller stops waiting; shutdown drains both bounded queues.
#[derive(Clone)]
pub struct Database {
    runtime: Arc<runtime::Runtime>,
    #[cfg(test)]
    path: Arc<PathBuf>,
}

fn connection(path: &Path, read_only: bool) -> Result<Connection> {
    let flags = if read_only {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
    };
    let conn = Connection::open_with_flags(path, flags).context("Open database")?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    if read_only {
        conn.pragma_update(None, "query_only", "ON")?;
    }
    conn.set_prepared_statement_cache_capacity(64);
    Ok(conn)
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, Options::default())
    }

    pub fn open_with_options(path: impl AsRef<Path>, options: Options) -> Result<Self> {
        anyhow::ensure!(
            (1..=16).contains(&options.readers),
            "Expected 1 to 16 database readers"
        );
        anyhow::ensure!(
            (1..=4096).contains(&options.queue_capacity),
            "Expected a database queue capacity of 1 to 4096"
        );
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = connection(path, false)?;
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
        let readers = (0..options.readers)
            .map(|_| connection(path, true))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            runtime: Arc::new(runtime::Runtime {
                writer: runtime::Lane::start("writer", vec![conn], &options)?,
                readers: runtime::Lane::start("reader", readers, &options)?,
            }),
            #[cfg(test)]
            path: Arc::new(path.to_owned()),
        })
    }

    /// Execute a named storage query in one consistent read-only snapshot.
    pub async fn read<T, F>(&self, operation: &'static str, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        self.runtime
            .readers
            .submit(operation, move |conn| {
                let tx = conn.transaction()?;
                let value = f(&tx)?;
                tx.commit()?;
                Ok(value)
            })
            .await
    }

    /// Execute a complete storage operation on the sole writer connection.
    /// Multi-statement changes must use one transaction within this closure.
    pub async fn write<T, F>(&self, operation: &'static str, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        self.runtime.writer.submit(operation, f).await
    }

    pub fn metrics(&self) -> Metrics {
        self.runtime.metrics()
    }

    /// Reject new work, drain accepted operations, and join the database threads.
    /// Safe to call through any clone, repeatedly, or after a cancelled shutdown.
    pub async fn shutdown(&self) -> Result<()> {
        self.runtime.writer.close();
        self.runtime.readers.close();
        let (writes, reads) = tokio::join!(self.runtime.writer.join(), self.runtime.readers.join());
        writes?;
        reads
    }

    pub async fn backup(&self, destination: PathBuf) -> Result<()> {
        self.read("database.backup", move |conn| {
            anyhow::ensure!(!destination.exists(), "Backup destination already exists");
            conn.backup(rusqlite::MAIN_DB, &destination, None)?;
            verify_snapshot(&destination)?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    fn connect(&self) -> Result<Connection> {
        connection(&self.path, false)
    }
}

#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod tests;
