//! Database operations for managers.episodes.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn audit(
    db: &Database,
    p: thelxinoe_core::Principal,
    action: &'static str,
    series: String,
) -> anyhow::Result<()> {
    db.write("managers.episodes.audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![p.user.id, action, series, now()],
        )?;
        Ok(())
    })
    .await
}
