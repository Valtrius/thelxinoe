use super::models::Registry;
use crate::error::{AppError, AppResult};
use rusqlite::{Connection, OptionalExtension};
use std::{fs, path::Path};

pub fn load(root: &Path) -> AppResult<Registry> {
    let connection = connect(root)?;
    let value: Option<String> = connection
        .query_row("SELECT value FROM manager_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|s| serde_json::from_str(&s).map_err(|e| AppError::internal(e.to_string())))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn save(root: &Path, state: &Registry) -> AppResult<()> {
    let value =
        serde_json::to_string(state).map_err(|error| AppError::internal(error.to_string()))?;
    let connection = connect(root)?;
    connection.execute(
        "INSERT INTO manager_state(id, value) VALUES(1, ?1) \
         ON CONFLICT(id) DO UPDATE SET value = excluded.value",
        [value],
    )?;
    Ok(())
}
fn connect(root: &Path) -> AppResult<Connection> {
    fs::create_dir_all(root)?;
    let conn = Connection::open(root.join("tools.db"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA busy_timeout=5000;
         CREATE TABLE IF NOT EXISTS manager_state(
             id INTEGER PRIMARY KEY CHECK(id=1),
             value TEXT NOT NULL
         );",
    )?;
    Ok(conn)
}
