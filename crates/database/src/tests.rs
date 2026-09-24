use super::*;

fn user(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "INSERT INTO users(id,username,password_hash,role,created_at) VALUES (?1,?1,'unused','user',1)",
        [id],
    )?;
    Ok(())
}

#[tokio::test]
async fn initialization_reopen_and_online_backup_preserve_data() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("main.db");
    let db = Database::open(&path)?;
    db.write("test.fixture", |c| {
        c.execute(
            "INSERT INTO settings VALUES ('timezone','Europe/Paris')",
            [],
        )?;
        assert_eq!(
            c.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))?,
            "wal"
        );
        assert_eq!(
            c.pragma_query_value(None, "foreign_keys", |r| r.get::<_, u32>(0))?,
            1
        );
        Ok(())
    })
    .await?;
    let snapshot = temp.path().join("snapshot.db");
    db.backup(snapshot.clone()).await?;
    assert!(db.backup(snapshot.clone()).await.is_err());
    drop(db);
    for path in [&path, &snapshot] {
        assert_eq!(verify_snapshot(path)?, SCHEMA_VERSION);
        let reopened = Database::open(path)?;
        assert_eq!(
            reopened.connect()?.query_row(
                "SELECT value FROM settings WHERE key='timezone'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "Europe/Paris"
        );
    }
    let missing = temp.path().join("missing.db");
    assert!(verify_snapshot(&missing).is_err());
    assert!(!missing.exists());
    Ok(())
}

#[test]
fn incompatible_databases_are_rejected_without_rewriting_them() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("unrecognized.db");
    let conn = Connection::open(&path)?;
    conn.execute_batch("CREATE TABLE original(value TEXT); INSERT INTO original VALUES('keep');")?;
    assert!(Database::open(&path).is_err());
    assert!(verify_snapshot(&path).is_err());
    assert_eq!(
        conn.query_row("SELECT value FROM original", [], |r| r.get::<_, String>(0))?,
        "keep"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |r| r
            .get::<_, u32>(0))?,
        1
    );

    let path = temp.path().join("future.db");
    let db = Database::open(&path)?;
    db.connect()?
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)?;
    assert!(Database::open(&path).is_err());
    // Snapshot inspection reports identity for release/recovery callers;
    // opening state for normal use requires the exact supported version.
    assert_eq!(verify_snapshot(&path)?, SCHEMA_VERSION + 1);
    db.connect()?.pragma_update(None, "application_id", 123)?;
    assert!(verify_snapshot(&path).is_err());
    Ok(())
}

#[test]
fn concurrent_initialization_publishes_one_complete_schema() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("main.db");
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Database::open(path)
            })
        })
        .collect();
    let databases: Vec<_> = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Result<_>>()?;
    assert_eq!(verify_snapshot(&path)?, SCHEMA_VERSION);
    for db in databases {
        assert_eq!(
            db.connect()?
                .query_row("SELECT COUNT(*) FROM user_profiles", [], |r| r
                    .get::<_, u32>(0))?,
            0
        );
    }
    Ok(())
}

#[test]
fn playback_ownership_is_enforced_and_history_survives_source_removal() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Database::open(temp.path().join("main.db"))?.connect()?;
    user(&db, "alice")?;
    user(&db, "bob")?;
    db.execute_batch("INSERT INTO sessions VALUES ('session','alice','hash','web','Browser',1,9999999999,1);
        INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Films','movies','/media');
        INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('film','root','movie','film','Film',1);
        INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('file','root','/media/film.mkv','g',1,'1','hash','{}',1);
        INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('play','alice','session','film','file','g','','playing','direct','{}',100,1,1);
        INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,device_name,started_at,updated_at,position,duration,state) VALUES ('play','alice','movies','film','Film','movie','Browser',1,2,50,100,'playing');
        INSERT INTO compat_playbacks VALUES ('play',7);
        INSERT INTO playback_statistics VALUES ('alice','movies','film','Film','film','Film','','movie',1,2,100,0);
        INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','film',1,1);")?;
    assert!(
        db.execute("UPDATE playback_sessions SET user_id='bob'", [])
            .is_err()
    );
    assert!(
        db.execute(
            "INSERT INTO playback_grants VALUES ('bad','bob','session','resource',9999999999)",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("UPDATE playback_sessions SET media_id=NULL", [])
            .is_err()
    );
    assert!(db.execute("UPDATE media_state SET watched=2", []).is_err());
    assert!(
        db.query_row("SELECT completed FROM playback_statistics", [], |r| r
            .get::<_, bool>(0))?
    );
    db.execute("DELETE FROM sessions", [])?;
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM compat_playbacks", [], |r| r
            .get::<_, u32>(0))?,
        0
    );
    db.execute("DELETE FROM media WHERE id='film'", [])?;
    assert_eq!(
        db.query_row("SELECT media_title FROM playback_history", [], |r| r
            .get::<_, String>(0))?,
        "Film"
    );
    assert_eq!(
        db.query_row("SELECT media_title FROM playback_statistics", [], |r| {
            r.get::<_, String>(0)
        })?,
        "Film"
    );
    db.execute("DELETE FROM users WHERE id='alice'", [])?;
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM playback_history", [], |r| r
            .get::<_, u32>(0))?,
        0
    );
    assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
    Ok(())
}
