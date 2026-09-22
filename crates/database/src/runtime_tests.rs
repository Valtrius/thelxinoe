use super::*;
use std::sync::mpsc;
use tokio::sync::oneshot;

#[tokio::test]
async fn a_read_operation_keeps_one_snapshot_across_a_concurrent_commit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path().join("db"))?;
    db.write("test.seed", |c| {
        c.execute("INSERT INTO settings VALUES ('snapshot','before')", [])?;
        Ok(())
    })
    .await?;
    let (entered, ready) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let reader = db.clone();
    let reading = tokio::spawn(async move {
        reader
            .read("test.snapshot", move |c| {
                let query = || -> rusqlite::Result<String> {
                    c.query_row("SELECT value FROM settings WHERE key='snapshot'", [], |r| {
                        r.get(0)
                    })
                };
                let before = query()?;
                let _ = entered.send(());
                blocked.recv()?;
                Ok((before, query()?))
            })
            .await
    });
    ready.await?;
    db.write("test.change", |c| {
        c.execute("UPDATE settings SET value='after' WHERE key='snapshot'", [])?;
        Ok(())
    })
    .await?;
    release.send(())?;
    assert_eq!(reading.await??, ("before".into(), "before".into()));
    assert_eq!(
        db.read("test.new_snapshot", |c| Ok(c.query_row(
            "SELECT value FROM settings WHERE key='snapshot'",
            [],
            |r| r.get::<_, String>(0)
        )?))
        .await?,
        "after"
    );
    db.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn readers_see_committed_state_while_writes_wait_in_order() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path().join("db"))?;
    db.write("test.seed", |c| {
        c.execute("INSERT INTO settings VALUES('counter','0')", [])?;
        Ok(())
    })
    .await?;
    let (entered, ready) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let writer = db.clone();
    let first = tokio::spawn(async move {
        writer
            .write("test.first", move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute("UPDATE settings SET value='1' WHERE key='counter'", [])?;
                let _ = entered.send(());
                blocked.recv()?;
                tx.commit()?;
                Ok(())
            })
            .await
    });
    ready.await?;
    let writer = db.clone();
    let second = tokio::spawn(async move {
        writer
            .write("test.second", |c| {
                c.execute(
                    "UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='counter'",
                    [],
                )?;
                Ok(())
            })
            .await
    });
    let seen = tokio::time::timeout(
        Duration::from_secs(1),
        db.read("test.read", |c| {
            Ok(
                c.query_row("SELECT value FROM settings WHERE key='counter'", [], |r| {
                    r.get::<_, String>(0)
                })?,
            )
        }),
    )
    .await??;
    assert_eq!(seen, "0");
    assert!(!second.is_finished());
    release.send(())?;
    first.await??;
    second.await??;
    assert_eq!(
        db.read("test.read", |c| Ok(c.query_row(
            "SELECT value FROM settings WHERE key='counter'",
            [],
            |r| r.get::<_, String>(0)
        )?))
        .await?,
        "2"
    );
    db.shutdown().await?;
    assert_eq!(db.metrics().writes.completed, 3);
    assert_eq!(db.metrics().reads.completed, 2);
    Ok(())
}

#[tokio::test]
async fn readers_run_concurrently_and_cannot_write() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path().join("db"))?;
    let (entered, ready) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let reader = db.clone();
    let first = tokio::spawn(async move {
        reader
            .read("test.slow_read", move |_| {
                let _ = entered.send(());
                blocked.recv()?;
                Ok(())
            })
            .await
    });
    ready.await?;
    tokio::time::timeout(
        Duration::from_secs(1),
        db.read("test.fast_read", |c| {
            c.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        }),
    )
    .await??;
    assert!(
        db.read("test.invalid_write", |c| {
            c.execute("INSERT INTO settings VALUES ('forbidden','value')", [])?;
            Ok(())
        })
        .await
        .is_err()
    );
    release.send(())?;
    first.await??;
    db.shutdown().await?;
    assert_eq!(db.metrics().reads.failed, 1);
    Ok(())
}

#[tokio::test]
async fn queue_overload_is_explicit_and_shutdown_drains_accepted_writes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let db = Database::open_with_options(
        &path,
        Options {
            queue_capacity: 1,
            admission_timeout: Duration::from_millis(100),
            ..Options::default()
        },
    )?;
    let (entered, ready) = oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let writer = db.clone();
    let first = tokio::spawn(async move {
        writer
            .write("test.first", move |c| {
                let _ = entered.send(());
                blocked.recv()?;
                c.execute("INSERT INTO settings VALUES ('first','saved')", [])?;
                Ok(())
            })
            .await
    });
    ready.await?;
    let writer = db.clone();
    let second = tokio::spawn(async move {
        writer
            .write("test.second", |c| {
                c.execute("INSERT INTO settings VALUES ('second','saved')", [])?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while db.metrics().writes.queued != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let error = db.write("test.overload", |_| Ok(())).await.unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AccessError>(),
        Some(AccessError::Overloaded)
    ));
    // Cancelling a request after acceptance does not discard its write.
    first.abort();
    second.abort();
    let closing = db.clone();
    let shutdown = tokio::spawn(async move { closing.shutdown().await });
    tokio::task::yield_now().await;
    release.send(())?;
    shutdown.await??;
    assert_eq!(
        connection(&path, true)?.query_row(
            "SELECT COUNT(*) FROM settings WHERE key IN ('first','second')",
            [],
            |r| r.get::<_, u32>(0)
        )?,
        2
    );
    assert!(matches!(
        db.read("test.closed", |_| Ok(()))
            .await
            .unwrap_err()
            .downcast_ref::<AccessError>(),
        Some(AccessError::Closed)
    ));
    assert!(db.write("test.closed", |_| Ok(())).await.is_err());
    db.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn panic_and_unfinished_transactions_roll_back_before_connection_reuse() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path().join("db"))?;
    assert!(
        db.write::<(), _>("test.panic", |c| {
            c.execute_batch("BEGIN IMMEDIATE; INSERT INTO settings VALUES ('panic','unsaved');")?;
            panic!("Intentional worker failure")
        })
        .await
        .is_err()
    );
    assert!(
        db.write("test.unfinished", |c| {
            c.execute_batch(
                "BEGIN IMMEDIATE; INSERT INTO settings VALUES ('unfinished','unsaved');",
            )?;
            Ok(())
        })
        .await
        .is_err()
    );
    db.write("test.reuse", |c| {
        c.execute("INSERT INTO settings VALUES ('after','saved')", [])?;
        Ok(())
    })
    .await?;
    assert_eq!(
        db.read("test.check", |c| Ok(c.query_row(
            "SELECT COUNT(*) FROM settings WHERE key IN ('panic','unfinished','after')",
            [],
            |r| r.get::<_, u32>(0)
        )?))
        .await?,
        1
    );
    db.shutdown().await?;
    assert_eq!(db.metrics().writes.failed, 2);
    Ok(())
}
