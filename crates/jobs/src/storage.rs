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
    subject: String,
    payload: serde_json::Value,
) -> anyhow::Result<String> {
    db.write("jobs.enqueue", move |c| {
        Ok(enqueue_in(c, &kind, &subject, &payload, 0)?)
    })
    .await
}

pub(super) async fn claim(db: &Database, services: Option<bool>) -> anyhow::Result<Option<Job>> {
    db.write("jobs.claim", move |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job = tx.query_row("UPDATE jobs SET state='running',attempts=attempts+1,started_at=?1 WHERE id=(SELECT id FROM jobs WHERE state='queued' AND available_at<=?1 AND (?2 IS NULL OR (kind IN ('stack.install','service.update','service.transfer','recyclarr.sync'))=?2) ORDER BY created_at LIMIT 1) RETURNING id,kind,subject,payload,state,attempts,error", params![now(), services], |r| Ok(Job { id:r.get(0)?, kind:r.get(1)?, subject:r.get(2)?, payload:serde_json::from_str(&r.get::<_,String>(3)?).unwrap_or_default(), state:r.get(4)?, attempts:r.get(5)?, error:r.get(6)? })).optional()?;
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

pub(super) fn find_queued(
    c: &Connection,
    kind: &str,
    subject: &str,
    payload: &str,
) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT id FROM jobs WHERE kind=?1 AND subject=?2 AND state='queued' AND payload=?3 ORDER BY created_at LIMIT 1",
        params![kind, subject, payload],
        |r| r.get(0),
    )
    .optional()
}

pub(super) fn insert(
    c: &Connection,
    kind: &str,
    subject: &str,
    payload: &str,
    delay: i64,
) -> rusqlite::Result<String> {
    let job = id();
    let created = now();
    c.execute(
        "INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES (?1,?2,?3,?4,'queued',?5,?6)",
        params![job, kind, subject, payload, created + delay, created],
    )?;
    Ok(job)
}

pub(super) fn requeue_newest(
    c: &Connection,
    kind: &str,
    subject: &str,
    delay: i64,
) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE jobs SET state='queued',available_at=?3,started_at=NULL,completed_at=NULL,error=NULL WHERE id=(SELECT id FROM jobs WHERE kind=?1 AND subject=?2 ORDER BY created_at DESC,rowid DESC LIMIT 1)",
        params![kind, subject, now() + delay],
    )? > 0)
}

pub(super) fn requeue_job(c: &Connection, job: &str, delay: i64) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE jobs SET state='queued',available_at=?2,started_at=NULL,completed_at=NULL,error=NULL WHERE id=?1",
        params![job, now() + delay],
    )? > 0)
}

pub(super) fn newest(
    c: &Connection,
    kind: &str,
    subject: &str,
) -> rusqlite::Result<Option<Latest>> {
    c.query_row(
        "SELECT id,state,created_at FROM jobs WHERE kind=?1 AND subject=?2 ORDER BY created_at DESC,rowid DESC LIMIT 1",
        params![kind, subject],
        |r| {
            Ok(Latest {
                id: r.get(0)?,
                state: r.get(1)?,
                created_at: r.get(2)?,
            })
        },
    )
    .optional()
}

pub(super) fn prune_batch(c: &Connection, before: i64, limit: i64) -> rusqlite::Result<usize> {
    // Jobs finished without a completion time (retired or cancelled by their owner)
    // age from when they started or were created.
    c.execute(
        "DELETE FROM jobs WHERE rowid IN (SELECT rowid FROM jobs WHERE state IN ('complete','failed') AND COALESCE(completed_at,started_at,created_at)<?1 LIMIT ?2)",
        params![before, limit],
    )
}
