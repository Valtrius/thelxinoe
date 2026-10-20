//! Daily database housekeeping: finished jobs, expired sessions and old tool runs.

#[path = "storage/maintenance.rs"]
mod storage;

use crate::AppState;
use std::time::Duration;
use thelxinoe_core::now;

/// Prunes once a day, starting a few minutes after startup so it never competes with it.
pub async fn run(state: AppState) -> anyhow::Result<()> {
    tokio::time::sleep(Duration::from_secs(300)).await;
    loop {
        prune(&state).await?;
        tokio::time::sleep(Duration::from_secs(86400)).await;
    }
}

async fn prune(state: &AppState) -> anyhow::Result<()> {
    let started = now();
    let mut removed = 0;
    loop {
        let batch = storage::prune(&state.db, started).await?;
        if batch == 0 {
            break;
        }
        removed += batch;
        tokio::task::yield_now().await;
    }
    if removed > 0 {
        tracing::info!(
            removed,
            "Pruned finished jobs, expired sessions and old tool runs"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    #[tokio::test]
    async fn pruning_keeps_recent_unfinished_and_latest_rows() {
        let (_temp, state, _cookie) = fixture().await;
        let old = now() - 8 * 86400;
        state
            .db
            .write("test.fixture", move |db| {
                let job = |id: &str, state: &str, completed: Option<i64>| {
                    db.execute(
                        "INSERT INTO jobs(id,kind,subject,payload,state,available_at,completed_at,created_at) VALUES (?1,'library.scan','root','{}',?2,?3,?4,?3)",
                        rusqlite::params![id, state, old, completed],
                    )
                };
                job("old-complete", "complete", Some(old))?;
                job("old-failed", "failed", Some(old))?;
                job("retired", "complete", None)?;
                job("recent-failed", "failed", Some(now()))?;
                job("waiting", "queued", None)?;
                for (id, expires) in [("expired", old), ("current", now() + 3600)] {
                    db.execute(
                        "INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen) VALUES (?1,'alice',?1,'web','Browser',?2,?3,?2)",
                        rusqlite::params![id, old, expires],
                    )?;
                }
                db.execute("INSERT INTO server_tools(id,channel) VALUES ('ffmpeg','stable') ON CONFLICT DO NOTHING", [])?;
                for (id, stage) in [("check-1", "complete"), ("check-2", "failed"), ("check-3", "complete")] {
                    db.execute(
                        "INSERT INTO tool_jobs(id,tool,action,manual,revision,stage,created_at,updated_at) VALUES (?1,'ffmpeg','check',0,0,?2,?3,?3)",
                        rusqlite::params![id, stage, old],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        prune(&state).await.unwrap();
        let rows = |sql: &'static str| {
            let db = state.db.clone();
            async move {
                db.read("test.rows", move |c| {
                    Ok(c.prepare(sql)?
                        .query_map([], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?)
                })
                .await
                .unwrap()
            }
        };
        assert_eq!(
            rows("SELECT id FROM jobs ORDER BY id").await,
            ["recent-failed", "waiting"]
        );
        let sessions = rows("SELECT id FROM sessions").await;
        assert!(
            sessions.contains(&"current".to_owned()) && !sessions.contains(&"expired".to_owned())
        );
        assert_eq!(
            rows("SELECT id FROM tool_jobs WHERE tool='ffmpeg'").await,
            ["check-3"]
        );
    }
}
