//! Database operations for managers.movies.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn audit(
    db: &Database,
    actor: Option<String>,
    action: &'static str,
    movie: String,
) -> anyhow::Result<()> {
    db.write("managers.movies.audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![actor, action, movie, now()],
        )?;
        Ok(())
    })
    .await
}
