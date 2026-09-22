//! Database operations for playback.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn source(
    media: String,
    file: Option<String>,
    db: &Database,
) -> anyhow::Result<Option<Source>> {
    db.read("playback.source", move|db| {
        Ok(db.query_row("SELECT f.id,f.generation,f.edition,f.path,r.path,f.size,f.modified,f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id JOIN library_roots r ON r.id=f.root_id WHERE s.media_id=?1 AND f.present=1 AND (?2 IS NULL OR f.id=?2) ORDER BY f.edition,f.id LIMIT 1",params![media,file],|r|Ok(Source {id:r.get(0)?,media_id:media.clone(),generation:r.get(1)?,edition:r.get(2)?,path:PathBuf::from(r.get::<_,String>(3)?),root:PathBuf::from(r.get::<_,String>(4)?),size:r.get::<_,i64>(5)? as u64,modified:r.get(6)?,probe:serde_json::from_str(&r.get::<_,String>(7)?).unwrap_or_default()})).optional()?)
    }).await
}

pub(super) async fn user_preferences(
    user: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("playback.user_preferences", move |db| {
        Ok(db
            .query_row(
                "SELECT value FROM playback_preferences WHERE user_id=?1",
                [user],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn save_preferences(
    db: &Database,
    value: Preferences,
    p: Principal,
) -> anyhow::Result<()> {
    db.write("playback.save_preferences", move|db|{db.execute("INSERT INTO playback_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![p.user.id,serde_json::to_string(&value)?])?;Ok(())}).await
}

pub(super) async fn media_info_read_youtube_state(
    db: &Database,
    video: String,
    owner: String,
) -> anyhow::Result<(f64, bool)> {
    db.read("playback.media_info_read_youtube_state", move |db| {
        Ok(db
            .query_row(
                "SELECT position,watched FROM youtube_state WHERE user_id=?1 AND video_id=?2",
                params![owner, video],
                |r| Ok((r.get::<_, f64>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?
            .unwrap_or((0.0, false)))
    })
    .await
}

pub(super) async fn media_info_read_media_sources(
    db: &Database,
    user: String,
    mid: String,
) -> anyhow::Result<(Vec<String>, Vec<Value>, bool)> {
    db.read("playback.media_info_read_media_sources", move|db| {
        let files=db.prepare("SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition,f.id")?.query_map([&mid],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let progress=db.prepare("SELECT edition,position,duration FROM edition_progress WHERE user_id=?1 AND media_id=?2")?.query_map(params![user,mid],|r|Ok(json!({"edition":r.get::<_,String>(0)?,"position":r.get::<_,f64>(1)?,"duration":r.get::<_,f64>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let watched=db.query_row("SELECT watched FROM media_state WHERE user_id=?1 AND media_id=?2",params![user,mid],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        Ok((files,progress,watched))
    }).await
}

pub(super) async fn queue_matches(
    user: Principal,
    queue: crate::user_media::QueueContext,
    media: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("playback.queue_matches", move |db| {
        let saved = crate::user_media::queue_value(db, &user, &queue.client_id)?;
        Ok(saved["revision"].as_i64() == Some(queue.revision)
            && saved["items"][queue.index].as_str() == Some(&media))
    })
    .await
}

pub(super) struct PlaybackSession {
    pub(super) online_video: Option<String>,
    pub(super) mode: &'static str,
    pub(super) duration: f64,
    pub(super) key: String,
    pub(super) user: String,
    pub(super) auth: String,
    pub(super) src: Source,
    pub(super) options: Options,
    pub(super) position: Option<f64>,
    pub(super) queue: Option<crate::user_media::QueueContext>,
}

pub(super) async fn create_session(db: &Database, input: PlaybackSession) -> anyhow::Result<f64> {
    let PlaybackSession {
        online_video,
        mode,
        duration,
        key,
        user,
        auth,
        src,
        options,
        position,
        queue,
    } = input;

    db.write("playback.create_session", move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved=if let Some(video)=&online_video {tx.query_row("SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",params![user,video],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0)} else {tx.query_row("SELECT position FROM edition_progress WHERE user_id=?1 AND media_id=?2 AND edition=?3",params![user,src.media_id,src.edition],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0)};
        let position=position.unwrap_or(if saved>=duration*0.9 {0.0} else {saved}).clamp(0.0,duration);
        if !position.is_finite() {anyhow::bail!("Invalid playback position");}
        if let Some(video)=&online_video {
            let available=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_downloads d JOIN youtube_videos v ON v.video_id=d.video_id WHERE d.video_id=?1 AND d.generation=?2 AND d.state='ready' AND v.user_id=?3)",params![video,src.generation,user],|r|r.get::<_,bool>(0))?;
            anyhow::ensure!(available,"Online media changed before playback");
        }
        tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,client_id,queue_revision,queue_index,youtube_video_id) VALUES (?1,?2,?3,?4,?5,?6,?7,'ready',?8,?9,?10,?11,?12,?12,?13,?14,?15,?16)",params![key,user,auth,online_video.is_none().then_some(&src.media_id),online_video.is_none().then_some(&src.id),src.generation,src.edition,mode,serde_json::to_string(&options)?,duration,position,now(),queue.as_ref().map(|q|&q.client_id),queue.as_ref().map(|q|q.revision),queue.as_ref().map(|q|q.index as i64),online_video])?;tx.commit()?;Ok(position)
    }).await
}

pub(super) async fn session(
    id: String,
    p: Principal,
    db: &Database,
) -> anyhow::Result<Option<Session>> {
    db.read("playback.session", move|db|Ok(db.query_row("SELECT COALESCE(media_id,'youtube:'||youtube_video_id,live_media_id),COALESCE(file_id,youtube_video_id,live_media_id),generation,mode,options,duration,streaming FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3 AND state IN ('ready','playing','paused') AND updated_at>?4",params![id,p.user.id,p.session_id,now()-120],|r|Ok(Session {media:r.get(0)?,file:r.get(1)?,generation:r.get(2)?,mode:r.get(3)?,options:serde_json::from_str(&r.get::<_,String>(4)?).unwrap(),duration:r.get(5)?,streaming:r.get(6)?})).optional()?)).await
}

pub(super) async fn keepalive(db: &Database, id: String) -> anyhow::Result<()> {
    db.write("playback.keepalive", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;tx.execute("UPDATE playback_sessions SET updated_at=?1 WHERE id=?2 AND state IN ('ready','playing','paused')",params![now(),id])?;tx.execute("UPDATE playback_grants SET expires_at=?1 WHERE resource=?2 AND expires_at>?3",params![now()+120,format!("playback:{id}"),now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn cancel(db: &Database, key: String) -> anyhow::Result<()> {
    db.write("playback.cancel", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE playback_sessions SET state='stopped',updated_at=?1 WHERE id=?2",
            params![now(), key],
        )?;
        tx.execute(
            "DELETE FROM playback_grants WHERE resource=?1",
            [format!("playback:{key}")],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn report(
    user: String,
    auth: String,
    key: String,
    stopped: bool,
    db: &Database,
    input: Progress,
) -> anyhow::Result<Option<bool>> {
    db.write("playback.report", move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT COALESCE(media_id,'youtube:'||youtube_video_id,live_media_id),edition,duration,sequence,state FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3",params![key,user,auth],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,f64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))).optional()?;
        let Some((media,edition,duration,sequence,status))=row else {return Ok(None);};
        if input.sequence<=sequence || ["stopped","failed"].contains(&status.as_str()) {return Ok(Some(false));}
        let position=if duration==0.0 {input.position}else{input.position.min(duration)};
        let seconds=crate::statistics::record(&tx,&key,&input)?;
        tx.execute("UPDATE playback_sessions SET position=?1,sequence=?2,state=?3,updated_at=?4 WHERE id=?5",params![position,input.sequence,input.state,now(),key])?;
        if let Some(video)=media.strip_prefix("youtube:") {
            crate::online::downloads::record_state(&tx,&user,video,position,duration)?;
        } else if !crate::online::live::domain(&media) {
            tx.execute("INSERT INTO edition_progress VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,media_id,edition) DO UPDATE SET position=excluded.position,duration=excluded.duration,updated_at=excluded.updated_at",params![user,media,edition,position,duration,now()])?;
            tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=MAX(watched,excluded.watched),updated_at=excluded.updated_at",params![user,media,position>=duration*0.9,now()])?;
        }
        crate::history::record(&tx,&key,position,&input.state,seconds)?;
        let resource=format!("playback:{key}");
        if stopped {tx.execute("DELETE FROM playback_grants WHERE resource=?1",[resource])?;} else {tx.execute("UPDATE playback_grants SET expires_at=?1 WHERE resource=?2 AND expires_at>?3",params![now()+120,resource,now()])?;}
        crate::storage::record_event(&tx, Some(&user), "playback.changed", &json!({"id":key,"state":input.state}))?;
        tx.commit()?;Ok(Some(true))
    }).await
}

pub(super) async fn fail(id: String, db: &Database) -> anyhow::Result<()> {
    db.write("playback.fail", move |db| {
        db.execute(
            "UPDATE playback_sessions SET state='failed',updated_at=?1 WHERE id=?2",
            params![now(), id],
        )?;
        db.execute(
            "DELETE FROM playback_grants WHERE resource=?1",
            [format!("playback:{id}")],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn maintain_write_playback_sessions(db: &Database) -> anyhow::Result<()> {
    db.write("playback.maintain_write_playback_sessions", |db|{db.execute("UPDATE playback_sessions SET state='stopped' WHERE state IN ('ready','playing','paused')",[])?;crate::history::finish_stale(db)?;Ok(())}).await
}

pub(super) async fn expire_sessions(db: &Database) -> anyhow::Result<Vec<String>> {
    db.write("playback.expire_sessions", |db| {
        db.execute("UPDATE playback_sessions SET state='stopped' WHERE state IN ('ready','playing','paused') AND updated_at<=?1",[now()-120])?;
        crate::history::finish_stale(db)?;
        Ok(db.prepare("SELECT p.id FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.state IN ('ready','playing','paused') AND s.expires_at>?1")?.query_map([now()],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await
}
