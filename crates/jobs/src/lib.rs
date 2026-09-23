#[path = "storage.rs"]
mod storage;

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
        storage::recover(&self.0).await
    }
    pub async fn enqueue(
        &self,
        kind: String,
        payload: serde_json::Value,
        key: String,
    ) -> Result<String> {
        storage::enqueue(&self.0, kind, payload, key).await
    }
    pub async fn claim(&self) -> Result<Option<Job>> {
        storage::claim(&self.0, None).await
    }
    pub async fn claim_services(&self) -> Result<Option<Job>> {
        storage::claim(&self.0, Some(true)).await
    }
    pub async fn claim_general(&self) -> Result<Option<Job>> {
        storage::claim(&self.0, Some(false)).await
    }
    pub async fn finish(&self, job: &Job, error: Option<String>) -> Result<()> {
        let id = job.id.clone();
        storage::finish(&self.0, id, error).await
    }
    pub async fn checkpoint(&self, job: &Job) -> Result<()> {
        let job = job.clone();
        storage::checkpoint(&self.0, job).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn service_jobs_can_be_claimed_together_without_taking_general_jobs() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let queue = Queue(Database::open(temp.path().join("db"))?);
        for (kind, key) in [
            ("stack.install", "sonarr"),
            ("service.update", "radarr"),
            ("checkpoint", "general"),
        ] {
            queue
                .enqueue(kind.into(), serde_json::json!({}), key.into())
                .await?;
        }
        let (first, second) = tokio::join!(queue.claim_services(), queue.claim_services());
        let first = first?.unwrap();
        let second = second?.unwrap();
        assert_ne!(first.id, second.id);
        assert_ne!(first.kind, "checkpoint");
        assert_ne!(second.kind, "checkpoint");
        assert!(queue.claim_services().await?.is_none());
        let general = queue.claim_general().await?.unwrap();
        assert_eq!(general.kind, "checkpoint");
        assert!(queue.claim_general().await?.is_none());
        Ok(())
    }

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
                .write("test.fixture", |c| Ok(c.query_row(
                    "SELECT count(*) FROM job_effects",
                    [],
                    |r| r.get::<_, i64>(0)
                )?))
                .await?,
            1
        );
        Ok(())
    }
}
