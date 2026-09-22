//! Database operations for online.streams.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn info(owner: String, id: String, db: &Database) -> anyhow::Result<f64> {
    db.read("online.streams.info", move |db| {
        Ok(db
            .query_row(
                "SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",
                params![owner, id],
                |r| r.get::<_, f64>(0),
            )
            .optional()?
            .unwrap_or(0.0))
    })
    .await
}

pub(super) struct StreamSession {
    pub(super) live_title: Option<String>,
    pub(super) mode: &'static str,
    pub(super) key: String,
    pub(super) owner: String,
    pub(super) auth: String,
    pub(super) video: String,
    pub(super) input: Options,
    pub(super) duration: f64,
    pub(super) live: bool,
    pub(super) position: Option<f64>,
}

pub(super) async fn create_with_delivery_write_twitch_streams(
    db: &Database,
    input: StreamSession,
) -> anyhow::Result<f64> {
    let StreamSession {
        live_title,
        mode,
        key,
        owner,
        auth,
        video,
        input,
        duration,
        live,
        position,
    } = input;

    db.write("online.streams.create_with_delivery_write_twitch_streams", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(title)=live_title {
            anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM twitch_streams WHERE user_id=?1 AND 'twitch:'||channel_id=?2 AND active=1) OR EXISTS(SELECT 1 FROM kick_channels WHERE user_id=?1 AND 'kick:'||slug=?2)",params![owner,video],|r|r.get::<_,bool>(0))?,"Channel was removed before playback");
            tx.execute("INSERT INTO live_media VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET title=excluded.title",params![video,title])?;
            tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,live_media_id,streaming) VALUES (?1,?2,?3,?1,'live','ready','transcode',?4,0,0,?5,?5,?6,1)",params![key,owner,auth,serde_json::to_string(&input)?,now(),video])?;
            tx.commit()?;return Ok(0.0)
        }
        anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",params![owner,video],|r|r.get::<_,bool>(0))?,"Video was removed before playback");
        let saved=tx.query_row("SELECT position FROM youtube_state WHERE user_id=?1 AND video_id=?2",params![owner,video],|r|r.get::<_,f64>(0)).optional()?.unwrap_or(0.0);
        let start=if live {0.0}else {position.unwrap_or(if saved>=duration*0.9 {0.0}else{saved}).clamp(0.0,(duration-0.1).max(0.0))};
        tx.execute("INSERT INTO youtube_media(video_id) VALUES (?1) ON CONFLICT DO NOTHING",[&video])?;
        tx.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,position,created_at,updated_at,youtube_video_id,streaming) VALUES (?1,?2,?3,?1,'public','ready',?9,?4,?5,?6,?7,?7,?8,1)",params![key,owner,auth,serde_json::to_string(&input)?,duration,start,now(),video,mode])?;
        tx.commit()?;Ok(start)
    }).await
}

pub(super) async fn create_with_delivery_write_playback_sessions(
    id: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write(
        "online.streams.create_with_delivery_write_playback_sessions",
        move |db| {
            db.execute(
                "UPDATE playback_sessions SET state='failed' WHERE id=?1",
                [id],
            )?;
            Ok(())
        },
    )
    .await
}

pub(super) async fn create_with_delivery_read_playback_sessions(
    key: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("online.streams.create_with_delivery_read_playback_sessions", move |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.id=p.auth_session_id WHERE p.id=?1 AND p.state='ready' AND s.expires_at>?2 AND (EXISTS(SELECT 1 FROM youtube_videos v WHERE v.user_id=p.user_id AND v.video_id=p.youtube_video_id) OR EXISTS(SELECT 1 FROM twitch_streams t WHERE t.user_id=p.user_id AND 'twitch:'||t.channel_id=p.live_media_id AND t.active=1) OR EXISTS(SELECT 1 FROM kick_channels k WHERE k.user_id=p.user_id AND 'kick:'||k.slug=p.live_media_id)))",params![key,now()],|r|r.get::<_,bool>(0))?)).await
}
