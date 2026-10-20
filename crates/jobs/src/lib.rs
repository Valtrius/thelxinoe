#[path = "storage.rs"]
mod storage;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use thelxinoe_core::{id, now};
use thelxinoe_database::Database;

#[derive(Clone)]
pub struct Queue(pub Database);
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    /// What the job acts on: a library root, request, transfer, provision or media item.
    pub subject: String,
    pub payload: serde_json::Value,
    pub state: String,
    pub attempts: i64,
    pub error: Option<String>,
}
/// The newest job of one kind for one subject.
#[derive(Clone, Debug)]
pub struct Latest {
    pub id: String,
    pub state: String,
    pub created_at: i64,
}

/// Queues a `kind` job acting on `subject` that becomes available after `delay` seconds.
/// While an identical job is still queued, returns that one instead of adding another.
pub fn enqueue_in(
    c: &Connection,
    kind: &str,
    subject: &str,
    payload: &serde_json::Value,
    delay: i64,
) -> rusqlite::Result<String> {
    let payload = payload.to_string();
    match storage::find_queued(c, kind, subject, &payload)? {
        Some(queued) => Ok(queued),
        None => storage::insert(c, kind, subject, &payload, delay),
    }
}

/// Queues the newest `kind` job for `subject` again after `delay` seconds, keeping its
/// payload and attempt count. Returns false when the subject has no such job.
pub fn requeue(c: &Connection, kind: &str, subject: &str, delay: i64) -> rusqlite::Result<bool> {
    storage::requeue_newest(c, kind, subject, delay)
}

/// Queues a claimed job again after `delay` seconds instead of finishing it.
pub fn retry(c: &Connection, job: &str, delay: i64) -> rusqlite::Result<bool> {
    storage::requeue_job(c, job, delay)
}

pub fn newest(c: &Connection, kind: &str, subject: &str) -> rusqlite::Result<Option<Latest>> {
    storage::newest(c, kind, subject)
}

/// Deletes up to `limit` jobs that finished before `before` and returns how many it
/// deleted, so callers can prune in batches between other writes.
pub fn prune(c: &Connection, before: i64, limit: i64) -> rusqlite::Result<usize> {
    storage::prune_batch(c, before, limit)
}

impl Queue {
    pub async fn recover(&self) -> Result<()> {
        storage::recover(&self.0).await
    }
    pub async fn enqueue(
        &self,
        kind: &str,
        subject: &str,
        payload: serde_json::Value,
    ) -> Result<String> {
        storage::enqueue(&self.0, kind.into(), subject.into(), payload).await
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn queue() -> Result<(tempfile::TempDir, Queue)> {
        let temp = tempfile::tempdir()?;
        let queue = Queue(Database::open(temp.path().join("db"))?);
        Ok((temp, queue))
    }

    #[tokio::test]
    async fn service_jobs_can_be_claimed_together_without_taking_general_jobs() -> Result<()> {
        let (_temp, queue) = queue()?;
        for (kind, subject) in [
            ("stack.install", "sonarr"),
            ("service.update", "radarr"),
            ("library.scan", "root"),
        ] {
            queue.enqueue(kind, subject, json!({})).await?;
        }
        let (first, second) = tokio::join!(queue.claim_services(), queue.claim_services());
        let first = first?.unwrap();
        let second = second?.unwrap();
        assert_ne!(first.id, second.id);
        assert_ne!(first.kind, "library.scan");
        assert_ne!(second.kind, "library.scan");
        assert!(queue.claim_services().await?.is_none());
        let general = queue.claim_general().await?.unwrap();
        assert_eq!(
            (general.kind.as_str(), general.subject.as_str()),
            ("library.scan", "root")
        );
        assert!(queue.claim_general().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn an_identical_job_is_queued_once_until_it_starts() -> Result<()> {
        let (_temp, queue) = queue()?;
        let scan =
            |root: &'static str| queue.enqueue("library.scan", root, json!({"root_id":root}));
        let first = scan("a").await?;
        assert_eq!(first, scan("a").await?);
        // Another subject, or other work for the same subject, is a job of its own.
        assert_ne!(first, scan("b").await?);
        let other = queue
            .enqueue("library.scan", "a", json!({"root_id":"a","full":true}))
            .await?;
        assert_ne!(first, other);
        // Changes made while a job runs need another run after it.
        let running = queue.claim().await?.unwrap();
        assert_eq!(running.id, first);
        let next = scan("a").await?;
        assert_ne!(first, next);
        queue.finish(&running, None).await?;
        assert_eq!(next, scan("a").await?);
        Ok(())
    }

    #[tokio::test]
    async fn an_interrupted_job_runs_again_after_a_restart() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("db");
        let queue = Queue(Database::open(&path)?);
        let id = queue
            .enqueue("library.scan", "root", json!({"root_id":"root"}))
            .await?;
        queue.claim().await?.unwrap();
        drop(queue);
        let queue = Queue(Database::open(path)?);
        queue.recover().await?;
        let job = queue.claim().await?.unwrap();
        assert_eq!((job.id.as_str(), job.attempts), (id.as_str(), 2));
        queue.finish(&job, None).await?;
        queue.recover().await?;
        assert!(queue.claim().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn requeueing_reuses_the_subjects_newest_job_and_pruning_keeps_unfinished_work()
    -> Result<()> {
        let (_temp, queue) = queue()?;
        let first = queue
            .enqueue("service.transfer", "t", json!({"id":"t"}))
            .await?;
        let job = queue.claim().await?.unwrap();
        queue.finish(&job, Some("Docker was busy".into())).await?;
        let (requeued, missing, latest) = queue
            .0
            .write("test.requeue", |c| {
                Ok((
                    requeue(c, "service.transfer", "t", 0)?,
                    requeue(c, "service.transfer", "other", 0)?,
                    newest(c, "service.transfer", "t")?,
                ))
            })
            .await?;
        assert!(requeued && !missing);
        let latest = latest.unwrap();
        assert_eq!(
            (latest.id.as_str(), latest.state.as_str()),
            (first.as_str(), "queued")
        );
        let again = queue.claim().await?.unwrap();
        assert_eq!(
            (again.id.as_str(), again.attempts, again.error),
            (first.as_str(), 2, None)
        );
        assert!(
            queue
                .0
                .write("test.retry", move |c| Ok(retry(c, &again.id, 60)?))
                .await?
        );
        assert!(
            queue.claim().await?.is_none(),
            "a retry waits for its delay"
        );

        let done = queue.enqueue("library.scan", "root", json!({})).await?;
        let job = queue.claim().await?.unwrap();
        assert_eq!(job.id, done);
        queue.finish(&job, None).await?;
        let (pruned, left) = queue
            .0
            .write("test.prune", |c| {
                let pruned = prune(c, now() + 1, 100)?;
                let left = c.query_row("SELECT group_concat(id) FROM jobs", [], |r| {
                    r.get::<_, String>(0)
                })?;
                Ok((pruned, left))
            })
            .await?;
        assert_eq!((pruned, left), (1, first));
        Ok(())
    }
}
