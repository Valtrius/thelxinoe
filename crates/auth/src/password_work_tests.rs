use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn cancelled_password_callers_keep_running_jobs_within_capacity() {
    let slots = Arc::new(tokio::sync::Semaphore::new(4));
    let running = Arc::new(AtomicUsize::new(0));
    let mut callers = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..4 {
        let permit = slots.clone().acquire_owned().await.unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let count = running.clone();
        callers.push(tokio::spawn(async move {
            password_work(Some(permit), move || {
                count.fetch_add(1, Ordering::SeqCst);
                started.send(()).unwrap();
                resume.recv().unwrap();
                count.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        }));
        ready.await.unwrap();
        releases.push(release);
    }
    for caller in callers {
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
    }
    let exceeded = slots.clone().try_acquire_owned().is_ok();
    let active = running.load(Ordering::SeqCst);
    for release in releases {
        release.send(()).unwrap();
    }
    let returned =
        tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire_many(4)).await;
    assert_eq!(active, 4);
    assert!(
        !exceeded,
        "cancelled callers must not admit more password work"
    );
    assert!(returned.is_ok(), "completed jobs must return capacity");
    assert_eq!(running.load(Ordering::SeqCst), 0);
}
