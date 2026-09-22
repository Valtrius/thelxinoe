//! User-owned minute activity, shared by web, native, music and compatibility playback.

#[path = "storage/statistics.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    playback::Progress,
    security,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use thelxinoe_core::Capability;

pub(crate) const PLATFORMS: [&str; 6] = ["youtube", "twitch", "kick", "movies", "shows", "music"];
pub(crate) const RANGES: [&str; 4] = ["7d", "30d", "90d", "all"];

pub(crate) fn validate_scope(range: &str, platform: &str) -> Result<()> {
    if !RANGES.contains(&range) || (platform != "all" && !PLATFORMS.contains(&platform)) {
        return Err(ApiError::bad("Unknown range or platform"));
    }
    Ok(())
}

pub(crate) fn range_cutoff(zone: Tz, range: &str, now: DateTime<Utc>) -> Option<i64> {
    let days = match range {
        "7d" => Some(7),
        "30d" => Some(30),
        "90d" => Some(90),
        _ => None,
    };
    days.map(|days| {
        let today = now.with_timezone(&zone).date_naive();
        let start = today - Duration::days(days - 1);
        let mut local = start.and_hms_opt(0, 0, 0).unwrap();
        loop {
            match zone.from_local_datetime(&local) {
                LocalResult::Single(value) => break value.timestamp(),
                LocalResult::Ambiguous(first, second) => {
                    break first.timestamp().min(second.timestamp());
                }
                LocalResult::None => local += Duration::minutes(1),
            }
        }
    })
}

#[derive(Debug)]
struct Snapshot {
    platform: String,
    title: String,
    channel: String,
    name: String,
    category: String,
    content_type: String,
}

pub(crate) use storage::record;

#[cfg(test)]
use storage::record_at;

pub(crate) use storage::delete_provider;

#[derive(Deserialize, Default)]
pub struct Filter {
    pub(crate) range: Option<String>,
    pub(crate) platform: Option<String>,
    pub(crate) user: Option<String>,
}

pub async fn mine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    overview(&state, Some(p.user.id), &p.user.timezone, filter, false)
        .await
        .map(Json)
}
pub async fn admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::InspectHistory).await?;
    overview(&state, filter.user.clone(), &p.user.timezone, filter, true)
        .await
        .map(Json)
}
async fn overview(
    state: &AppState,
    user: Option<String>,
    zone: &str,
    filter: Filter,
    admin: bool,
) -> Result<Value> {
    let range = filter.range.unwrap_or_else(|| "30d".into());
    let platform = filter.platform.unwrap_or_else(|| "all".into());
    validate_scope(&range, &platform)?;
    let zone: Tz = zone
        .parse()
        .map_err(|_| ApiError::bad("Unknown display timezone"))?;
    Ok(storage::overview(range, platform, zone, &state.db, user, admin).await?)
}

#[derive(Default)]
struct Channel {
    name: String,
    seconds: f64,
    days: BTreeSet<NaiveDate>,
    media: HashSet<String>,
}
fn series(seconds: &[f64; 6]) -> Value {
    json!({"youtubeSeconds":seconds[0],"twitchSeconds":seconds[1],"kickSeconds":seconds[2],"moviesSeconds":seconds[3],"showsSeconds":seconds[4],"musicSeconds":seconds[5]})
}
fn period(date: NaiveDate, interval: &str) -> NaiveDate {
    match interval {
        "week" => date - Duration::days(date.weekday().num_days_from_monday().into()),
        "month" => date.with_day(1).unwrap(),
        _ => date,
    }
}
fn next_period(date: NaiveDate, interval: &str) -> NaiveDate {
    match interval {
        "week" => date + Duration::days(7),
        "month" => (date + Duration::days(32)).with_day(1).unwrap(),
        _ => date + Duration::days(1),
    }
}

#[cfg(test)]
use storage::aggregate;

#[cfg(test)]
mod tests;
