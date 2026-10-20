//! Database operations for maintenance.
use rusqlite::params;
use thelxinoe_database::Database;

/// Rows a pass deletes per table, so other writes interleave with a large backlog.
const BATCH: i64 = 2000;

/// Deletes one batch of rows that outlived their use and returns how many it removed.
pub(super) async fn prune(db: &Database, now: i64) -> anyhow::Result<usize> {
    db.write("maintenance.prune", move |c| {
        let week = now - 7 * 86400;
        // Failed jobs stay in attention for a week, and their history with them.
        let mut removed = thelxinoe_jobs::prune(c, week, BATCH)?;
        // An expired session ends what it owned, as signing out does.
        removed += c.execute(
            "DELETE FROM sessions WHERE rowid IN (SELECT rowid FROM sessions WHERE expires_at<?1 LIMIT ?2)",
            params![now, BATCH],
        )?;
        // Each tool keeps its latest run, however old.
        removed += c.execute(
            "DELETE FROM tool_jobs WHERE rowid IN (SELECT rowid FROM tool_jobs WHERE stage IN ('complete','failed','canceled') AND updated_at<?1 AND rowid NOT IN (SELECT max(rowid) FROM tool_jobs GROUP BY tool) LIMIT ?2)",
            params![week, BATCH],
        )?;
        Ok(removed)
    })
    .await
}
