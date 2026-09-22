//! Database operations for jellyfin.playback.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn sources(mid: String, db: &Database) -> anyhow::Result<Vec<String>> {
    db.read("jellyfin.playback.sources", move|db|Ok(db.prepare("SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition,f.id")?.query_map([mid],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?)).await
}

pub(super) async fn info_source(key: String, db: &Database) -> anyhow::Result<()> {
    db.write("jellyfin.playback.info_source", move |db| {
        db.execute(
            "INSERT INTO compat_playbacks(playback_id) VALUES (?1)",
            [key],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn report(
    media: String,
    key: String,
    user: String,
    auth: String,
    db: &Database,
) -> anyhow::Result<Option<(i64, f64)>> {
    db.write("jellyfin.playback.report", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT c.sequence,s.position FROM compat_playbacks c JOIN playback_sessions s ON s.id=c.playback_id WHERE s.id=?1 AND COALESCE(s.media_id,'youtube:'||s.youtube_video_id,s.live_media_id)=?2 AND s.user_id=?3 AND s.auth_session_id=?4",params![key,media,user,auth],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,f64>(1)?))).optional()?;
        if row.is_some(){tx.execute("UPDATE compat_playbacks SET sequence=sequence+1 WHERE playback_id=?1",[key])?;}
        tx.commit()?;Ok(row)
    }).await
}

pub(super) async fn stop_encoding(
    key: String,
    user: String,
    auth: String,
    device: Option<String>,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("jellyfin.playback.stop_encoding", move|db|Ok(db.query_row("SELECT s.media_id FROM playback_sessions s JOIN compat_playbacks c ON c.playback_id=s.id JOIN compat_devices d ON d.session_id=s.auth_session_id WHERE s.id=?1 AND s.user_id=?2 AND s.auth_session_id=?3 AND (?4 IS NULL OR d.device_id=?4)",params![key,user,auth,device],|r|r.get::<_,String>(0)).optional()?)).await
}

pub(super) async fn stream_tag(
    hash: String,
    media: String,
    file: Option<String>,
    explicit: Option<String>,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("jellyfin.playback.stream_tag", move|db|Ok(db.query_row("SELECT s.id FROM playback_grants g JOIN playback_sessions s ON g.resource='playback:'||s.id JOIN compat_playbacks c ON c.playback_id=s.id WHERE g.token_hash=?1 AND s.media_id=?2 AND s.mode='direct' AND (?3 IS NULL OR s.file_id=?3) AND (?4 IS NULL OR s.id=?4)",params![hash,media,file,explicit],|r|r.get::<_,String>(0)).optional()?)).await
}

pub(super) async fn stream(
    key: String,
    uid: String,
    auth: String,
    media: String,
    file: Option<String>,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("jellyfin.playback.stream", move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3 AND media_id=?4 AND mode='direct' AND (?5 IS NULL OR file_id=?5))",params![key,uid,auth,media,file],|r|r.get::<_,bool>(0))?)).await
}
