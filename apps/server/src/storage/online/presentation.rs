//! Database operations for online.presentation.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn cancel(
    db: &Database,
    provider: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("online.presentation.cancel", move|db|{
        if provider=="youtube" {db.execute("DELETE FROM oauth_attempts WHERE user_id=?1 AND session_id=?2 AND provider='youtube'",params![p.user.id,p.session_id])?;}
        else {db.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND session_id=?2",params![p.user.id,p.session_id])?;}
        Ok(())
    }).await
}

pub(super) async fn refresh(
    db: &Database,
    provider: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<usize> {
    db.write("online.presentation.refresh", move|db|{
        if provider=="twitch" {Ok(db.execute("UPDATE twitch_sync SET next_run=?1 WHERE user_id=?2 AND failures=0 AND (last_complete IS NULL OR last_complete<?1-30) AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=twitch_sync.user_id AND a.provider='twitch' AND a.status='connected' AND a.generation=twitch_sync.generation)",params![now(),p.user.id])?)}
        else {Ok(db.execute("UPDATE kick_channels SET next_run=?1 WHERE user_id=?2 AND failures=0 AND updated_at<?1-30 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=kick_channels.user_id AND a.provider='kick' AND a.status='connected')",params![now(),p.user.id])?)}
    }).await
}

pub(super) async fn resolve(
    db: &Database,
    p: thelxinoe_core::Principal,
    id: String,
) -> anyhow::Result<bool> {
    db.write("online.presentation.resolve", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count=tx.query_row("SELECT COUNT(*) FROM youtube_videos WHERE user_id=?1 AND metadata_at=0",[&p.user.id],|r|r.get::<_,i64>(0))?;
        if count>=1000{return Ok(false);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,?2,?2) ON CONFLICT DO NOTHING",params![p.user.id,id])?;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),p.user.id])?;
        tx.commit()?;Ok(true)
    }).await
}
