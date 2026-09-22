//! Database operations for online.sync.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn request(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<bool> {
    db.write("online.sync.request", move|db|Ok(db.execute("UPDATE youtube_sync SET next_run=?1 WHERE user_id=?2 AND (last_complete IS NULL OR last_complete<?1-300) AND failures=0 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=youtube_sync.user_id AND a.provider='youtube' AND a.status='connected' AND a.generation=youtube_sync.generation)",params![now(),p.user.id])?==1)).await
}

pub(super) async fn claim_classification(
    db: &Database,
) -> anyhow::Result<Option<(String, String, String)>> {
    db.write("online.sync.claim_classification", |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT v.user_id,v.video_id,a.generation FROM youtube_videos v JOIN online_accounts a ON a.user_id=v.user_id AND a.provider='youtube' AND a.status='connected' WHERE v.available=1 AND v.privacy='public' AND v.broadcast='none' AND v.is_short IS NULL AND v.duration BETWEEN 1 AND 180 AND v.short_checked<?1 ORDER BY v.short_checked,v.published_at DESC LIMIT 1",[now()-86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((user,video,_))=&row {tx.execute("UPDATE youtube_videos SET short_checked=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;}
        tx.commit()?;Ok(row)
    }).await
}

pub(super) async fn save_classification(
    result: Option<bool>,
    user: String,
    video: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<usize> {
    db.write("online.sync.save_classification", move|db|Ok(db.execute("UPDATE youtube_videos SET is_short=?1,short_checked=?2 WHERE user_id=?3 AND video_id=?4 AND EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?3 AND provider='youtube' AND status='connected' AND generation=?5)",params![result,if result.is_some(){now()}else{now()-86400+300},user,video,generation])?)).await
}

pub(super) async fn claim(db: &Database) -> anyhow::Result<Option<Turn>> {
    db.write("online.sync.claim", |db|{
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

pub(super) async fn tick(
    user: String,
    generation: String,
    message: String,
    delay: i64,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.sync.tick", move|db|{db.execute("UPDATE youtube_sync SET next_run=?1,failures=failures+1,error=?2 WHERE user_id=?3 AND generation=?4",params![now()+delay,message,user,generation])?;Ok(())}).await
}

pub(super) async fn save(
    cursor: String,
    db: &Database,
    turn: Turn,
    done: bool,
    page: Page,
) -> anyhow::Result<bool> {
    db.write("online.sync.save", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let active=tx.query_row("SELECT EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND status='connected' AND generation=?2)",params![turn.user,turn.generation],|r|r.get::<_,bool>(0))?;
        if !active {return Ok(false);}
        page.apply(&tx, &turn.user)?;
        tx.execute("UPDATE youtube_sync SET cursor=?1,next_run=?2,last_complete=CASE WHEN ?3 THEN ?4 ELSE last_complete END,failures=0,error=NULL WHERE user_id=?5 AND generation=?6",params![cursor,if done{now()+1800}else{now()},done,now(),turn.user,turn.generation])?;
        crate::storage::record_event(&tx, Some(&turn.user), "youtube.changed", &json!({"complete":done}))?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn pending_metadata(owner: String, db: &Database) -> anyhow::Result<Vec<String>> {
    db.read("online.sync.pending_metadata", move|db|Ok(db.prepare("SELECT v.video_id FROM youtube_videos v LEFT JOIN youtube_video_state s USING(user_id,video_id) WHERE v.user_id=?1 AND v.metadata_at=0 ORDER BY s.added_at,v.video_id LIMIT 50")?.query_map([owner],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn channels_to_refresh(
    owner: String,
    after: String,
    db: &Database,
) -> anyhow::Result<Vec<String>> {
    db.read("online.sync.channels_to_refresh", move|db|Ok(db.prepare("SELECT channel_id FROM youtube_subscriptions WHERE user_id=?1 AND active=1 AND channel_id>?2 ORDER BY channel_id LIMIT 50")?.query_map(params![owner,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn step_read_online_accounts(
    owner: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("online.sync.step_read_online_accounts", move |db| {
        Ok(db.query_row("SELECT external_id FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND external_id<>'' AND profile_checked_at<?2", params![owner, now()-86400], |row| row.get::<_, String>(0)).optional()?)
    }).await
}

pub(super) async fn next_upload_playlist(
    owner: String,
    after: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("online.sync.next_upload_playlist", move|db|Ok(db.query_row("SELECT channel_id,uploads FROM youtube_subscriptions WHERE user_id=?1 AND active=1 AND uploads IS NOT NULL AND channel_id>?2 ORDER BY channel_id LIMIT 1",params![owner,after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await
}

pub(super) async fn videos_to_refresh(
    owner: String,
    after: String,
    db: &Database,
) -> anyhow::Result<Vec<String>> {
    db.read("online.sync.videos_to_refresh", move|db|Ok(db.prepare("SELECT v.video_id FROM youtube_videos v LEFT JOIN youtube_video_state s USING(user_id,video_id) WHERE v.user_id=?1 AND v.video_id>?2 AND (v.broadcast IN ('live','upcoming') OR s.watchlist=1 OR s.pinned=1) ORDER BY v.video_id LIMIT 50")?.query_map(params![owner,after],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(in super::super) fn upsert_video(
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

pub(super) enum Page {
    CursorOnly,
    PendingVideos {
        pending: Vec<String>,
        rows: Vec<Value>,
    },
    Subscriptions {
        rows: Vec<(String, String, Option<String>)>,
        snapshot: String,
        complete: bool,
    },
    Channels {
        refresh_profile: bool,
        name: Option<String>,
        has_profile: bool,
        avatar: Option<String>,
        channels: Vec<String>,
        rows: Vec<(String, String)>,
    },
    Videos {
        ids: Vec<String>,
        rows: Vec<Value>,
    },
}
impl Page {
    fn apply(self, tx: &rusqlite::Transaction<'_>, user: &str) -> anyhow::Result<()> {
        match self {
            Self::CursorOnly => Ok(()),
            Self::PendingVideos { pending, rows } => {
                for video in &pending {
                    tx.execute("UPDATE youtube_videos SET available=0,metadata_at=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;
                }
                for video in rows {
                    upsert_video(tx, user, &video, &pending)?;
                }
                Ok(())
            }
            Self::Subscriptions {
                rows,
                snapshot,
                complete,
            } => {
                for (channel, title, thumbnail) in rows {
                    tx.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,thumbnail_url) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(user_id,channel_id) DO UPDATE SET title=excluded.title,snapshot=excluded.snapshot,thumbnail_url=excluded.thumbnail_url",params![user,channel,title,snapshot,thumbnail])?;
                }
                if complete {
                    tx.execute(
                        "DELETE FROM youtube_subscriptions WHERE user_id=?1 AND snapshot<>?2",
                        params![user, snapshot],
                    )?;
                    tx.execute("UPDATE youtube_subscriptions SET active=1 WHERE user_id=?1 AND snapshot=?2",params![user,snapshot])?;
                }
                Ok(())
            }
            Self::Channels {
                refresh_profile,
                name,
                has_profile,
                avatar,
                channels,
                rows,
            } => {
                if refresh_profile {
                    tx.execute("UPDATE online_accounts SET profile_checked_at=?1,display_name=COALESCE(?2,display_name),avatar_url=CASE WHEN ?3 THEN ?4 ELSE avatar_url END WHERE user_id=?5 AND provider='youtube'",params![now(),name,has_profile,avatar,user])?;
                }
                for channel in channels {
                    tx.execute("UPDATE youtube_subscriptions SET uploads=NULL WHERE user_id=?1 AND channel_id=?2",params![user,channel])?;
                }
                for (channel, playlist) in rows {
                    tx.execute("UPDATE youtube_subscriptions SET uploads=?1 WHERE user_id=?2 AND channel_id=?3",params![playlist,user,channel])?;
                }
                if refresh_profile {
                    crate::storage::record_event(
                        tx,
                        Some(user),
                        "online.account.changed",
                        &json!({"provider":"youtube"}),
                    )?;
                }
                Ok(())
            }
            Self::Videos { ids, rows } => {
                for video in &ids {
                    tx.execute("UPDATE youtube_videos SET available=0,metadata_at=?1 WHERE user_id=?2 AND video_id=?3",params![now(),user,video])?;
                }
                for video in rows {
                    upsert_video(tx, user, &video, &ids)?;
                }
                tx.execute("DELETE FROM youtube_videos WHERE user_id=?1 AND published_at<?2 AND metadata_at<?3 AND NOT EXISTS(SELECT 1 FROM youtube_state s WHERE s.user_id=youtube_videos.user_id AND s.video_id=youtube_videos.video_id) AND NOT EXISTS(SELECT 1 FROM youtube_watchlist_items i WHERE i.user_id=youtube_videos.user_id AND i.video_id=youtube_videos.video_id)",params![user,now()-90*86400,now()-30*86400])?;
                Ok(())
            }
        }
    }
}
