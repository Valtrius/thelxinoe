use super::VideoUsage;
use anyhow::{Context, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use thelxinoe_core::{id, now};
use thelxinoe_database::Database;

const MAX_DOWNLOAD_BYTES: i64 = 2 * 1024 * 1024 * 1024;

struct Policy {
    watched: bool,
    delay: i64,
    limit: i64,
    revision: i64,
}
struct Download {
    video: String,
    generation: String,
    path: Option<String>,
    size: i64,
    modified: Option<String>,
}
struct Eligibility {
    stamp: String,
    users: Vec<String>,
    delay: i64,
}
struct Candidate {
    id: String,
    video: String,
    generation: String,
    stamp: String,
    reason: String,
    state: String,
    due: i64,
}

fn policy(db: &Connection) -> anyhow::Result<Policy> {
    Ok(db.query_row("SELECT enabled,grace_seconds,storage_limit_bytes,updated_at FROM retention_policies WHERE domain='videos'",[],|r|Ok(Policy{watched:r.get(0)?,delay:r.get(1)?,limit:r.get(2)?,revision:r.get(3)?}))?)
}

fn pinned(db: &Connection, video: &str) -> anyhow::Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM youtube_state WHERE video_id=?1 AND pinned=1)",
        [video],
        |r| r.get(0),
    )?)
}

fn playing(db: &Connection, video: &str) -> anyhow::Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE youtube_video_id=?1 AND state IN ('ready','playing','paused') AND updated_at>?2)",params![video,now()-120],|r|r.get(0))?)
}

fn download(db: &Connection, video: &str) -> anyhow::Result<Option<Download>> {
    Ok(db.query_row("SELECT video_id,generation,path,COALESCE(size,0),modified FROM youtube_downloads WHERE video_id=?1 AND state='ready'",[video],|r|Ok(Download{video:r.get(0)?,generation:r.get(1)?,path:r.get(2)?,size:r.get(3)?,modified:r.get(4)?})).optional()?)
}

fn usage(db: &Connection) -> anyhow::Result<VideoUsage> {
    let (unpinned_bytes,pinned_bytes)=db.query_row("SELECT COALESCE(SUM(CASE WHEN EXISTS(SELECT 1 FROM youtube_state s WHERE s.video_id=d.video_id AND s.pinned=1) THEN 0 ELSE d.size END),0),COALESCE(SUM(CASE WHEN EXISTS(SELECT 1 FROM youtube_state s WHERE s.video_id=d.video_id AND s.pinned=1) THEN d.size ELSE 0 END),0) FROM youtube_downloads d WHERE d.state='ready'",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let waiting = db.query_row(
        "SELECT COUNT(*) FROM youtube_downloads WHERE state='waiting_for_space'",
        [],
        |r| r.get(0),
    )?;
    Ok(VideoUsage {
        unpinned_bytes,
        pinned_bytes,
        waiting,
    })
}

fn eligibility(db: &Connection, d: &Download) -> anyhow::Result<Option<Eligibility>> {
    let p = policy(db)?;
    if !p.watched || pinned(db, &d.video)? || playing(db, &d.video)? {
        return Ok(None);
    }
    let audience=db.prepare("WITH interested(user_id,added_at) AS (SELECT user_id,MAX(added_at) FROM youtube_watchlist_items WHERE video_id=?1 GROUP BY user_id UNION ALL SELECT user_id,requested_at FROM youtube_download_requests WHERE video_id=?1) SELECT u.id,u.username,COALESCE(s.watched,0),COALESCE(s.watched_revision,0),MAX(i.added_at) FROM interested i JOIN users u ON u.id=i.user_id LEFT JOIN youtube_state s ON s.user_id=u.id AND s.video_id=?1 GROUP BY u.id ORDER BY u.id")?.query_map([&d.video],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if audience.iter().any(|a| !a.2) {
        return Ok(None);
    }
    let stamp = Sha256::digest(serde_json::to_vec(&(
        &d.generation,
        d.size,
        &d.modified,
        p.revision,
        &audience,
    ))?)
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect();
    Ok(Some(Eligibility {
        stamp,
        users: audience.into_iter().map(|a| a.1).collect(),
        delay: p.delay,
    }))
}

fn candidate(db: &Connection, key: &str) -> anyhow::Result<Option<Candidate>> {
    Ok(db.query_row("SELECT id,video_id,generation,stamp,reason,state,due_at FROM video_retention_candidates WHERE id=?1",[key],|r|Ok(Candidate{id:r.get(0)?,video:r.get(1)?,generation:r.get(2)?,stamp:r.get(3)?,reason:r.get(4)?,state:r.get(5)?,due:r.get(6)?})).optional()?)
}

fn insert_candidate(
    db: &Connection,
    d: &Download,
    reason: &str,
    stamp: &str,
    delay: i64,
    users: &[String],
) -> anyhow::Result<String> {
    let key = id();
    let title = db
        .query_row(
            "SELECT title FROM youtube_videos WHERE video_id=?1 ORDER BY user_id LIMIT 1",
            [&d.video],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| d.video.clone());
    db.execute("INSERT INTO video_retention_candidates(id,video_id,generation,stamp,title,reason,state,eligible_at,due_at,watched_users) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?8,?9)",params![key,d.video,d.generation,stamp,title,reason,now(),now()+delay,json!(users).to_string()])?;
    Ok(key)
}

fn refresh(db: &mut Connection) -> anyhow::Result<usize> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut changed=tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='Download changed' WHERE state='pending' AND NOT EXISTS(SELECT 1 FROM youtube_downloads d WHERE d.video_id=video_retention_candidates.video_id AND d.generation=video_retention_candidates.generation AND d.state='ready')",[])?;
    changed+=tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='Storage evaluation restarted or file requires review' WHERE state='pending' AND (reason='storage_limit' OR EXISTS(SELECT 1 FROM video_retention_candidates attempted WHERE attempted.video_id=video_retention_candidates.video_id AND attempted.generation=video_retention_candidates.generation AND attempted.state IN ('blocked','executing')))",[])?;
    let ids = tx
        .prepare("SELECT video_id FROM youtube_downloads WHERE state='ready'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for video in ids {
        let Some(d) = download(&tx, &video)? else {
            continue;
        };
        if let Some(e) = eligibility(&tx, &d)? {
            changed+=tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='Watched eligibility changed' WHERE video_id=?1 AND reason='watched' AND state='pending' AND stamp<>?2",params![video,e.stamp])?;
            let exists=tx.query_row("SELECT EXISTS(SELECT 1 FROM video_retention_candidates WHERE video_id=?1 AND generation=?2 AND (state IN ('executing','blocked','complete') OR (state='pending' AND stamp=?3)))",params![video,d.generation,e.stamp],|r|r.get::<_,bool>(0))?;
            if !exists {
                insert_candidate(&tx, &d, "watched", &e.stamp, e.delay, &e.users)?;
                changed += 1;
            }
        } else {
            changed+=tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='Watched eligibility changed' WHERE video_id=?1 AND reason='watched' AND state='pending'",[video])?;
        }
    }
    tx.commit()?;
    Ok(changed)
}

fn delete_file(root: &Path, d: &Download) -> anyhow::Result<()> {
    ensure!(
        d.video.len() == 11
            && d.video
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
        "Invalid downloaded video identity"
    );
    ensure!(
        uuid::Uuid::parse_str(&d.generation).is_ok(),
        "Invalid download generation"
    );
    let root = root
        .canonicalize()
        .context("Download storage is unavailable")?;
    let directory = root.join(&d.video).join(&d.generation);
    ensure!(
        directory.canonicalize().ok().as_ref() == Some(&directory),
        "Download directory identity changed"
    );
    let path = directory.join("media.mp4");
    ensure!(d.path.as_ref().is_some_and(|recorded|Path::new(recorded).canonicalize().ok().as_ref()==Some(&path)),"Recorded download path changed");
    let metadata = std::fs::symlink_metadata(&path).context("Downloaded file is unavailable")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Downloaded file identity changed"
    );
    ensure!(
        metadata.len() as i64 == d.size
            && d.modified.as_ref()
                == Some(
                    &metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)?
                        .as_nanos()
                        .to_string()
                ),
        "Downloaded file generation changed"
    );
    std::fs::remove_file(&path).context("Could not delete downloaded file")?;
    // Only recorded media is ours. Unexpected sidecars keep the directory alive.
    let _ = std::fs::remove_dir(&directory);
    let _ = std::fs::remove_dir(root.join(&d.video));
    Ok(())
}

fn execute(
    db: &mut Connection,
    root: &Path,
    c: Candidate,
    automatic: bool,
    reserve: i64,
    actor: Option<&str>,
) -> anyhow::Result<bool> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure!(c.state == "pending", "This deletion is no longer scheduled");
    let d = download(&tx, &c.video)?.context("Download changed before deletion")?;
    ensure!(d.generation == c.generation, "Download generation changed");
    ensure!(
        !pinned(&tx, &c.video)? && !playing(&tx, &c.video)?,
        "The download is kept or playing"
    );
    if c.reason == "watched" {
        ensure!(
            eligibility(&tx, &d)?.is_some_and(|e| e.stamp == c.stamp),
            "Watched eligibility changed"
        );
        ensure!(
            !automatic || now() >= c.due,
            "The deletion countdown has not elapsed"
        );
    } else if automatic {
        ensure!(
            usage(&tx)?.unpinned_bytes.saturating_add(reserve) > policy(&tx)?.limit,
            "Storage is already below its limit"
        );
    }
    tx.execute(
        "UPDATE video_retention_candidates SET state='executing' WHERE id=?1",
        [&c.id],
    )?;
    tx.commit()?;
    // The storage writer remains held across the durable intent, file deletion,
    // and result. Pins, policy changes, and new playback cannot interleave.
    let result = delete_file(root, &d);
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Err(error) = result {
        tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='File requires review' WHERE video_id=?1 AND state='pending'",[&c.video])?;
        tx.execute(
            "UPDATE video_retention_candidates SET state='blocked',error=?2 WHERE id=?1",
            params![c.id, error.to_string()],
        )?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'auto_delete.blocked',?2,?3)",params![actor,c.video,now()])?;
        tx.commit()?;
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM playback_sessions WHERE youtube_video_id=?1",
        [&c.video],
    )?;
    tx.execute(
        "DELETE FROM youtube_downloads WHERE video_id=?1 AND generation=?2",
        params![c.video, c.generation],
    )?;
    tx.execute(
        "DELETE FROM youtube_download_requests WHERE video_id=?1",
        [&c.video],
    )?;
    tx.execute(
        "INSERT INTO youtube_download_suppressed VALUES (?1) ON CONFLICT DO NOTHING",
        [&c.video],
    )?;
    tx.execute("UPDATE video_retention_candidates SET state='cancelled',error='Download removed' WHERE video_id=?1 AND state='pending'",[&c.video])?;
    tx.execute(
        "UPDATE video_retention_candidates SET state='complete',error=NULL WHERE id=?1",
        [&c.id],
    )?;
    tx.execute(
        "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
        params![actor, format!("auto_delete.{}", c.reason), c.video, now()],
    )?;
    tx.commit()?;
    Ok(true)
}

fn evict(db: &mut Connection, root: &Path, reserve: i64) -> anyhow::Result<usize> {
    if usage(db)?.unpinned_bytes.saturating_add(reserve) <= policy(db)?.limit {
        return Ok(0);
    }
    let ids=db.prepare("SELECT d.video_id FROM youtube_downloads d WHERE d.state='ready' AND NOT EXISTS(SELECT 1 FROM youtube_state s WHERE s.video_id=d.video_id AND s.pinned=1) AND NOT EXISTS(SELECT 1 FROM video_retention_candidates c WHERE c.video_id=d.video_id AND c.generation=d.generation AND c.state IN ('blocked','executing')) ORDER BY d.completed_at,d.video_id")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut changed = 0;
    for video in ids {
        if usage(db)?.unpinned_bytes.saturating_add(reserve) <= policy(db)?.limit {
            break;
        }
        if playing(db, &video)? {
            continue;
        }
        let Some(d) = download(db, &video)? else {
            continue;
        };
        let key = insert_candidate(db, &d, "storage_limit", &d.generation, 0, &[])?;
        let c = candidate(db, &key)?.context("Scheduled deletion disappeared")?;
        execute(db, root, c, true, reserve, None)?;
        changed += 1;
    }
    Ok(changed)
}

pub(super) async fn evaluate(db: &Database, root: PathBuf) -> anyhow::Result<usize> {
    db.write("auto_delete.evaluate",move|db| {
        // A crashed destructive attempt requires review, not an automatic retry.
        let mut changed=db.execute("UPDATE video_retention_candidates SET state='blocked',error='Deletion interrupted; inspect the file before retrying' WHERE state='executing'",[])?;
        changed+=refresh(db)?;
        let keys=db.prepare("SELECT id FROM video_retention_candidates WHERE state='pending' AND reason='watched' AND due_at<=?1 ORDER BY due_at")?.query_map([now()],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for key in keys {
            if let Some(c)=candidate(db,&key)? { execute(db,&root,c,true,0,None)?;changed+=1; }
        }
        changed+=evict(db,&root,0)?;
        Ok(changed)
    }).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<(Vec<Value>, VideoUsage)> {
    db.read("auto_delete.list",|db| {
        let items=db.prepare("SELECT id,video_id,title,state,reason,eligible_at,due_at,error,watched_users FROM video_retention_candidates ORDER BY state='pending' DESC,eligible_at DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"video_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"reason":r.get::<_,String>(4)?,"eligible_at":r.get::<_,i64>(5)?,"due_at":r.get::<_,i64>(6)?,"error":r.get::<_,Option<String>>(7)?,"watched_users":serde_json::from_str::<Value>(&r.get::<_,String>(8)?).unwrap_or_default(),"domain":"videos"})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((items,usage(db)?))
    }).await
}

pub(super) async fn action(
    db: &Database,
    root: PathBuf,
    key: String,
    action: String,
    actor: String,
) -> anyhow::Result<bool> {
    db.write("auto_delete.action",move|db| {
        let Some(c)=candidate(db,&key)? else {return Ok(false)};
        ensure!(c.state=="pending","This deletion is no longer scheduled");
        if action=="keep" {
            let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            ensure!(download(&tx,&c.video)?.is_some_and(|d|d.generation==c.generation),"Download generation changed");
            tx.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) SELECT ?1,video_id,title,'public' FROM youtube_videos WHERE video_id=?2 LIMIT 1 ON CONFLICT(user_id,video_id) DO NOTHING",params![actor,c.video])?;
            tx.execute("INSERT INTO youtube_state(user_id,video_id,pinned,updated_at) VALUES (?1,?2,1,?3) ON CONFLICT(user_id,video_id) DO UPDATE SET pinned=1,updated_at=excluded.updated_at",params![actor,c.video,now()])?;
            tx.execute("UPDATE video_retention_candidates SET state='kept',error=NULL WHERE video_id=?1 AND state='pending'",[&c.video])?;
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'auto_delete.keep',?2,?3)",params![actor,c.video,now()])?;
            tx.commit()?;
        } else {
            ensure!(action=="delete","Unsupported deletion action");
            ensure!(execute(db,&root,c,false,0,Some(&actor))?,"The file changed or could not be deleted; review its history");
        }
        Ok(true)
    }).await
}

pub(super) async fn admit(
    db: &Database,
    root: PathBuf,
    video: String,
    generation: String,
) -> anyhow::Result<bool> {
    db.write("auto_delete.admit",move|db| {
        ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_downloads WHERE video_id=?1 AND generation=?2 AND state='downloading')",params![video,generation],|r|r.get::<_,bool>(0))?,"Download was cancelled");
        if pinned(db,&video)? {return Ok(true);}
        let limit=policy(db)?.limit;
        let reserve=MAX_DOWNLOAD_BYTES.min(limit);
        evict(db,&root,reserve)?;
        Ok(usage(db)?.unpinned_bytes.saturating_add(reserve)<=limit)
    }).await
}

pub(super) async fn budget(db: &Database, video: String) -> anyhow::Result<i64> {
    db.read("auto_delete.budget", move |db| available_budget(db, &video))
        .await
}

pub(crate) fn available_budget(db: &Connection, video: &str) -> anyhow::Result<i64> {
    if pinned(db, video)? {
        return Ok(MAX_DOWNLOAD_BYTES);
    }
    Ok((policy(db)?.limit - usage(db)?.unpinned_bytes).clamp(0, MAX_DOWNLOAD_BYTES))
}

pub(crate) fn download_limit(db: &Connection, video: &str) -> anyhow::Result<i64> {
    Ok(if pinned(db, video)? {
        MAX_DOWNLOAD_BYTES
    } else {
        policy(db)?.limit.min(MAX_DOWNLOAD_BYTES)
    })
}

pub(crate) fn remove_ready(db: &Connection, root: &Path, video: &str) -> anyhow::Result<()> {
    if let Some(d) = download(db, video)? {
        delete_file(root, &d)?;
    }
    Ok(())
}
