//! Database operations for jellyfin.
use thelxinoe_database::Database;

/// A page's `LIMIT`; SQLite reads a negative one as no limit.
pub(super) fn sql_limit(limit: Option<usize>) -> i64 {
    limit.map_or(-1, |limit| i64::try_from(limit).unwrap_or(-1))
}

pub(super) async fn handle(p: &thelxinoe_core::Principal, db: &Database) -> anyhow::Result<()> {
    let p = p.clone();

    db.write("jellyfin.handle", move |db| {
        db.execute("DELETE FROM sessions WHERE id=?1", [p.session_id])?;
        Ok(())
    })
    .await
}
