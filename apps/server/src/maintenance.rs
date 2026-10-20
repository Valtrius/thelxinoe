//! Daily database housekeeping: finished jobs, expired sessions, old tool runs, stale
//! segment fingerprints and files missing for a month.

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
            "Pruned finished jobs, expired sessions, old tool runs, stale fingerprints and missing files"
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
        assert_eq!(
            rows(&state, "SELECT id FROM jobs ORDER BY id").await,
            ["recent-failed", "waiting"]
        );
        let sessions = rows(&state, "SELECT id FROM sessions").await;
        assert!(
            sessions.contains(&"current".to_owned()) && !sessions.contains(&"expired".to_owned())
        );
        assert_eq!(
            rows(&state, "SELECT id FROM tool_jobs WHERE tool='ffmpeg'").await,
            ["check-3"]
        );
    }

    #[tokio::test]
    async fn pruning_drops_stale_fingerprints_and_long_missing_files_but_keeps_their_media() {
        let (_temp, state, _cookie) = fixture().await;
        let old = now() - 31 * 86400;
        state
            .db
            .write("test.fixture", move |db| {
                db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Films','movies','/media')", [])?;
                db.execute(
                    "INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen) VALUES ('auth','alice','auth','web','Browser',?1,?2,?1)",
                    rusqlite::params![old, now() + 3600],
                )?;
                for (file, generation, present, scanned) in [
                    ("current", "g2", 1, old),
                    ("gone", "g", 0, old),
                    ("recent", "g", 0, now() - 86400),
                    ("playing", "g", 0, old),
                ] {
                    db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES (?1,'root','movie',?1,?1,1)", [file])?;
                    db.execute(
                        "INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,present,scanned_at) VALUES (?1,'root','/media/'||?1,?2,1,'1',?1,'{}',?3,?4)",
                        rusqlite::params![file, generation, present, scanned],
                    )?;
                    db.execute("INSERT INTO media_sources VALUES (?1,?1)", [file])?;
                    db.execute("INSERT INTO segment_fingerprints VALUES (?1,?2,'intro',0,'[]')", [file, generation])?;
                }
                db.execute("INSERT INTO segment_fingerprints VALUES ('current','g1','intro',0,'[]')", [])?;
                db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','gone',1,1)", [])?;
                db.execute(
                    "INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('play','alice','auth','playing','playing','g','','stopped','direct','{}',100,1,1)",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        prune(&state).await.unwrap();
        assert_eq!(
            rows(
                &state,
                "SELECT file_id||':'||generation FROM segment_fingerprints ORDER BY 1"
            )
            .await,
            ["current:g2", "playing:g", "recent:g"]
        );
        assert_eq!(
            rows(&state, "SELECT id FROM media_files ORDER BY id").await,
            ["current", "playing", "recent"]
        );
        assert_eq!(
            rows(&state, "SELECT file_id FROM media_sources ORDER BY file_id").await,
            ["current", "playing", "recent"]
        );
        assert_eq!(
            rows(&state, "SELECT id FROM media ORDER BY id").await,
            ["current", "gone", "playing", "recent"]
        );
        assert_eq!(
            rows(&state, "SELECT media_id FROM media_state WHERE watched=1").await,
            ["gone"]
        );
    }

    async fn rows(state: &AppState, sql: &'static str) -> Vec<String> {
        state
            .db
            .read("test.rows", move |c| {
                Ok(c.prepare(sql)?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap()
    }
}
