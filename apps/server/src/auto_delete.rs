#[path = "storage/auto_delete.rs"]
mod storage;
pub(crate) use storage::{make_room, remove_ready};

use crate::{
    AppState,
    error::{ApiError, Result},
};
use serde::Serialize;
use serde_json::{Value, json};
use thelxinoe_database::Database;

/// The smallest unpinned storage limit, which still holds hours of 8K video, so a single
/// download is never too large to keep.
pub(crate) const MIN_STORAGE_LIMIT_BYTES: i64 = 40_000_000_000;

#[derive(Default, Serialize)]
pub(crate) struct VideoUsage {
    unpinned_bytes: i64,
    pinned_bytes: i64,
    waiting: i64,
}

pub(crate) async fn list(db: &Database) -> anyhow::Result<(Vec<Value>, VideoUsage)> {
    storage::list(db).await
}

pub(crate) async fn evaluate(state: &AppState) -> anyhow::Result<()> {
    let changed = storage::evaluate(&state.db, state.config.cache.join("youtube")).await?;
    if changed > 0 {
        state.emit(None, "retention.changed", json!({})).await?;
        state
            .emit(None, "online.download.changed", json!({}))
            .await?;
    }
    Ok(())
}

pub(crate) async fn admit(state: &AppState, video: &str, generation: &str) -> anyhow::Result<bool> {
    storage::admit(&state.db, video.to_owned(), generation.to_owned()).await
}

/// Changes when admitting a download that waits for space may have a different outcome.
pub(crate) async fn capacity(db: &Database) -> anyhow::Result<[i64; 4]> {
    storage::capacity(db).await
}

pub(crate) async fn budget(db: &Database, video: String) -> anyhow::Result<Option<i64>> {
    storage::budget(db, video).await
}

pub(crate) async fn action(state: &AppState, key: &str, action: &str, actor: &str) -> Result<bool> {
    storage::action(
        &state.db,
        state.config.cache.join("youtube"),
        key.to_owned(),
        action.to_owned(),
        actor.to_owned(),
    )
    .await
    .map_err(|error| ApiError::conflict(error.to_string()))
}
