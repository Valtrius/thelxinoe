//! Database operations for jellyfin.audio.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn session(
    auth: String,
    user: String,
    media: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("jellyfin.audio.session", move|db|Ok(db.query_row("SELECT a.playback_id FROM compat_audio_playbacks a JOIN playback_sessions s ON s.id=a.playback_id JOIN media m ON m.id=a.media_id WHERE a.auth_session_id=?1 AND a.media_id=?2 AND s.user_id=?3 AND m.kind='track'",params![auth,media,user],|r|r.get(0)).optional()?)).await
}

pub(super) async fn stream_read_media(mid: String, db: &Database) -> anyhow::Result<bool> {
    db.read("jellyfin.audio.stream_read_media", move |db| {
        Ok(
            db.query_row("SELECT kind='track' FROM media WHERE id=?1", [mid], |r| {
                r.get::<_, bool>(0)
            })?,
        )
    })
    .await
}

pub(super) async fn stream_read_playback_sessions(
    key: String,
    db: &Database,
) -> anyhow::Result<(String, String)> {
    db.read("jellyfin.audio.stream_read_playback_sessions", move |db| {
        Ok(db.query_row(
            "SELECT state,file_id FROM playback_sessions WHERE id=?1",
            [key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?)
    })
    .await
}

pub(super) async fn stream_write_compat_playbacks(
    key: String,
    auth: String,
    media: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("jellyfin.audio.stream_write_compat_playbacks", move|db| {
        let tx=db.transaction()?;
        tx.execute("INSERT INTO compat_playbacks(playback_id) VALUES (?1)",[&key])?;
        tx.execute("INSERT INTO compat_audio_playbacks VALUES (?1,?2,?3) ON CONFLICT(auth_session_id,media_id) DO UPDATE SET playback_id=excluded.playback_id",params![auth,media,key])?;
        tx.commit()?;Ok(())
    }).await
}
