use anyhow::{Result, anyhow};
use rusqlite::Connection;
use serde::Serialize;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc, oneshot};

#[derive(Clone, Debug)]
pub struct Options {
    pub readers: usize,
    pub queue_capacity: usize,
    pub admission_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            readers: 2,
            queue_capacity: 64,
            admission_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
pub enum AccessError {
    Closed,
    Overloaded,
    WorkerStopped,
}
impl std::fmt::Display for AccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Closed => "Database is shutting down",
            Self::Overloaded => "Database queue admission timed out",
            Self::WorkerStopped => "Database worker stopped before returning a result",
        })
    }
}
impl std::error::Error for AccessError {}

#[derive(Default)]
struct Counters {
    queued: AtomicUsize,
    active: AtomicUsize,
    completed: AtomicU64,
    failed: AtomicU64,
    wait_micros: AtomicU64,
    execution_micros: AtomicU64,
}

#[derive(Debug, Serialize)]
pub struct WorkStats {
    pub queued: usize,
    pub active: usize,
    pub completed: u64,
    pub failed: u64,
    pub wait_micros: u64,
    pub execution_micros: u64,
}

impl Counters {
    fn snapshot(&self) -> WorkStats {
        WorkStats {
            queued: self.queued.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            wait_micros: self.wait_micros.load(Ordering::Relaxed),
            execution_micros: self.execution_micros.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Metrics {
    pub reads: WorkStats,
    pub writes: WorkStats,
}

type Work = Box<dyn FnOnce(&mut Connection) -> bool + Send>;
struct Job {
    operation: &'static str,
    submitted: Instant,
    work: Work,
}

#[derive(Default)]
struct Completion {
    remaining: AtomicUsize,
    changed: Notify,
}
struct WorkerExit(Arc<Completion>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.remaining.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_waiters();
    }
}

pub(crate) struct Lane {
    sender: Mutex<Option<mpsc::Sender<Job>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    completion: Arc<Completion>,
    counters: Arc<Counters>,
    timeout: Duration,
}

fn micros(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

impl Lane {
    pub(crate) fn start(
        name: &str,
        connections: Vec<Connection>,
        options: &Options,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>(options.queue_capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let completion = Arc::new(Completion::default());
        let counters = Arc::new(Counters::default());
        let mut threads = Vec::new();
        for (index, mut conn) in connections.into_iter().enumerate() {
            let receiver = receiver.clone();
            let counters = counters.clone();
            let completion = completion.clone();
            completion.remaining.fetch_add(1, Ordering::Relaxed);
            let exit = WorkerExit(completion);
            threads.push(
                std::thread::Builder::new()
                    .name(format!("database-{name}-{index}"))
                    .spawn(move || {
                        let _exit = exit;
                        loop {
                            // Only receiving is locked. Each reader executes on its own connection.
                            let job = receiver
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .blocking_recv();
                            let Some(job) = job else { break };
                            counters.queued.fetch_sub(1, Ordering::Relaxed);
                            counters.active.fetch_add(1, Ordering::Relaxed);
                            let waited = micros(job.submitted.elapsed());
                            let started = Instant::now();
                            let healthy = (job.work)(&mut conn);
                            let elapsed = micros(started.elapsed());
                            counters.active.fetch_sub(1, Ordering::Relaxed);
                            counters.completed.fetch_add(1, Ordering::Relaxed);
                            counters.wait_micros.fetch_add(waited, Ordering::Relaxed);
                            counters
                                .execution_micros
                                .fetch_add(elapsed, Ordering::Relaxed);
                            tracing::debug!(
                                operation = job.operation,
                                wait_micros = waited,
                                execution_micros = elapsed,
                                "Database operation completed"
                            );
                            if !healthy {
                                break;
                            }
                        }
                    })?,
            );
        }
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            threads: Mutex::new(threads),
            completion,
            counters,
            timeout: options.admission_timeout,
        })
    }

    pub(crate) async fn submit<T, F>(&self, operation: &'static str, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let submitted = Instant::now();
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .cloned()
            .ok_or(AccessError::Closed)?;
        let permit = tokio::time::timeout(self.timeout, sender.reserve_owned())
            .await
            .map_err(|_| AccessError::Overloaded)?
            .map_err(|_| AccessError::WorkerStopped)?;
        let (reply, response) = oneshot::channel();
        let counters = self.counters.clone();
        counters.queued.fetch_add(1, Ordering::Relaxed);
        permit.send(Job {
            operation,
            submitted,
            work: Box::new(move |conn| {
                // An accepted write runs to completion even when its HTTP caller leaves.
                let mut result = catch_unwind(AssertUnwindSafe(|| f(conn)))
                    .unwrap_or_else(|_| Err(anyhow!("Database operation panicked")));
                let mut healthy = true;
                if !conn.is_autocommit() {
                    healthy = conn.execute_batch("ROLLBACK").is_ok();
                    result = Err(anyhow!(
                        "Database operation left a transaction open; changes rolled back"
                    ));
                }
                if result.is_err() {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
                }
                let _ = reply.send(result);
                healthy
            }),
        });
        response.await.map_err(|_| AccessError::WorkerStopped)?
    }

    pub(crate) fn close(&self) {
        self.sender.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    pub(crate) async fn join(&self) -> Result<()> {
        loop {
            let changed = self.completion.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.completion.remaining.load(Ordering::Acquire) == 0 {
                break;
            }
            changed.await;
        }
        let threads = std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        tokio::task::spawn_blocking(move || {
            for thread in threads {
                thread
                    .join()
                    .map_err(|_| anyhow!("Database worker panicked"))?;
            }
            Ok(())
        })
        .await?
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) struct Runtime {
    pub(crate) writer: Lane,
    pub(crate) readers: Lane,
}
impl Runtime {
    pub(crate) fn metrics(&self) -> Metrics {
        Metrics {
            reads: self.readers.counters.snapshot(),
            writes: self.writer.counters.snapshot(),
        }
    }
}
