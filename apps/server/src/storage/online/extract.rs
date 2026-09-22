//! Database operations for online.extract.
use thelxinoe_database::Database;

pub(super) async fn inspect(db: &Database, user: String, video: String) -> anyhow::Result<bool> {
    db.read("online.extract.inspect", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",
            rusqlite::params![user, video],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}
