//! Database operations for jobs.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn recover(db: &Database) -> anyhow::Result<()> {
    db.write("jobs.recover", |c| {
        c.execute(
            "UPDATE jobs SET state='queued',started_at=NULL WHERE state='running'",
            [],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn enqueue(
    db: &Database,
    kind: String,
    payload: serde_json::Value,
    key: String,
) -> anyhow::Result<String> {
    db.write("jobs.enqueue", move |c| {
        c.execute("INSERT OR IGNORE INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,?2,?3,?4,'queued',?5,?5)", params![id(),kind,payload.to_string(),key,now()])?;
        Ok(c.query_row("SELECT id FROM jobs WHERE dedupe_key=?1", [key], |r| r.get(0))?)
    }).await
}

pub(super) async fn claim(db: &Database) -> anyhow::Result<Option<Job>> {
    db.write("jobs.claim", |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job = tx.query_row("UPDATE jobs SET state='running',attempts=attempts+1,started_at=?1 WHERE id=(SELECT id FROM jobs WHERE state='queued' AND available_at<=?1 ORDER BY created_at LIMIT 1) RETURNING id,kind,payload,state,attempts,error", [now()], |r| Ok(Job { id:r.get(0)?, kind:r.get(1)?, payload:serde_json::from_str(&r.get::<_,String>(2)?).unwrap_or_default(), state:r.get(3)?, attempts:r.get(4)?, error:r.get(5)? })).optional()?;
        tx.commit()?; Ok(job)
    }).await
}

pub(super) async fn finish(db: &Database, id: String, error: Option<String>) -> anyhow::Result<()> {
    db.write("jobs.finish", move |c| {
        c.execute(
            "UPDATE jobs SET state=?1,completed_at=?2,error=?3 WHERE id=?4",
            params![
                if error.is_some() {
                    "failed"
                } else {
                    "complete"
                },
                now(),
                error,
                id
            ],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn checkpoint(db: &Database, job: Job) -> anyhow::Result<()> {
    db.write("jobs.checkpoint", move |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO job_effects VALUES (?1,?2)",
            params![job.id, job.payload.to_string()],
        )?;
        tx.execute(
            "UPDATE jobs SET state='complete',completed_at=?1 WHERE id=?2",
            params![now(), job.id],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
