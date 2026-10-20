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
    db.execute_batch("INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen) VALUES ('session','alice','hash','web','Browser',1,9999999999,1);
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

fn frozen_schema(version: u32) -> String {
    let path = format!("{}/schemas/{version}.sql", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{path} is missing"))
        .replace("\r\n", "\n")
}

/// What a migrated database must share with a fresh one. Whitespace and the
/// order columns were added in do not count. CHECK constraints live only in a
/// table's SQL text, which migrations may format differently, so a migration
/// that changes one needs its own test.
fn shape(c: &Connection) -> Result<Vec<String>> {
    let objects = c
        .prepare("SELECT type,name,tbl_name,COALESCE(sql,'') FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut shape = Vec::new();
    for (kind, name, table, sql) in objects {
        if kind != "table" {
            let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
            shape.push(format!("{kind} {name} on {table}: {sql}"));
            continue;
        }
        let (strict, without_rowid): (bool, bool) = c.query_row(
            "SELECT strict,wr FROM pragma_table_list WHERE schema='main' AND name=?1",
            [&name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        shape.push(format!(
            "table {name} strict={strict} without_rowid={without_rowid}"
        ));
        let mut details = c
            .prepare("SELECT name,type,\"notnull\",dflt_value,pk FROM pragma_table_xinfo(?1)")?
            .query_map([&name], |r| {
                Ok(format!(
                    "  column {} {} notnull={} default={:?} pk={}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        details.extend(
            c.prepare("SELECT \"table\",\"from\",\"to\",on_update,on_delete FROM pragma_foreign_key_list(?1)")?
                .query_map([&name], |r| {
                    Ok(format!(
                        "  reference {} -> {}.{} update={} delete={}",
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        // Indexes behind UNIQUE and PRIMARY KEY constraints have generated names, so
        // they are compared by what they cover. Named indexes also match by SQL above.
        let indexes = c
            .prepare("SELECT name,\"unique\",origin,partial FROM pragma_index_list(?1)")?
            .query_map([&name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (index, unique, origin, partial) in indexes {
            let columns = c
                .prepare("SELECT COALESCE(name,'<expression>'),\"desc\",coll FROM pragma_index_xinfo(?1) WHERE key=1 ORDER BY seqno")?
                .query_map([&index], |r| {
                    Ok(format!(
                        "{}{} {}",
                        r.get::<_, String>(0)?,
                        if r.get::<_, bool>(1)? { " desc" } else { "" },
                        r.get::<_, String>(2)?
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let label = if origin == "c" { index } else { origin };
            details.push(format!(
                "  index {label} unique={unique} partial={partial} ({})",
                columns.join(", ")
            ));
        }
        details.sort();
        shape.extend(details);
    }
    Ok(shape)
}

// A shipped schema never changes; the next one starts as a frozen copy.
#[test]
fn current_schema_matches_its_frozen_copy() {
    assert!(
        frozen_schema(SCHEMA_VERSION) == SCHEMA.replace("\r\n", "\n"),
        "schema.sql differs from schemas/{SCHEMA_VERSION}.sql. If schema {SCHEMA_VERSION} shipped in a release (schemas/SHA256SUMS lists it), bump SCHEMA_VERSION, add its upgrade to MIGRATIONS and copy schema.sql to the new schemas/<version>.sql. Before its first release, update schemas/{SCHEMA_VERSION}.sql instead."
    );
}

// Releasing a schema pins its digest (`pnpm run version:sync`); editing it fails here.
#[test]
fn released_schemas_never_change() {
    use sha2::{Digest, Sha256};
    let sums = include_str!("../schemas/SHA256SUMS").replace("\r\n", "\n");
    let mut released = Vec::new();
    for line in sums.lines().filter(|line| !line.is_empty()) {
        let (digest, file) = line.split_once("  ").expect("<sha256>  <version>.sql");
        let version: u32 = file.trim_end_matches(".sql").parse().unwrap();
        let actual = Sha256::digest(frozen_schema(version).as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            actual, digest,
            "schemas/{file} shipped in a release and must not change; add the next schema version instead"
        );
        released.push(version);
    }
    assert!(released.contains(&MIGRATION_BASE));
}

// Every released schema still supported upgrades to what a fresh database gets.
#[test]
fn released_schemas_upgrade_to_the_fresh_schema() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fresh = temp.path().join("fresh.db");
    drop(Database::open(&fresh)?);
    let expected = shape(&Connection::open(&fresh)?)?;
    for version in MIGRATION_BASE..=SCHEMA_VERSION {
        let path = temp.path().join(format!("{version}.db"));
        let conn = Connection::open(&path)?;
        conn.execute_batch(&frozen_schema(version))?;
        conn.pragma_update(None, "application_id", APPLICATION_ID)?;
        conn.pragma_update(None, "user_version", version)?;
        drop(conn);
        drop(Database::open(&path)?);
        assert_eq!(verify_snapshot(&path)?, SCHEMA_VERSION);
        assert_eq!(
            shape(&Connection::open(&path)?)?,
            expected,
            "schema {version} upgrades to a different shape"
        );
    }
    Ok(())
}

#[test]
fn upgrades_run_in_order_from_the_stored_version() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("CREATE TABLE t(a INTEGER, b INTEGER); PRAGMA user_version=2;")?;
    let steps = [
        "ALTER TABLE t ADD COLUMN b INTEGER",
        "ALTER TABLE t ADD COLUMN c INTEGER",
        "ALTER TABLE t ADD COLUMN d INTEGER",
    ];
    let columns = |conn: &Connection| -> Result<Vec<String>> {
        Ok(conn
            .prepare("SELECT name FROM pragma_table_info('t')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    };
    // Schema 2 already has the first step's column.
    upgrade(&conn, 2, 1, &steps)?;
    assert_eq!(columns(&conn)?, ["a", "b", "c", "d"]);
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))?,
        4
    );
    upgrade(&conn, 4, 1, &steps)?;
    assert_eq!(columns(&conn)?, ["a", "b", "c", "d"]);
    // A failing step names the versions it was upgrading between.
    conn.execute_batch("PRAGMA user_version=3")?;
    let error = upgrade(&conn, 3, 1, &steps).unwrap_err();
    assert!(error.to_string().contains("schema 3 to 4"), "{error}");
    Ok(())
}
