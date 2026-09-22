#[path = "../storage/online/quota.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
fn day() -> String {
    Utc::now()
        .with_timezone(&chrono_tz::America::Los_Angeles)
        .date_naive()
        .to_string()
}
pub async fn block(state: &AppState) -> Result<()> {
    let today = day();
    storage::block(today, &state.db).await?;
    Ok(())
}
pub async fn status(state: &AppState) -> Result<Value> {
    let today = day();
    Ok(storage::status(today, &state.db).await?)
}
// Reserve before sending. Failed and retried requests also consume provider
// quota. The scheduler will take one bounded page per user on each turn.
pub async fn reserve(state: &AppState, user: &str) -> Result<()> {
    let today = day();
    let user = user.to_owned();
    let allowed = storage::reserve(today, user, &state.db).await?;
    if !allowed {
        return Err(ApiError::conflict(
            "YouTube's shared daily API budget is exhausted; synchronization resumes after midnight Pacific time",
        ));
    }
    Ok(())
}
