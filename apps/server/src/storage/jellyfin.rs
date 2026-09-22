//! Database operations for jellyfin.
use thelxinoe_database::Database;

pub(super) async fn handle(p: &thelxinoe_core::Principal, db: &Database) -> anyhow::Result<()> {
    let p = p.clone();

    db.write("jellyfin.handle", move |db| {
        db.execute("DELETE FROM sessions WHERE id=?1", [p.session_id])?;
        Ok(())
    })
    .await
}
