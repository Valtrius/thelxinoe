use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const SCHEMA_VERSION: u32 = 27;

/// Inspect a quiesced database without applying migrations or creating missing files.
pub fn verify_snapshot(path: &Path) -> Result<u32> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let checks = conn
        .prepare("PRAGMA integrity_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(checks == ["ok"], "Database integrity check failed");
    anyhow::ensure!(
        !conn.prepare("PRAGMA foreign_key_check")?.exists([])?,
        "Database has invalid references"
    );
    let schema: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?;
    anyhow::ensure!(schema > 0, "Missing database schema");
    Ok(schema)
}

#[derive(Clone)]
pub struct Database {
    path: Arc<PathBuf>,
    slots: Arc<tokio::sync::Semaphore>,
    // Keep WAL/SHM alive between calls rather than repeatedly closing the last connection.
    _anchor: Arc<std::sync::Mutex<Option<Connection>>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Self {
            path: Arc::new(path.as_ref().to_owned()),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
            _anchor: Arc::new(std::sync::Mutex::new(None)),
        };
        let mut conn = db.connect()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY);",
        )?;
        let version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        let migrations = [
            include_str!("../migrations/001.sql"),
            include_str!("../migrations/002.sql"),
            include_str!("../migrations/003.sql"),
            include_str!("../migrations/004.sql"),
            include_str!("../migrations/005.sql"),
            include_str!("../migrations/006.sql"),
            include_str!("../migrations/007.sql"),
            include_str!("../migrations/008.sql"),
            include_str!("../migrations/009.sql"),
            include_str!("../migrations/010.sql"),
            include_str!("../migrations/011.sql"),
            include_str!("../migrations/012.sql"),
            include_str!("../migrations/013.sql"),
            include_str!("../migrations/014.sql"),
            include_str!("../migrations/015.sql"),
            include_str!("../migrations/016.sql"),
            include_str!("../migrations/017.sql"),
            include_str!("../migrations/018.sql"),
            include_str!("../migrations/019.sql"),
            include_str!("../migrations/020.sql"),
            include_str!("../migrations/021.sql"),
            include_str!("../migrations/022.sql"),
            include_str!("../migrations/023.sql"),
            include_str!("../migrations/024.sql"),
            include_str!("../migrations/025.sql"),
            include_str!("../migrations/026.sql"),
            include_str!("../migrations/027.sql"),
        ];
        if version > migrations.len() as i64 {
            anyhow::bail!("Database is newer than this server; use the matching release");
        }
        for (index, migration) in migrations.iter().enumerate().skip(version as usize) {
            tx.execute_batch(migration)?;
            tx.execute(
                "INSERT INTO schema_migrations VALUES (?1)",
                [(index + 1) as i64],
            )?;
        }
        tx.commit()?;
        *db._anchor.lock().expect("new database anchor") = Some(conn);
        Ok(db)
    }
    pub fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(self.path.as_ref()).context("Open database")?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(conn)
    }
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        let permit = self.slots.clone().acquire_owned().await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f(&mut db.connect()?)
        })
        .await?
    }
    pub async fn backup(&self, destination: PathBuf) -> Result<()> {
        self.call(move |conn| {
            anyhow::ensure!(!destination.exists(), "Backup destination already exists");
            conn.backup(rusqlite::MAIN_DB, &destination, None)?;
            verify_snapshot(&destination)?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_watchlist_migration_preserves_existing_private_saved_videos() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        for version in 1..=26 {
            let path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("migrations/{version:03}.sql"));
            db.execute_batch(&std::fs::read_to_string(path)?)?;
        }
        for user in ["alice", "bob"] {
            db.execute(
                "INSERT INTO users VALUES(?1,?1,'unused','user','UTC',1)",
                [user],
            )?;
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,'abcdefghijk','Saved video')",[user])?;
            db.execute("INSERT INTO youtube_state(user_id,video_id,watchlist,position,added_at,updated_at) VALUES(?1,'abcdefghijk',1,123,1,1)",[user])?;
        }
        db.execute_batch(include_str!("../migrations/027.sql"))?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM youtube_watchlist_items", [], |r| r
                .get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM youtube_watchlists WHERE is_default=1 AND name='Watch Later'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT position FROM youtube_state WHERE user_id='alice'",
                [],
                |r| r.get::<_, f64>(0)
            )?,
            123.0
        );
        db.execute("DELETE FROM users WHERE id='alice'", [])?;
        assert_eq!(
            db.query_row("SELECT user_id FROM youtube_watchlist_items", [], |r| {
                r.get::<_, String>(0)
            })?,
            "bob"
        );
        assert!(db.query_row(
            "SELECT watchlist FROM youtube_state WHERE user_id='bob'",
            [],
            |r| r.get::<_, bool>(0)
        )?);
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        Ok(())
    }
    #[test]
    fn online_migration_preserves_local_sessions_and_compatibility_references() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        for migration in [
            include_str!("../migrations/001.sql"),
            include_str!("../migrations/002.sql"),
            include_str!("../migrations/003.sql"),
            include_str!("../migrations/004.sql"),
            include_str!("../migrations/005.sql"),
            include_str!("../migrations/006.sql"),
            include_str!("../migrations/007.sql"),
            include_str!("../migrations/008.sql"),
        ] {
            db.execute_batch(migration)?;
        }
        db.execute_batch("INSERT INTO users VALUES ('u','u','unused','user','UTC',1);
            INSERT INTO sessions VALUES ('s','u','hash','jellyfin','TV',1,9999999999,1);
            INSERT INTO library_roots(id,name,kind,path) VALUES ('r','r','movies','/media');
            INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('m','r','movie','key','Movie',1);
            INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('f','r','/media/movie.mp4','g',10,'1','f','{}',1);
            INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('p','u','s','m','f','g','','playing','direct','{}',100,1,1);
            INSERT INTO compat_playbacks VALUES ('p',7);
            INSERT INTO compat_audio_playbacks VALUES ('s','m','p');")?;
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../migrations/009.sql"))?;
        tx.commit()?;
        db.execute_batch("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES ('abcdefghijk','g','ready','{}',1,1);
            INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id) VALUES ('yp','u','s','g','public','paused','direct','{}',100,1,1,'abcdefghijk');")?;
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../migrations/010.sql"))?;
        tx.execute_batch(include_str!("../migrations/011.sql"))?;
        tx.execute_batch(include_str!("../migrations/012.sql"))?;
        tx.execute_batch(include_str!("../migrations/013.sql"))?;
        tx.commit()?;
        assert_eq!(
            db.query_row(
                "SELECT streaming FROM playback_sessions WHERE id='yp'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT sequence FROM compat_playbacks WHERE playback_id='p'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            7
        );
        assert_eq!(
            db.query_row("SELECT playback_id FROM compat_audio_playbacks", [], |r| {
                r.get::<_, String>(0)
            })?,
            "p"
        );
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        db.execute("DELETE FROM sessions WHERE id='s'", [])?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM compat_playbacks", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    }
    #[tokio::test]
    async fn migration_and_online_backup_are_restart_safe() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("main.db"))?;
        db.call(|c| {
            c.execute("INSERT INTO settings VALUES ('timezone', '\"UTC\"')", [])?;
            Ok(())
        })
        .await?;
        db.backup(temp.path().join("snapshot.db")).await?;
        let copy = Database::open(temp.path().join("snapshot.db"))?;
        assert_eq!(
            copy.call(|c| Ok(c.query_row(
                "SELECT value FROM settings WHERE key='timezone'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await?,
            "\"UTC\""
        );
        Database::open(temp.path().join("main.db"))?;
        Ok(())
    }
}
