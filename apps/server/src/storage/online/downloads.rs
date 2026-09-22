//! Database operations for online.downloads.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn authorize(user: String, video: String, db: &Database) -> anyhow::Result<bool> {
    db.read("online.downloads.authorize", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",
            params![user, video],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn enabled(db: &Database) -> anyhow::Result<bool> {
    db.read("online.downloads.enabled", |db| {
        Ok(db
            .query_row(
                "SELECT value='true' FROM settings WHERE key='youtube_downloads'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    })
    .await
}

pub(super) async fn remove(
    db: &Database,
    video: String,
    root: PathBuf,
    user: String,
) -> anyhow::Result<bool> {
    db.write("online.downloads.remove", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let protected=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE video_id=?1 AND user_id<>?2 AND (watchlist=1 OR pinned=1)) OR EXISTS(SELECT 1 FROM playback_sessions WHERE youtube_video_id=?1 AND state IN ('ready','playing','paused') AND updated_at>?3)",params![video,user,now()-120],|r|r.get::<_,bool>(0))?;
        if protected {return Ok(false);}
        let row=tx.query_row("SELECT generation,state FROM youtube_downloads WHERE video_id=?1",[&video],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        if let Some((generation,status))=row {
            ensure!(uuid::Uuid::parse_str(&generation).is_ok(),"Invalid download generation");
            // The worker owns a running process and removes its cancelled generation.
            if status!="downloading" && root.exists() {
                let root=root.canonicalize()?;
                let path=root.join(&video).join(&generation);
                if path.exists(){ensure!(path.canonicalize()?==path,"Download path changed");std::fs::remove_dir_all(path)?;}
            }
            tx.execute("DELETE FROM youtube_downloads WHERE video_id=?1",[&video])?;
        }
        tx.execute("INSERT INTO youtube_download_suppressed VALUES(?1) ON CONFLICT DO NOTHING",[video])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn maintain_watchlists_write_youtube_watchlist_items(
    db: &Database,
) -> anyhow::Result<Vec<String>> {
    db.write("online.downloads.maintain_watchlists_write_youtube_watchlist_items", |db|{
        let users=db.prepare("SELECT DISTINCT i.user_id FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id JOIN youtube_state s ON s.user_id=i.user_id AND s.video_id=i.video_id WHERE w.auto_remove_watched=1 AND s.watched=1 AND s.updated_at<?1 AND i.added_at<?1")?.query_map([now()-5],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        db.execute("DELETE FROM youtube_watchlist_items WHERE added_at<?1 AND EXISTS(SELECT 1 FROM youtube_watchlists w JOIN youtube_state s ON s.user_id=w.user_id WHERE w.id=youtube_watchlist_items.watchlist_id AND w.auto_remove_watched=1 AND s.video_id=youtube_watchlist_items.video_id AND s.watched=1 AND s.updated_at<?1)",[now()-5])?;
        Ok(users)
    }).await
}

pub(super) async fn maintain_watchlists_read_youtube_watchlist_items(
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("online.downloads.maintain_watchlists_read_youtube_watchlist_items", |db|Ok(db.query_row("SELECT i.video_id FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id JOIN youtube_videos v ON v.user_id=i.user_id AND v.video_id=i.video_id WHERE w.auto_download=1 AND v.metadata_at>0 AND v.available=1 AND v.privacy='public' AND v.broadcast IN ('none','replay') AND NOT EXISTS(SELECT 1 FROM youtube_downloads d WHERE d.video_id=i.video_id) AND NOT EXISTS(SELECT 1 FROM youtube_download_suppressed b WHERE b.video_id=i.video_id) ORDER BY i.added_at LIMIT 1",[],|r|r.get::<_,String>(0)).optional()?)).await
}

pub(super) async fn maintain_watchlists_write_youtube_media(
    bundle: tools::Bundle,
    video: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.downloads.maintain_watchlists_write_youtube_media", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO youtube_media(video_id) VALUES(?1) ON CONFLICT DO NOTHING",[&video])?;
        tx.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) SELECT ?1,?2,'queued',?3,?4,?4 WHERE EXISTS(SELECT 1 FROM youtube_watchlist_items i JOIN youtube_watchlists w ON w.id=i.watchlist_id WHERE i.video_id=?1 AND w.auto_download=1) AND NOT EXISTS(SELECT 1 FROM youtube_download_suppressed WHERE video_id=?1) ON CONFLICT DO NOTHING",params![video,thelxinoe_core::id(),serde_json::to_string(&bundle)?,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn configure(db: &Database, enabled: bool, p: Principal) -> anyhow::Result<()> {
    db.write("online.downloads.configure", move |db| {let tx=db.transaction()?; tx.execute("INSERT INTO settings VALUES ('youtube_downloads',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[enabled.to_string()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.downloads.configure',?2,?3)",params![p.user.id,enabled.to_string(),now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn status(db: &Database, video: String) -> anyhow::Result<Option<Value>> {
    db.read("online.downloads.status", move |db| Ok(db.query_row("SELECT state,size,error,downloaded_bytes,total_bytes,eta_seconds,media_kind FROM youtube_downloads WHERE video_id=?1",[video],|r|Ok(json!({"state":r.get::<_,String>(0)?,"size":r.get::<_,Option<i64>>(1)?,"error":r.get::<_,Option<String>>(2)?,"downloaded_bytes":r.get::<_,i64>(3)?,"total_bytes":r.get::<_,Option<i64>>(4)?,"eta_seconds":r.get::<_,Option<i64>>(5)?,"media_kind":r.get::<_,Option<String>>(6)?}))).optional()?)).await
}

pub(super) async fn request_for(
    bundle: tools::Bundle,
    p: Principal,
    db: &Database,
    video: String,
) -> anyhow::Result<bool> {
    db.write("online.downloads.request_for", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let interest=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE user_id=?1 AND video_id=?2 AND (watchlist=1 OR pinned=1))",params![p.user.id,video],|r|r.get::<_,bool>(0))?;
        if !interest { return Ok(false); }
        tx.execute("INSERT INTO youtube_media(video_id) VALUES (?1) ON CONFLICT DO NOTHING",[&video])?;
        tx.execute("DELETE FROM youtube_download_suppressed WHERE video_id=?1",[&video])?;
        // A retained interest is required before acquiring shared physical media.
        tx.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES (?1,?2,'queued',?3,?4,?4) ON CONFLICT(video_id) DO UPDATE SET generation=excluded.generation,state='queued',tools=excluded.tools,error=NULL,downloaded_bytes=0,total_bytes=NULL,eta_seconds=NULL,media_kind=NULL,updated_at=excluded.updated_at WHERE youtube_downloads.state IN ('failed','unavailable','extractor_authentication_required')",params![video,thelxinoe_core::id(),serde_json::to_string(&bundle)?,now()])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn source(
    video: String,
    root: PathBuf,
    db: &Database,
) -> anyhow::Result<Option<Source>> {
    db.read("online.downloads.source", move |db| Ok(db.query_row("SELECT generation,path,size,modified,probe FROM youtube_downloads WHERE video_id=?1 AND state='ready'",[&video],|r|Ok(Source {id:video.clone(),media_id:format!("youtube:{video}"),generation:r.get(0)?,edition:"public".into(),path:PathBuf::from(r.get::<_,String>(1)?),root,size:r.get::<_,i64>(2)? as u64,modified:r.get(3)?,probe:serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or_default()})).optional()?)).await
}

pub(super) async fn recover_downloads(db: &Database) -> anyhow::Result<()> {
    db.write("online.downloads.recover_downloads", |db| {
        db.execute(
            "UPDATE youtube_downloads SET state='queued' WHERE state='downloading'",
            [],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn claim_download(
    db: &Database,
) -> anyhow::Result<Option<(String, String, String)>> {
    db.write("online.downloads.claim_download", |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT video_id,generation,tools FROM youtube_downloads d WHERE state='queued' AND EXISTS(SELECT 1 FROM youtube_video_state s WHERE s.video_id=d.video_id AND (s.watchlist=1 OR s.pinned=1)) ORDER BY updated_at LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        if let Some((video,_,_))=&row {tx.execute("UPDATE youtube_downloads SET state='downloading',updated_at=?2 WHERE video_id=?1",params![video,now()])?;}
        tx.commit()?;Ok(row)
    }).await
}

pub(super) async fn finish_download(
    status: &'static str,
    video: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.downloads.finish_download", move |db| {db.execute("UPDATE youtube_downloads SET state=?3,error=CASE WHEN ?3='ready' THEN NULL ELSE ?3 END,updated_at=?4 WHERE video_id=?1 AND generation=?2",params![video,generation,status,now()])?;Ok(())}).await
}

pub(super) async fn cleanup(root: PathBuf, db: &Database) -> anyhow::Result<()> {
    db.write("online.downloads.cleanup", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let protection="EXISTS(SELECT 1 FROM youtube_video_state s WHERE s.video_id=youtube_downloads.video_id AND (s.watchlist=1 OR s.pinned=1)) OR EXISTS(SELECT 1 FROM playback_sessions p WHERE p.youtube_video_id=youtube_downloads.video_id AND p.state IN ('ready','playing','paused') AND p.updated_at>".to_owned()+&(now()-120).to_string()+")";
        tx.execute(&format!("UPDATE youtube_downloads SET unprotected_at=NULL WHERE {protection}"),[])?;
        tx.execute(&format!("UPDATE youtube_downloads SET unprotected_at=?1 WHERE unprotected_at IS NULL AND NOT ({protection}) AND state!='downloading'"),[now()])?;
        let candidate=tx.query_row(&format!("SELECT video_id,generation,path,size,modified FROM youtube_downloads WHERE unprotected_at<?1 AND state!='downloading' AND NOT ({protection}) ORDER BY unprotected_at LIMIT 1"),[now()-86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,Option<String>>(4)?))).optional()?;
        if let Some((video,generation,file,size,modified))=candidate {
            ensure!(sync::identifier(&video,11)&&uuid::Uuid::parse_str(&generation).is_ok(),"Invalid download identity");
            let path=root.join(&video).join(&generation);
            if path.exists() {
                ensure!(path.canonicalize()?==path,"Download cleanup path changed");
                if let Some(file)=file {
                    let file=PathBuf::from(file);
                    let expected=path.join("media.mp4");
                    ensure!(file.canonicalize()?==expected,"Downloaded file path changed before cleanup");
                    let metadata=std::fs::metadata(&expected)?;
                    ensure!(size==Some(metadata.len() as i64)&&modified==Some(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string()),"Downloaded file generation changed before cleanup");
                }
                // The write transaction fences new interests/playback until the
                // captured generation is removed. Only owned cache files enter here.
                std::fs::remove_dir_all(path)?;
            }
            tx.execute("DELETE FROM playback_sessions WHERE youtube_video_id=?1",[&video])?;
            tx.execute("DELETE FROM youtube_downloads WHERE video_id=?1 AND generation=?2",params![video,generation])?;
        }
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn publish_progress(
    id: String,
    db: &Database,
) -> anyhow::Result<(Option<Value>, Vec<String>)> {
    db.read("online.downloads.publish_progress", move|db|{
        let progress=db.query_row("SELECT generation,state,size,downloaded_bytes,total_bytes,eta_seconds,media_kind FROM youtube_downloads WHERE video_id=?1",[&id],|r|Ok(json!({"video_id":id,"generation":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"size":r.get::<_,Option<i64>>(2)?,"downloaded_bytes":r.get::<_,i64>(3)?,"total_bytes":r.get::<_,Option<i64>>(4)?,"eta_seconds":r.get::<_,Option<i64>>(5)?,"media_kind":r.get::<_,Option<String>>(6)?}))).optional()?;
        let users=db.prepare("SELECT user_id FROM youtube_videos WHERE video_id=?1")?.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((progress,users))
    }).await
}

pub(super) async fn cache_size(db: &Database) -> anyhow::Result<i64> {
    db.read("online.downloads.cache_size", |db| {
        Ok(db.query_row(
            "SELECT COALESCE(SUM(size),0) FROM youtube_downloads WHERE state='ready'",
            [],
            |r| r.get::<_, i64>(0),
        )?)
    })
    .await
}

pub(super) async fn publish_download(
    target: PathBuf,
    metadata: std::fs::Metadata,
    value: Value,
    video: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.downloads.publish_download", move |db| {let changed=db.execute("UPDATE youtube_downloads SET path=?3,size=?4,modified=?5,probe=?6 WHERE video_id=?1 AND generation=?2 AND state='downloading'",params![video,generation,target.to_string_lossy(),metadata.len() as i64,metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),serde_json::to_string(&value)?])?;ensure!(changed==1,"Download was cancelled");Ok(())}).await
}

pub(crate) fn record_state(
    tx: &rusqlite::Transaction<'_>,
    user: &str,
    video: &str,
    position: f64,
    duration: f64,
) -> anyhow::Result<()> {
    tx.execute("INSERT INTO youtube_state(user_id,video_id,watched,position,updated_at) SELECT ?1,?2,?3,?4,?5 WHERE EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2) ON CONFLICT(user_id,video_id) DO UPDATE SET watched=MAX(watched,excluded.watched),position=excluded.position,updated_at=excluded.updated_at",params![user,video,duration>0.0&&position>=duration*0.9,if duration>0.0 {position}else{0.0},now()])?;
    Ok(())
}

pub(super) async fn save_progress(
    db: &Database,
    id: String,
    version: String,
    progress: (i64, Option<i64>, Option<i64>, &'static str),
) -> anyhow::Result<usize> {
    db.write("online.downloads.save_progress", move|db|Ok(db.execute("UPDATE youtube_downloads SET downloaded_bytes=?3,total_bytes=?4,eta_seconds=?5,media_kind=?6 WHERE video_id=?1 AND generation=?2 AND state='downloading'",params![id,version,progress.0,progress.1,progress.2,progress.3])?)).await
}

pub(super) async fn has_interest(
    db: &Database,
    video: String,
    generation: String,
) -> anyhow::Result<bool> {
    db.read("online.downloads.has_interest", move |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE video_id=?1 AND (watchlist=1 OR pinned=1)) AND EXISTS(SELECT 1 FROM youtube_downloads WHERE video_id=?1 AND generation=?2 AND state='downloading')",params![video,generation],|r|r.get::<_,bool>(0))?)).await
}
