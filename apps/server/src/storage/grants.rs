//! Database operations for grants.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn issue(
    hash: String,
    p: Principal,
    resource: String,
    db: &Database,
    ttl: i64,
) -> anyhow::Result<()> {
    db.write("grants.issue", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM playback_grants WHERE expires_at<=?1", [now()])?;
        tx.execute(
            "INSERT INTO playback_grants VALUES (?1,?2,?3,?4,?5)",
            params![hash, p.user.id, p.session_id, resource, now() + ttl],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn resolve(
    hash: String,
    resource: String,
    db: &Database,
    consume: bool,
) -> anyhow::Result<Option<Principal>> {
    db.write("grants.resolve", move|db|{
        let query=|db:&rusqlite::Connection|db.query_row("SELECT u.id,u.username,u.role,u.timezone,s.id,s.transport FROM playback_grants g JOIN sessions s ON s.id=g.session_id JOIN user_profiles u ON u.id=g.user_id WHERE g.token_hash=?1 AND g.resource=?2 AND g.expires_at>?3 AND s.expires_at>?3",params![hash,resource,now()],|r|Ok(Principal{user:thelxinoe_auth::user_row(r)?,session_id:r.get(4)?,transport:r.get(5)?})).optional();
        if !consume{return Ok(query(db)?);}
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let principal=query(&tx)?;
        tx.execute("DELETE FROM playback_grants WHERE token_hash=?1",[hash])?;tx.commit()?;Ok(principal)
    }).await
}

pub(super) async fn event_ticket(db: &Database) -> anyhow::Result<i64> {
    db.read("grants.event_ticket", |db| {
        Ok(
            db.query_row("SELECT COALESCE(MAX(id),0) FROM events", [], |r| {
                r.get::<_, i64>(0)
            })?,
        )
    })
    .await
}
