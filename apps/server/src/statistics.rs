//! User-owned minute activity, shared by web, native, music and compatibility playback.
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
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use thelxinoe_core::Capability;

const PLATFORMS: [&str; 6] = ["youtube", "twitch", "kick", "movies", "shows", "music"];

#[derive(Debug)]
struct Snapshot {
    platform: String,
    title: String,
    channel: String,
    name: String,
    category: String,
    content_type: String,
}

pub(crate) fn record(
    tx: &rusqlite::Transaction<'_>,
    playback: &str,
    input: &Progress,
) -> anyhow::Result<f64> {
    record_at(tx, playback, input, Utc::now().timestamp_millis())
}

fn record_at(
    tx: &rusqlite::Transaction<'_>,
    playback: &str,
    input: &Progress,
    at: i64,
) -> anyhow::Result<f64> {
    let (user, media, duration, created, old_position, old_state): (String,String,f64,i64,f64,String) = tx.query_row(
        "SELECT user_id,COALESCE(media_id,'youtube:'||youtube_video_id,live_media_id),duration,created_at,position,state FROM playback_sessions WHERE id=?1", [playback],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    )?;
    let previous: Option<(i64,Option<f64>)> = tx.query_row(
        "SELECT reported_at_ms,client_active_seconds FROM playback_activity_clocks WHERE playback_id=?1", [playback], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional()?;
    let since = previous
        .map(|p| p.0)
        .unwrap_or(created * 1000)
        .max(at - 30_000)
        .min(at);
    let elapsed = (at - since) as f64 / 1000.0;
    let advanced = input.position - old_position;
    let seconds = if let Some(counter) = input.active_seconds {
        // Cumulative counters survive duplicate reports and request retries.
        // Bound them by server wall time, so a seek cannot create watch time.
        (counter - previous.and_then(|p| p.1).unwrap_or(0.0)).clamp(0.0, elapsed)
    } else if previous.is_some()
        && old_state == "playing"
        && advanced >= 0.0
        && advanced <= elapsed * 2.0 + 2.0
    {
        // Clients using the compatibility API only provide playhead samples.
        advanced.min(elapsed)
    } else {
        0.0
    };
    let counter = input
        .active_seconds
        .map(|value| value.max(previous.and_then(|p| p.1).unwrap_or(0.0)));
    tx.execute("INSERT INTO playback_activity_clocks VALUES (?1,?2,?3) ON CONFLICT(playback_id) DO UPDATE SET reported_at_ms=excluded.reported_at_ms,client_active_seconds=excluded.client_active_seconds", params![playback,at,counter])?;

    // Prepared/prefetched or cancelled media has not been played. In particular,
    // restoring a nonzero resume position alone does not count as a new start.
    if input.state != "playing"
        && seconds == 0.0
        && !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM playback_statistics WHERE user_id=?1 AND media_id=?2)",
            params![user, media],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Ok(0.0);
    }
    let snapshot = tx.query_row(
        "SELECT platform,media_title,COALESCE(NULLIF(channel_id,''),media_id),COALESCE(NULLIF(channel_name,''),media_title),category_name,content_type FROM statistics_metadata WHERE media_id=?1 AND (user_id=?2 OR user_id IS NULL)", params![media,user],
        |r| Ok(Snapshot {platform:r.get(0)?,title:r.get(1)?,channel:r.get(2)?,name:r.get(3)?,category:r.get(4)?,content_type:r.get(5)?}),
    ).optional()?.or(tx.query_row(
        "SELECT platform,media_title,channel_id,channel_name,category_name,content_type FROM playback_statistics WHERE user_id=?1 AND media_id=?2",params![user,media],
        |r| Ok(Snapshot {platform:r.get(0)?,title:r.get(1)?,channel:r.get(2)?,name:r.get(3)?,category:r.get(4)?,content_type:r.get(5)?}),
    ).optional()?);
    let Some(s) = snapshot else {
        return Ok(seconds);
    };
    let timestamp = at / 1000;
    tx.execute("INSERT INTO playback_statistics VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11)
        ON CONFLICT(user_id,platform,media_id) DO UPDATE SET last_played_at=excluded.last_played_at,media_title=excluded.media_title,channel_id=excluded.channel_id,channel_name=excluded.channel_name,category_name=excluded.category_name,content_type=excluded.content_type,duration=excluded.duration,completed=MAX(completed,excluded.completed)",
        params![user,s.platform,media,s.title,s.channel,s.name,s.category,s.content_type,timestamp,duration,duration>0.0 && input.position>=duration*0.9])?;
    // Split intervals crossing minutes, hours and local dates instead of
    // attributing an entire long session to the day on which it started.
    let mut cursor = since;
    while seconds > 0.0 && cursor < at {
        let bucket = cursor.div_euclid(60_000) * 60;
        let end = ((bucket + 60) * 1000).min(at);
        let part = seconds * (end - cursor) as f64 / (at - since) as f64;
        tx.execute("INSERT INTO playback_activity(user_id,bucket_started_at,platform,media_id,active_seconds,media_title,channel_id,channel_name,category_name,content_type)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
            ON CONFLICT(user_id,bucket_started_at,platform,media_id) DO UPDATE SET active_seconds=active_seconds+excluded.active_seconds,media_title=excluded.media_title,channel_id=excluded.channel_id,channel_name=excluded.channel_name,category_name=excluded.category_name,content_type=excluded.content_type",
            params![user,bucket,s.platform,media,part,s.title,s.channel,s.name,s.category,s.content_type])?;
        cursor = end;
    }
    Ok(seconds)
}

pub(crate) fn delete_provider(
    tx: &rusqlite::Transaction<'_>,
    user: &str,
    provider: &str,
) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM playback_activity WHERE user_id=?1 AND platform=?2",
        params![user, provider],
    )?;
    tx.execute(
        "DELETE FROM playback_statistics WHERE user_id=?1 AND platform=?2",
        params![user, provider],
    )?;
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct Filter {
    range: Option<String>,
    platform: Option<String>,
    user: Option<String>,
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
    if !["7d", "30d", "90d", "all"].contains(&range.as_str())
        || (platform != "all" && !PLATFORMS.contains(&platform.as_str()))
    {
        return Err(ApiError::bad("Unknown statistics range or platform"));
    }
    let zone: Tz = zone
        .parse()
        .map_err(|_| ApiError::bad("Unknown display timezone"))?;
    Ok(state
        .db
        .call(move |db| {
            aggregate(
                db,
                user.as_deref(),
                zone,
                &range,
                &platform,
                admin,
                Utc::now(),
            )
        })
        .await?)
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

fn aggregate(
    db: &rusqlite::Connection,
    user: Option<&str>,
    zone: Tz,
    range: &str,
    platform: &str,
    admin: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<Value> {
    let today = now.with_timezone(&zone).date_naive();
    let days = match range {
        "7d" => Some(7),
        "30d" => Some(30),
        "90d" => Some(90),
        _ => None,
    };
    let start = days.map(|days| today - Duration::days(days - 1));
    // Bound SQLite's scan conservatively, then compare local dates below. This
    // also handles zones whose midnight is ambiguous or skipped during DST.
    let cutoff = start.map(|date| {
        (date - Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    });
    let interval = match range {
        "90d" => "week",
        "all" => "month",
        _ => "day",
    };
    let tracking: Option<i64> = db.query_row("SELECT MIN(bucket_started_at) FROM playback_activity WHERE (?1 IS NULL OR user_id=?1) AND (?2='all' OR platform=?2)",params![user,platform],|r|r.get(0))?;
    let mut activity = BTreeMap::<NaiveDate, [f64; 6]>::new();
    let mut rhythm = [[0.0; 6]; 168];
    let mut totals = [0.0; 6];
    let mut active_days = BTreeSet::new();
    let mut channels = HashMap::<(String, String), Channel>::new();
    let mut people = BTreeMap::<String, f64>::new();
    let mut streams = [HashSet::<String>::new(), HashSet::<String>::new()];
    let mut estimated = 0.0;
    let mut statement = db.prepare("SELECT bucket_started_at,platform,media_id,active_seconds,estimated_seconds,channel_id,channel_name,user_id FROM playback_activity WHERE (?1 IS NULL OR user_id=?1) AND (?2='all' OR platform=?2) AND (?3 IS NULL OR bucket_started_at>=?3) AND bucket_started_at<=?4 ORDER BY bucket_started_at")?;
    let mut rows = statement.query(params![user, platform, cutoff, now.timestamp()])?;
    while let Some(row) = rows.next()? {
        let timestamp: i64 = row.get(0)?;
        let Some(local) = DateTime::from_timestamp(timestamp, 0).map(|t| t.with_timezone(&zone))
        else {
            continue;
        };
        let date = local.date_naive();
        if start.is_some_and(|start| date < start) || date > today {
            continue;
        }
        let source: String = row.get(1)?;
        let Some(index) = PLATFORMS.iter().position(|p| *p == source) else {
            continue;
        };
        let media: String = row.get(2)?;
        let seconds: f64 = row.get(3)?;
        if seconds <= 0.0 {
            continue;
        }
        totals[index] += seconds;
        estimated += row.get::<_, f64>(4)?;
        active_days.insert(date);
        activity.entry(period(date, interval)).or_default()[index] += seconds;
        rhythm[local.weekday().num_days_from_monday() as usize * 24 + local.hour() as usize]
            [index] += seconds;
        if index == 1 || index == 2 {
            streams[index - 1].insert(media.clone());
        }
        let channel = channels.entry((source, row.get(5)?)).or_default();
        channel.name = row.get(6)?;
        channel.seconds += seconds;
        channel.days.insert(date);
        channel.media.insert(media);
        *people.entry(row.get(7)?).or_default() += seconds;
    }
    let period_days = days.unwrap_or_else(|| {
        active_days
            .first()
            .map(|first| (today - *first).num_days() + 1)
            .unwrap_or(0)
    });
    if let Some(mut cursor) = start
        .or_else(|| active_days.first().copied())
        .map(|day| period(day, interval))
    {
        let end = period(today, interval);
        while cursor <= end {
            activity.entry(cursor).or_default();
            cursor = next_period(cursor, interval);
        }
    }
    let activity: Vec<_> = activity
        .into_iter()
        .map(|(date, seconds)| {
            let mut point = series(&seconds);
            point["periodStart"] = json!(date.to_string());
            point
        })
        .collect();
    let rhythm: Vec<_> = rhythm
        .into_iter()
        .enumerate()
        .map(|(index, seconds)| {
            let mut point = series(&seconds);
            point["weekday"] = json!(index / 24);
            point["hour"] = json!(index % 24);
            point
        })
        .collect();
    let mut top: Vec<_> = channels.into_iter().map(|((source,id),c)|json!({"platform":source,"id":id,"name":c.name,"activeSeconds":c.seconds,"watchedDays":c.days.len(),"contentCount":c.media.len()})).collect();
    top.sort_by(|a, b| {
        b["activeSeconds"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["activeSeconds"].as_f64().unwrap())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let mut started = [0; 6];
    let mut completed = [0; 6];
    let mut mix = [0; 3];
    let mut groups: [HashSet<String>; 6] = Default::default();
    let mut albums = HashSet::new();
    let mut statement = db.prepare("SELECT s.platform,CASE WHEN s.platform='youtube' THEN s.first_played_at ELSE s.last_played_at END,s.content_type,COALESCE(y.watched,m.watched,s.completed),s.channel_id,s.category_name
        FROM playback_statistics s LEFT JOIN youtube_state y ON y.user_id=s.user_id AND 'youtube:'||y.video_id=s.media_id
        LEFT JOIN media_state m ON m.user_id=s.user_id AND m.media_id=s.media_id
        WHERE (?1 IS NULL OR s.user_id=?1) AND (?2='all' OR s.platform=?2) AND (?3 IS NULL OR CASE WHEN s.platform='youtube' THEN s.first_played_at ELSE s.last_played_at END>=?3) AND s.last_played_at<=?4")?;
    let mut rows = statement.query(params![user, platform, cutoff, now.timestamp()])?;
    while let Some(row) = rows.next()? {
        let Some(date) =
            DateTime::from_timestamp(row.get(1)?, 0).map(|t| t.with_timezone(&zone).date_naive())
        else {
            continue;
        };
        if start.is_some_and(|start| date < start) {
            continue;
        }
        let source: String = row.get(0)?;
        let Some(index) = PLATFORMS.iter().position(|p| *p == source) else {
            continue;
        };
        let kind: String = row.get(2)?;
        if index == 0 && ["live", "upcoming"].contains(&kind.as_str()) {
            continue;
        }
        started[index] += 1;
        groups[index].insert(row.get::<_, String>(4)?);
        if index == 5 {
            albums.insert((row.get::<_, String>(4)?, row.get::<_, String>(5)?));
        }
        if row.get::<_, bool>(3)? {
            completed[index] += 1;
            if index == 0 {
                mix[match kind.as_str() {
                    "short" => 2,
                    "live_replay" => 1,
                    _ => 0,
                }] += 1;
            }
        }
    }
    let mut users = Vec::new();
    if admin {
        for (id, seconds) in people {
            let name: String =
                db.query_row("SELECT username FROM users WHERE id=?1", [&id], |r| {
                    r.get(0)
                })?;
            users.push(json!({"id":id,"username":name,"activeSeconds":seconds}));
        }
        users.sort_by(|a, b| {
            b["activeSeconds"]
                .as_f64()
                .unwrap()
                .total_cmp(&a["activeSeconds"].as_f64().unwrap())
        });
    }
    let total: f64 = totals.iter().sum();
    Ok(
        json!({"range":range,"platform":platform,"timezone":zone.name(),"interval":interval,
        "trackingStartedAt":tracking.and_then(|t|DateTime::from_timestamp(t,0)).map(|t|t.to_rfc3339()),
        "totalActiveSeconds":total,"estimatedActiveSeconds":estimated,"activeDays":active_days.len(),"periodDays":period_days,
        "averageActiveSecondsPerDay":if active_days.is_empty(){0.0}else{total/active_days.len() as f64},
        "youtubeVideosStarted":started[0],"youtubeVideosWatched":completed[0],"twitchChannelsWatched":streams[0].len(),"kickChannelsWatched":streams[1].len(),
        "moviesStarted":started[3],"moviesWatched":completed[3],"episodesStarted":started[4],"episodesWatched":completed[4],"showsWatched":groups[4].len(),
        "tracksStarted":started[5],"tracksCompleted":completed[5],"artistsListened":groups[5].len(),"albumsListened":albums.len(),
        "activity":activity,"rhythm":rhythm,"platformTotals":PLATFORMS.iter().enumerate().map(|(i,p)|json!({"platform":p,"activeSeconds":totals[i]})).collect::<Vec<_>>(),
        "topChannels":top,"youtubeContentMix":{"uploads":mix[0],"liveReplays":mix[1],"shorts":mix[2]},"users":users}),
    )
}

#[cfg(test)]
mod tests;
