//! The subscription feed contract shared by the provider presentation components.

#[path = "../storage/online/browse.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{Connection, params, params_from_iter, types::Value as SqlValue};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

pub(super) fn date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

pub(super) use storage::one;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    search: String,
    watch_states: Vec<String>,
    include_shorts: bool,
    include_live: bool,
    include_live_replays: bool,
    include_upcoming: bool,
    channel_id: Option<String>,
    duration_filter: String,
    published_filter: String,
    sort_field: String,
    sort_direction: String,
    grouping: String,
    download_filter: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    query: Query,
    page: u32,
    page_size: u32,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let q = &input.query;
    if input.page > 2000
        || !(1..=100).contains(&input.page_size)
        || q.search.len() > 200
        || q.watch_states.is_empty()
        || q.watch_states.len() > 3
        || q.watch_states
            .iter()
            .any(|s| !["unwatched", "in_progress", "watched"].contains(&s.as_str()))
        || !["any", "under_10", "10_plus", "30_plus"].contains(&q.duration_filter.as_str())
        || !["any", "7_days", "30_days"].contains(&q.published_filter.as_str())
        || !["date", "channel", "duration"].contains(&q.sort_field.as_str())
        || !["asc", "desc"].contains(&q.sort_direction.as_str())
        || !["smart", "day", "week", "month", "none"].contains(&q.grouping.as_str())
        || !["all", "downloaded"].contains(&q.download_filter.as_str())
        || q.channel_id.as_ref().is_some_and(|s| s.len() > 24)
    {
        return Err(ApiError::bad("Invalid YouTube feed filters"));
    }
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    let output = storage::list(&state.db, input, p, grant).await?;
    Ok(Json(output))
}
