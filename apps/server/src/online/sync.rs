//! One bounded Data API page per turn, with a durable cursor and fair ordering.
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
    let changed=state.db.call(move|db|Ok(db.execute("UPDATE youtube_sync SET next_run=?1 WHERE user_id=?2 AND (last_complete IS NULL OR last_complete<?1-300) AND failures=0 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=youtube_sync.user_id AND a.provider='youtube' AND a.status='connected' AND a.generation=youtube_sync.generation)",params![now(),p.user.id])?==1)).await?;
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
    let candidate=state.db.call(|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT v.user_id,v.video_id,a.generation FROM youtube_videos v JOIN online_accounts a ON a.user_id=v.user_id AND a.provider='youtube' AND a.status='connected' WHERE v.available=1 AND v.privacy='public' AND v.broadcast='none' AND v.is_short IS NULL AND v.duration BETWEEN 1 AND 180 AND v.short_checked<?1 ORDER BY v.short_checked,v.published_at DESC LIMIT 1",[now()-86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((user,video,_))=&row {tx.execute("UPDATE youtube_videos SET short_checked=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;}
        tx.commit()?;Ok(row)
    }).await?;
    if let Some((user, video, generation)) = candidate {
        let result = short(state, &video).await;
        let recipient = user.clone();
        let changed=state.db.call(move|db|Ok(db.execute("UPDATE youtube_videos SET is_short=?1,short_checked=?2 WHERE user_id=?3 AND video_id=?4 AND EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?3 AND provider='youtube' AND status='connected' AND generation=?5)",params![result,if result.is_some(){now()}else{now()-86400+300},user,video,generation])?)).await?;
        if changed > 0 && result.is_some() {
            state
                .emit(Some(recipient), "youtube.changed", json!({}))
                .await?;
        }
    }
    Ok(())
}
async fn claim(state: &AppState) -> anyhow::Result<Option<Turn>> {
    state.db.call(|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let turn=tx.query_row("SELECT y.user_id,y.generation,y.cursor,y.failures FROM youtube_sync y JOIN online_accounts a ON a.user_id=y.user_id AND a.provider='youtube' AND a.generation=y.generation AND a.status='connected' WHERE y.next_run<=?1 ORDER BY y.last_turn,y.user_id LIMIT 1",[now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?))).optional()?;
        let turn=turn.map(|(user,generation,cursor,failures)|->anyhow::Result<Turn>{Ok(Turn{user,generation,cursor:serde_json::from_str(&cursor)?,failures})}).transpose()?;
        if let Some(turn)=&turn {
            // Crash recovery: the lease expires without resetting the cursor.
            tx.execute("UPDATE youtube_sync SET next_run=?1,last_turn=(SELECT COALESCE(MAX(last_turn),0)+1 FROM youtube_sync) WHERE user_id=?2",params![now()+90,turn.user])?;
        }
        tx.commit()?;Ok(turn)
    }).await
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
        state.db.call(move|db|{db.execute("UPDATE youtube_sync SET next_run=?1,failures=failures+1,error=?2 WHERE user_id=?3 AND generation=?4",params![now()+delay,message,user,generation])?;Ok(())}).await?;
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
async fn save(
    state: &AppState,
    turn: Turn,
    done: bool,
    write: impl FnOnce(&rusqlite::Transaction<'_>) -> anyhow::Result<()> + Send + 'static,
) -> Result<()> {
    let cursor = if done {
        "{}".into()
    } else {
        serde_json::to_string(&turn.cursor).map_err(anyhow::Error::from)?
    };
    let user = turn.user.clone();
    let changed=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let active=tx.query_row("SELECT EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND status='connected' AND generation=?2)",params![turn.user,turn.generation],|r|r.get::<_,bool>(0))?;
        if !active {return Ok(false);}
        write(&tx)?;
        tx.execute("UPDATE youtube_sync SET cursor=?1,next_run=?2,last_complete=CASE WHEN ?3 THEN ?4 ELSE last_complete END,failures=0,error=NULL WHERE user_id=?5 AND generation=?6",params![cursor,if done{now()+1800}else{now()},done,now(),turn.user,turn.generation])?;
        tx.commit()?;Ok(true)
    }).await?;
    if changed {
        state
            .emit(Some(user), "youtube.changed", json!({"complete":done}))
            .await?;
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
    let pending=state.db.call(move|db|Ok(db.prepare("SELECT v.video_id FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id) WHERE v.user_id=?1 AND v.metadata_at=0 ORDER BY s.added_at,v.video_id LIMIT 50")?.query_map([owner],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
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
        return save(state,turn,false,move|tx|{
            for video in &pending {tx.execute("UPDATE youtube_videos SET available=0,metadata_at=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;}
            for video in rows {upsert_video(tx,&user,&video,&pending)?;}
            Ok(())
        }).await;
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
            save(state,turn,false,move|tx|{
                for (channel,title,thumbnail) in rows {tx.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,thumbnail_url) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(user_id,channel_id) DO UPDATE SET title=excluded.title,snapshot=excluded.snapshot,thumbnail_url=excluded.thumbnail_url",params![user,channel,title,snapshot,thumbnail])?;}
                if complete {
                    tx.execute("DELETE FROM youtube_subscriptions WHERE user_id=?1 AND snapshot<>?2",params![user,snapshot])?;
                    tx.execute("UPDATE youtube_subscriptions SET active=1 WHERE user_id=?1 AND snapshot=?2",params![user,snapshot])?;
                }
                Ok(())
            }).await
        }
        "channels" => {
            let owner = user.clone();
            let after = turn.cursor.after.clone();
            let channels=state.db.call(move|db|Ok(db.prepare("SELECT channel_id FROM youtube_subscriptions WHERE user_id=?1 AND active=1 AND channel_id>?2 ORDER BY channel_id LIMIT 50")?.query_map(params![owner,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
            let owner = user.clone();
            let profile = state.db.call(move |db| {
                Ok(db.query_row("SELECT external_id FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND external_id<>'' AND profile_checked_at<?2", params![owner, now()-86400], |row| row.get::<_, String>(0)).optional()?)
            }).await?.filter(|_| channels.len() < 50);
            if channels.is_empty() && profile.is_none() {
                turn.cursor.phase = "uploads".into();
                turn.cursor.after.clear();
                turn.cursor.page.clear();
                turn.cursor.pages = 0;
                return save(state, turn, false, |_| Ok(())).await;
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
            let owner = user.clone();
            turn.cursor.after = channels.last().cloned().unwrap_or_default();
            if channels.is_empty() {
                turn.cursor.phase = "uploads".into();
                turn.cursor.after.clear();
                turn.cursor.page.clear();
                turn.cursor.pages = 0;
            }
            save(state,turn,false,move|tx|{
                if refresh_profile {
                    tx.execute("UPDATE online_accounts SET profile_checked_at=?1,display_name=COALESCE(?2,display_name),avatar_url=CASE WHEN ?3 THEN ?4 ELSE avatar_url END WHERE user_id=?5 AND provider='youtube'",params![now(),name,has_profile,avatar,user])?;
                }
                for channel in channels {tx.execute("UPDATE youtube_subscriptions SET uploads=NULL WHERE user_id=?1 AND channel_id=?2",params![user,channel])?;}
                for (channel,playlist) in rows {tx.execute("UPDATE youtube_subscriptions SET uploads=?1 WHERE user_id=?2 AND channel_id=?3",params![playlist,user,channel])?;}
                Ok(())
            }).await?;
            if refresh_profile {
                state
                    .emit(
                        Some(owner),
                        "online.account.changed",
                        json!({"provider":"youtube"}),
                    )
                    .await?;
            }
            Ok(())
        }
        "uploads" => {
            if turn.cursor.page.is_empty() {
                let owner = user.clone();
                let after = turn.cursor.after.clone();
                let next=state.db.call(move|db|Ok(db.query_row("SELECT channel_id,uploads FROM youtube_subscriptions WHERE user_id=?1 AND active=1 AND uploads IS NOT NULL AND channel_id>?2 ORDER BY channel_id LIMIT 1",params![owner,after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await?;
                let Some((channel, playlist)) = next else {
                    turn.cursor.phase = "refresh".into();
                    turn.cursor.after.clear();
                    return save(state, turn, false, |_| Ok(())).await;
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
            save(state, turn, false, |_| Ok(())).await
        }
        "videos" | "refresh" => {
            let refresh = turn.cursor.phase == "refresh";
            if refresh {
                let owner = user.clone();
                let after = turn.cursor.after.clone();
                turn.cursor.ids=state.db.call(move|db|Ok(db.prepare("SELECT v.video_id FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id) WHERE v.user_id=?1 AND v.video_id>?2 AND (v.broadcast IN ('live','upcoming') OR s.watchlist=1 OR s.pinned=1) ORDER BY v.video_id LIMIT 50")?.query_map(params![owner,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
                if turn.cursor.ids.is_empty() {
                    turn.cursor.phase = "shorts".into();
                    turn.cursor.pages = 0;
                    return save(state, turn, false, |_| Ok(())).await;
                }
                turn.cursor.after = turn.cursor.ids.last().unwrap().clone();
            }
            let ids = turn.cursor.ids.clone();
            if ids.is_empty() {
                turn.cursor.phase = "uploads".into();
                return save(state, turn, false, |_| Ok(())).await;
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
            save(state,turn,false,move|tx|{
                for video in &ids {tx.execute("UPDATE youtube_videos SET available=0,metadata_at=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;}
                for video in rows {upsert_video(tx,&user,&video,&ids)?;}
                tx.execute("DELETE FROM youtube_videos WHERE user_id=?1 AND published_at<?2 AND metadata_at<?3 AND NOT EXISTS(SELECT 1 FROM youtube_state s WHERE s.user_id=youtube_videos.user_id AND s.video_id=youtube_videos.video_id)",params![user,now()-90*86400,now()-30*86400])?;
                Ok(())
            }).await
        }
        "shorts" => save(state, turn, true, |_| Ok(())).await,
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
pub(super) fn upsert_video(
    tx: &rusqlite::Transaction<'_>,
    user: &str,
    video: &Value,
    requested: &[String],
) -> anyhow::Result<()> {
    let Some(video_id) = video["id"]
        .as_str()
        .filter(|v| identifier(v, 11) && requested.iter().any(|r| r == v))
    else {
        return Ok(());
    };
    let Some(channel) = channel_id(&video["snippet"]["channelId"]) else {
        return Ok(());
    };
    let snippet = &video["snippet"];
    let broadcast = match snippet["liveBroadcastContent"].as_str() {
        Some("live") => "live",
        Some("upcoming") => "upcoming",
        _ if video["liveStreamingDetails"]["actualEndTime"].is_string() => "replay",
        _ => "none",
    };
    let duration = video["contentDetails"]["duration"]
        .as_str()
        .and_then(duration);
    let privacy = match video["status"]["privacyStatus"].as_str() {
        Some("public") => "public",
        Some("unlisted") => "unlisted",
        Some("private") => "private",
        _ => "unknown",
    };
    tx.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,channel_title,published_at,duration,broadcast,privacy,metadata_at,is_short) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(user_id,video_id) DO UPDATE SET channel_id=excluded.channel_id,title=excluded.title,channel_title=excluded.channel_title,published_at=excluded.published_at,duration=excluded.duration,broadcast=excluded.broadcast,privacy=excluded.privacy,metadata_at=excluded.metadata_at,available=1,is_short=COALESCE(excluded.is_short,youtube_videos.is_short)",params![user,video_id,channel,text(&snippet["title"],500),text(&snippet["channelTitle"],300),timestamp(&snippet["publishedAt"]),duration,broadcast,privacy,now(),if broadcast!="none"||duration.is_some_and(|d|d>180){Some(false)}else{None}])?;
    tx.execute("UPDATE youtube_videos SET scheduled_start=?1,actual_start=?2,actual_end=?3 WHERE user_id=?4 AND video_id=?5",params![video["liveStreamingDetails"]["scheduledStartTime"].as_str(),video["liveStreamingDetails"]["actualStartTime"].as_str(),video["liveStreamingDetails"]["actualEndTime"].as_str(),user,video_id])?;
    Ok(())
}
