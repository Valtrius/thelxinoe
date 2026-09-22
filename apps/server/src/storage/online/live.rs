//! Database operations for online.live.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn authorize_read_kick_channels(
    channel: String,
    user: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("online.live.authorize_read_kick_channels", move |db| {
        Ok(db
            .query_row(
                "SELECT slug,slug||' - '||title FROM kick_channels WHERE user_id=?1 AND slug=?2",
                params![user, channel],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn authorize_read_twitch_streams(
    channel: String,
    user: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("online.live.authorize_read_twitch_streams", move|db|Ok(db.query_row("SELECT login,display_name||' - '||title FROM twitch_streams WHERE user_id=?1 AND channel_id=?2 AND active=1",params![user,channel],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)).await
}

pub(super) async fn extract(channel: String, db: &Database) -> anyhow::Result<Option<String>> {
    db.read("online.live.extract", move |db| {
        Ok(db
            .query_row(
                "SELECT login FROM twitch_streams WHERE channel_id=?1 AND active=1 LIMIT 1",
                [channel],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}
