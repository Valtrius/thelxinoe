//! Database operations for online.youtube.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn stored(user: String, db: &Database) -> anyhow::Result<Option<Stored>> {
    db.read("online.youtube.stored", move|db|Ok(db.query_row("SELECT generation,credential,expires_at FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND status='connected' AND credential IS NOT NULL",[user],|r|Ok(Stored{generation:r.get(0)?,encrypted:r.get(1)?,expires:r.get(2)?})).optional()?)).await
}

pub(super) async fn invalidate(
    user: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.youtube.invalidate", move|db|{
        db.execute("UPDATE online_accounts SET status='reconnect_required',credential=NULL,expires_at=0,updated_at=?1 WHERE user_id=?2 AND provider='youtube' AND generation=?3",params![now(),user,generation])?;
        Ok(())
    }).await
}

pub(super) async fn access(
    encrypted: Vec<u8>,
    expires: i64,
    owner: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.write("online.youtube.access", move|db|Ok(db.execute("UPDATE online_accounts SET credential=?1,expires_at=?2 WHERE user_id=?3 AND provider='youtube' AND generation=?4 AND status='connected' AND credential IS NOT NULL",params![encrypted,expires,owner,generation])?==1)).await
}

pub(super) async fn get(user: String, generation: String, db: &Database) -> anyhow::Result<()> {
    db.write("online.youtube.get", move|db|{db.execute("UPDATE online_accounts SET expires_at=0 WHERE user_id=?1 AND provider='youtube' AND generation=?2",params![user,generation])?;Ok(())}).await
}
