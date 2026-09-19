use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use thelxinoe_core::{id, now};
use thelxinoe_database::Database;

#[derive(Clone)]
pub struct Queue(pub Database);
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub state: String,
    pub attempts: i64,
    pub error: Option<String>,
}

impl Queue {
    pub async fn recover(&self) -> Result<()> {
        self.0
            .call(|c| {
                c.execute(
                    "UPDATE jobs SET state='queued',started_at=NULL WHERE state='running'",
                    [],
                )?;
                Ok(())
            })
            .await
    }
    pub async fn enqueue(
        &self,
        kind: String,
        payload: serde_json::Value,
        key: String,
    ) -> Result<String> {
        self.0.call(move |c| {
            c.execute("INSERT OR IGNORE INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,?2,?3,?4,'queued',?5,?5)", params![id(),kind,payload.to_string(),key,now()])?;
            Ok(c.query_row("SELECT id FROM jobs WHERE dedupe_key=?1", [key], |r| r.get(0))?)
        }).await
    }
    pub async fn claim(&self) -> Result<Option<Job>> {
        self.0.call(|c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let job = tx.query_row("UPDATE jobs SET state='running',attempts=attempts+1,started_at=?1 WHERE id=(SELECT id FROM jobs WHERE state='queued' AND available_at<=?1 ORDER BY created_at LIMIT 1) RETURNING id,kind,payload,state,attempts,error", [now()], |r| Ok(Job { id:r.get(0)?, kind:r.get(1)?, payload:serde_json::from_str(&r.get::<_,String>(2)?).unwrap_or_default(), state:r.get(3)?, attempts:r.get(4)?, error:r.get(5)? })).optional()?;
            tx.commit()?; Ok(job)
        }).await
    }
    pub async fn finish(&self, job: &Job, error: Option<String>) -> Result<()> {
        let id = job.id.clone();
        self.0
            .call(move |c| {
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
    pub async fn checkpoint(&self, job: &Job) -> Result<()> {
        let job = job.clone();
        self.0
            .call(move |c| {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn interrupted_job_recovers_without_duplicating_completed_effect() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("db");
        let queue = Queue(Database::open(&path)?);
        let id = queue
            .enqueue(
                "checkpoint".into(),
                serde_json::json!({"value":1}),
                "key".into(),
            )
            .await?;
        assert_eq!(
            id,
            queue
                .enqueue("checkpoint".into(), serde_json::json!({}), "key".into())
                .await?
        );
        queue.claim().await?.unwrap();
        drop(queue);
        let queue = Queue(Database::open(path)?);
        queue.recover().await?;
        let job = queue.claim().await?.unwrap();
        assert_eq!(job.attempts, 2);
        queue.checkpoint(&job).await?;
        queue.checkpoint(&job).await?;
        queue.recover().await?;
        assert!(queue.claim().await?.is_none());
        assert_eq!(
            queue
                .0
                .call(
                    |c| Ok(c.query_row("SELECT count(*) FROM job_effects", [], |r| r
                        .get::<_, i64>(0))?)
                )
                .await?,
            1
        );
        Ok(())
    }
}
