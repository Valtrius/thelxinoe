//! Database operations for maintenance.
use rusqlite::params;
use thelxinoe_auth::lifetime;
use thelxinoe_database::Database;

/// Rows a pass deletes per table, so other writes interleave with a large backlog.
const BATCH: i64 = 2000;
/// Missing files a pass deletes. Each one's references are checked across several tables
/// that aren't indexed by file, so these batches stay small.
const FILE_BATCH: i64 = 100;
/// How long a missing file keeps its row, links and segments, so a library that is offline
/// for a while, such as an unmounted share, finds them again when it returns.
const MISSING_FILE_GRACE: i64 = 30 * 86400;

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
        // Sign-in already refuses these; a device left unused or past the cap is forgotten.
        removed += c.execute(
            "DELETE FROM auth_remembered_devices WHERE rowid IN (SELECT rowid FROM auth_remembered_devices WHERE last_seen<=?1 OR created_at<=?2 LIMIT ?3)",
            params![now - lifetime::REMEMBERED_IDLE, now - lifetime::MAX, BATCH],
        )?;
        // Each tool keeps its latest run, however old.
        removed += c.execute(
            "DELETE FROM tool_jobs WHERE rowid IN (SELECT rowid FROM tool_jobs WHERE stage IN ('complete','failed','canceled') AND updated_at<?1 AND rowid NOT IN (SELECT max(rowid) FROM tool_jobs GROUP BY tool) LIMIT ?2)",
            params![week, BATCH],
        )?;
        // A changed file gets a new generation and never matches its old fingerprints again.
        removed += c.execute(
            "DELETE FROM segment_fingerprints WHERE rowid IN (SELECT p.rowid FROM segment_fingerprints p JOIN media_files f ON f.id=p.file_id WHERE p.generation<>f.generation LIMIT ?1)",
            [BATCH],
        )?;
        // A file missing for the grace period goes with its links, segments and fingerprints.
        // Its media stays, and with it watch state and history, for a file that returns later.
        // A playback session still naming the file keeps it until the session is pruned.
        let missing: String = c.query_row(
            "SELECT json_group_array(id) FROM (SELECT id FROM media_files f WHERE present=0 AND scanned_at<?1 AND NOT EXISTS(SELECT 1 FROM playback_sessions p WHERE p.file_id=f.id) LIMIT ?2)",
            params![now - MISSING_FILE_GRACE, FILE_BATCH],
            |r| r.get(0),
        )?;
        if missing != "[]" {
            let tx = c.transaction()?;
            for links in ["media_sources", "local_trailers", "manager_bindings"] {
                tx.execute(
                    &format!(
                        "DELETE FROM {links} WHERE file_id IN (SELECT value FROM json_each(?1))"
                    ),
                    [&missing],
                )?;
            }
            removed += tx.execute(
                "DELETE FROM media_files WHERE id IN (SELECT value FROM json_each(?1))",
                [&missing],
            )?;
            tx.commit()?;
        }
        Ok(removed)
    })
    .await
}
