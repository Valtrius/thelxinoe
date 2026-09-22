//! Database operations for jellyfin.auth.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn issue(
    hash: String,
    db: &Database,
    user: String,
    device: Device,
) -> anyhow::Result<(String, String)> {
    db.write("jellyfin.auth.issue", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let session:String=tx.query_row("SELECT id FROM sessions WHERE token_hash=?1",[hash],|r|r.get(0))?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1 AND id IN (SELECT session_id FROM compat_devices WHERE device_id=?2 AND client=?3)",params![user,device.id,device.client])?;
        tx.execute("INSERT INTO compat_devices VALUES (?1,?2,?3,?4)",params![session,device.id,device.client,device.version])?;
        let date:String=tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')",[],|r|r.get(0))?;tx.commit()?;Ok((session,date))
    }).await
}

pub(super) async fn user_dto(user: String, db: &Database) -> anyhow::Result<Option<String>> {
    db.read("jellyfin.auth.user_dto", move |db| {
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
