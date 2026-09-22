//! One bounded Data API page per turn, with a durable cursor and fair ordering.

#[path = "../storage/online/sync.rs"]
mod storage;

use super::{quota, youtube};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{id, now};
#[cfg(test)]
pub(super) mod tests;

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct Cursor {
    phase: String,
    snapshot: String,
    page: String,
    pages: u32,
    after: String,
    channel: String,
    playlist: String,
    ids: Vec<String>,
}
struct Turn {
    user: String,
    generation: String,
    cursor: Cursor,
    failures: u32,
}
pub async fn request(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let changed = storage::request(&state.db, p).await?;
    if !changed {
        return Err(ApiError::conflict(
            "Connect YouTube first, or wait for the current retry delay and five-minute sync cooldown",
        ));
    }
    Ok(Json(json!({"queued":true})))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        let delay = if tick(&state).await? { 250 } else { 2000 };
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
}
pub(super) async fn run_classifications(state: AppState) -> anyhow::Result<()> {
    loop {
        if classify_next(&state).await.is_err() {
            tracing::warn!("YouTube Shorts classification will retry");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
async fn classify_next(state: &AppState) -> anyhow::Result<()> {
    let candidate = storage::claim_classification(&state.db).await?;
    if let Some((user, video, generation)) = candidate {
        let result = short(state, &video).await;
        let recipient = user.clone();
        let changed =
            storage::save_classification(result, user, video, generation, &state.db).await?;
        if changed > 0 && result.is_some() {
            state
                .emit(Some(recipient), "youtube.changed", json!({}))
                .await?;
        }
    }
    Ok(())
}
async fn claim(state: &AppState) -> anyhow::Result<Option<Turn>> {
    storage::claim(&state.db).await
}
async fn tick(state: &AppState) -> anyhow::Result<bool> {
    let Some(turn) = claim(state).await? else {
        return Ok(false);
    };
    let user = turn.user.clone();
    let generation = turn.generation.clone();
    let failures = turn.failures;
    if let Err(error) = step(state, turn).await {
        // Only locally authored messages reach the persisted UI status.
        let message = error.2;
        let blocked = quota::status(state)
            .await
            .map(|q| q["blocked"] == true)
            .unwrap_or(false);
        let delay = if blocked {
            1800
        } else {
            30i64.saturating_mul(1i64 << failures.min(7)).min(3600)
        };
        storage::tick(user, generation, message, delay, &state.db).await?;
    }
    Ok(true)
}
fn items(data: &Value) -> Result<&Vec<Value>> {
    data["items"]
        .as_array()
        .filter(|a| a.len() <= 50)
        .ok_or_else(|| ApiError::bad("YouTube returned invalid list data"))
}
fn next_page(data: &Value, current: &str) -> Result<String> {
    let page = data["nextPageToken"].as_str().unwrap_or("");
    if page.len() > 2048 || (!page.is_empty() && page == current) {
        return Err(ApiError::bad("YouTube returned an invalid page cursor"));
    }
    Ok(page.into())
}
pub(super) fn identifier(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn channel_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| identifier(s, 24) && s.starts_with("UC"))
        .map(str::to_owned)
}
fn text(value: &Value, max: usize) -> String {
    value.as_str().unwrap_or("").chars().take(max).collect()
}
fn timestamp(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp())
        .unwrap_or(0)
}
pub(super) fn duration(value: &str) -> Option<i64> {
    let value = value.strip_prefix('P')?;
    let (date, mut time) = value.split_once('T').unwrap_or((value, ""));
    let days = if date.is_empty() {
        0.0
    } else {
        date.strip_suffix('D')?.parse::<f64>().ok()?
    };
    let mut total = days * 86400.;
    for (unit, multiplier) in [('H', 3600.), ('M', 60.), ('S', 1.)] {
        if let Some(index) = time.find(unit) {
            let n = time[..index].parse::<f64>().ok()?;
            if !n.is_finite() || n < 0. {
                return None;
            }
            total += n * multiplier;
            time = &time[index + 1..];
        }
    }
    (time.is_empty() && days.is_finite() && days >= 0. && total <= 31536000.)
        .then_some(total.round() as i64)
}
async fn save(state: &AppState, turn: Turn, done: bool, page: storage::Page) -> Result<()> {
    let cursor = if done {
        "{}".into()
    } else {
        serde_json::to_string(&turn.cursor).map_err(anyhow::Error::from)?
    };
    let changed = storage::save(cursor, &state.db, turn, done, page).await?;
    if changed {
        state.notify_events();
    }
    Ok(())
}
async fn step(state: &AppState, mut turn: Turn) -> Result<()> {
    let access = youtube::access(state, &turn.user).await?;
    if access.generation != turn.generation {
        return Ok(());
    }
    if turn.cursor.phase.is_empty() {
        turn.cursor.phase = "subscriptions".into();
        turn.cursor.snapshot = id();
    }
    let user = turn.user.clone();
    // URL additions get metadata in a bounded batch on the same fair queue.
    let owner = user.clone();
    let pending = storage::pending_metadata(owner, &state.db).await?;
    if !pending.is_empty() {
        let joined = pending.join(",");
        let data = youtube::get(
            state,
            &user,
            &access,
            "videos",
            &[
                ("part", "snippet,contentDetails,status,liveStreamingDetails"),
                ("id", &joined),
                ("maxResults", "50"),
            ],
        )
        .await?;
        let rows = items(&data)?.clone();
        return save(
            state,
            turn,
            false,
            storage::Page::PendingVideos { pending, rows },
        )
        .await;
    }
    match turn.cursor.phase.as_str() {
        "subscriptions" => {
            let data = youtube::get(
                state,
                &user,
                &access,
                "subscriptions",
                &[
                    ("part", "snippet"),
                    ("mine", "true"),
                    ("maxResults", "50"),
                    ("pageToken", &turn.cursor.page),
                ],
            )
            .await?;
            let subscriptions = items(&data)?;
            let rows = subscriptions
                .iter()
                .filter_map(|v| {
                    Some((
                        channel_id(&v["snippet"]["resourceId"]["channelId"])?,
                        text(&v["snippet"]["title"], 300),
                        super::public_image(
                            v["snippet"]["thumbnails"]["medium"]["url"]
                                .as_str()
                                .or(v["snippet"]["thumbnails"]["default"]["url"].as_str()),
                        ),
                    ))
                })
                .collect::<Vec<_>>();
            if rows.len() != subscriptions.len() {
                return Err(ApiError::conflict(
                    "YouTube returned an incomplete subscription page; existing subscriptions were preserved",
                ));
            }
            let page = next_page(&data, &turn.cursor.page)?;
            turn.cursor.pages += 1;
            if turn.cursor.pages > 200 {
                return Err(ApiError::bad(
                    "Subscription sync exceeded 10,000 channels; reconnect to start a new snapshot",
                ));
            }
            let complete = page.is_empty();
            let snapshot = turn.cursor.snapshot.clone();
            turn.cursor.page = page;
            if complete {
                turn.cursor.phase = "channels".into();
                turn.cursor.after.clear();
            }
            save(
                state,
                turn,
                false,
                storage::Page::Subscriptions {
                    rows,
                    snapshot,
                    complete,
                },
            )
            .await
        }
        "channels" => {
            let owner = user.clone();
            let after = turn.cursor.after.clone();
            let channels = storage::channels_to_refresh(owner, after, &state.db).await?;
            let owner = user.clone();
            let profile = storage::step_read_online_accounts(owner, &state.db)
                .await?
                .filter(|_| channels.len() < 50);
            if channels.is_empty() && profile.is_none() {
                turn.cursor.phase = "uploads".into();
                turn.cursor.after.clear();
                turn.cursor.page.clear();
                turn.cursor.pages = 0;
                return save(state, turn, false, storage::Page::CursorOnly).await;
            }
            // Reuse the channel metadata batch to refresh the connected profile.
            // Existing accounts acquire an avatar without needing to reconnect.
            let mut ids = channels.clone();
            if let Some(profile) = &profile
                && !ids.contains(profile)
            {
                ids.push(profile.clone());
            }
            let ids = ids.join(",");
            let data = youtube::get(
                state,
                &user,
                &access,
                "channels",
                &[
                    ("part", "contentDetails,snippet"),
                    ("id", &ids),
                    ("maxResults", "50"),
                ],
            )
            .await?;
            let rows = items(&data)?
                .iter()
                .filter_map(|v| {
                    Some((
                        channel_id(&v["id"])?,
                        v["contentDetails"]["relatedPlaylists"]["uploads"]
                            .as_str()
                            .filter(|s| {
                                s.len() <= 128
                                    && s.bytes().all(|c| {
                                        c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
                                    })
                            })?
                            .to_owned(),
                    ))
                })
                .collect::<Vec<_>>();
            let own_channel = profile.as_ref().and_then(|id| {
                items(&data)
                    .ok()?
                    .iter()
                    .find(|item| item["id"].as_str() == Some(id.as_str()))
            });
            let avatar = own_channel.and_then(super::channel_avatar);
            let name = own_channel
                .and_then(|channel| channel["snippet"]["title"].as_str())
                .map(|s| s.chars().take(200).collect::<String>());
            let has_profile = own_channel.is_some();
            let refresh_profile = profile.is_some();
            turn.cursor.after = channels.last().cloned().unwrap_or_default();
            if channels.is_empty() {
                turn.cursor.phase = "uploads".into();
                turn.cursor.after.clear();
                turn.cursor.page.clear();
                turn.cursor.pages = 0;
            }
            save(
                state,
                turn,
                false,
                storage::Page::Channels {
                    refresh_profile,
                    name,
                    has_profile,
                    avatar,
                    channels,
                    rows,
                },
            )
            .await?;

            Ok(())
        }
        "uploads" => {
            if turn.cursor.page.is_empty() {
                let owner = user.clone();
                let after = turn.cursor.after.clone();
                let next = storage::next_upload_playlist(owner, after, &state.db).await?;
                let Some((channel, playlist)) = next else {
                    turn.cursor.phase = "refresh".into();
                    turn.cursor.after.clear();
                    return save(state, turn, false, storage::Page::CursorOnly).await;
                };
                turn.cursor.channel = channel;
                turn.cursor.playlist = playlist;
                turn.cursor.pages = 0;
            }
            let data = youtube::get(
                state,
                &user,
                &access,
                "playlistItems",
                &[
                    ("part", "contentDetails"),
                    ("playlistId", &turn.cursor.playlist),
                    ("maxResults", "50"),
                    ("pageToken", &turn.cursor.page),
                ],
            )
            .await?;
            let rows = items(&data)?;
            turn.cursor.ids = rows
                .iter()
                .filter_map(|v| {
                    v["contentDetails"]["videoId"]
                        .as_str()
                        .filter(|s| identifier(s, 11))
                        .map(str::to_owned)
                })
                .collect();
            turn.cursor.pages += 1;
            let page = next_page(&data, &turn.cursor.page)?;
            // Bound initial backfill to 150 uploads / 90 days per channel.
            let old = rows.iter().any(|v| {
                let date = timestamp(&v["contentDetails"]["videoPublishedAt"]);
                date > 0 && date < now() - 90 * 86400
            });
            turn.cursor.page = if turn.cursor.pages < 3 && !old {
                page
            } else {
                String::new()
            };
            if turn.cursor.page.is_empty() {
                turn.cursor.after = turn.cursor.channel.clone();
            }
            turn.cursor.phase = "videos".into();
            save(state, turn, false, storage::Page::CursorOnly).await
        }
        "videos" | "refresh" => {
            let refresh = turn.cursor.phase == "refresh";
            if refresh {
                let owner = user.clone();
                let after = turn.cursor.after.clone();
                turn.cursor.ids = storage::videos_to_refresh(owner, after, &state.db).await?;
                if turn.cursor.ids.is_empty() {
                    turn.cursor.phase = "shorts".into();
                    turn.cursor.pages = 0;
                    return save(state, turn, false, storage::Page::CursorOnly).await;
                }
                turn.cursor.after = turn.cursor.ids.last().unwrap().clone();
            }
            let ids = turn.cursor.ids.clone();
            if ids.is_empty() {
                turn.cursor.phase = "uploads".into();
                return save(state, turn, false, storage::Page::CursorOnly).await;
            }
            let joined = ids.join(",");
            let data = youtube::get(
                state,
                &user,
                &access,
                "videos",
                &[
                    ("part", "snippet,contentDetails,status,liveStreamingDetails"),
                    ("id", &joined),
                    ("maxResults", "50"),
                ],
            )
            .await?;
            let rows = items(&data)?.clone();
            if !refresh {
                turn.cursor.phase = "uploads".into();
            }
            turn.cursor.ids.clear();
            save(state, turn, false, storage::Page::Videos { ids, rows }).await
        }
        "shorts" => save(state, turn, true, storage::Page::CursorOnly).await,
        _ => Err(ApiError::bad("Invalid persisted YouTube sync cursor")),
    }
}
async fn short(state: &AppState, video: &str) -> Option<bool> {
    // A duration alone cannot distinguish a Short from a regular short video.
    // This public HEAD request carries no viewer token and follows no redirects.
    let response = state
        .online
        .http
        .head(format!("https://www.youtube.com/shorts/{video}"))
        .send()
        .await
        .ok()?;
    if response.status().is_success() {
        return Some(true);
    }
    if !response.status().is_redirection() {
        return None;
    }
    let base = url::Url::parse("https://www.youtube.com").ok()?;
    let target = base
        .join(
            response
                .headers()
                .get(axum::http::header::LOCATION)?
                .to_str()
                .ok()?,
        )
        .ok()?;
    (matches!(target.host_str(), Some("www.youtube.com" | "youtube.com"))
        && target.path() == "/watch"
        && target.query_pairs().any(|(k, v)| k == "v" && v == video))
    .then_some(false)
}
