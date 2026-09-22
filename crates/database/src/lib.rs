use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const SCHEMA_VERSION: u32 = 38;

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
            include_str!("../migrations/028.sql"),
            include_str!("../migrations/029.sql"),
            include_str!("../migrations/030.sql"),
            include_str!("../migrations/031.sql"),
            include_str!("../migrations/032.sql"),
            include_str!("../migrations/033.sql"),
            include_str!("../migrations/034.sql"),
            include_str!("../migrations/035.sql"),
            include_str!("../migrations/036.sql"),
            include_str!("../migrations/037.sql"),
            include_str!("../migrations/038.sql"),
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
    fn statistics_migration_keeps_legacy_totals_and_marks_their_time_as_estimated() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        for version in 1..=33 {
            let path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("migrations/{version:03}.sql"));
            db.execute_batch(&std::fs::read_to_string(path)?)?;
        }
        db.execute_batch("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('alice','Alice','unused','user',1);
            INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Films','movies','/media');
            INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('film','root','movie','film','Film',1);
            INSERT INTO playback_history(playback_id,user_id,media_id,edition,device_name,started_at,updated_at,position,duration,played_seconds,state) VALUES ('old','alice','film','','Web',100,300,180,200,150,'stopped');
            INSERT INTO youtube_videos(user_id,video_id,title,channel_title) VALUES ('alice','video','Video','Channel');
            INSERT INTO youtube_state(user_id,video_id,watched,added_at,updated_at) VALUES ('alice','video',1,1,1);
            INSERT INTO youtube_history VALUES ('yt','alice','video','Video','Web',300,500,100,100,90,'stopped');
            INSERT INTO live_history VALUES ('live','alice','twitch:42','Old live stream','Web',500,600,100,70,'stopped');")?;
        db.execute_batch(include_str!("../migrations/034.sql"))?;
        let totals: (f64, f64, i64) = db.query_row(
            "SELECT SUM(active_seconds),SUM(estimated_seconds),COUNT(*) FROM playback_activity",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        assert_eq!(totals, (310.0, 310.0, 3));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM playback_statistics WHERE completed=1",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT channel_name FROM playback_statistics WHERE platform='twitch'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "Old live stream"
        );
        db.execute("DELETE FROM media WHERE id='film'", [])?;
        assert_eq!(
            db.query_row(
                "SELECT SUM(active_seconds) FROM playback_activity",
                [],
                |r| r.get::<_, f64>(0)
            )?,
            310.0
        );
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        db.execute("DELETE FROM users WHERE id='alice'", [])?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM playback_activity", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    }
    #[test]
    fn history_migration_unifies_sources_and_moves_activity_clocks() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        for version in 1..=36 {
            let path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("migrations/{version:03}.sql"));
            db.execute_batch(&std::fs::read_to_string(path)?)?;
        }
        db.execute_batch(
            "INSERT INTO users(id,username,password_hash,role,timezone,created_at) VALUES ('alice','Alice','unused','user','UTC',1);
             INSERT INTO sessions VALUES ('session','alice','hash','web','Browser',1,9999999999,1);
             INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Films','movies','/media');
             INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('film','root','movie','film','Film',1);
             INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('file','root','/media/film.mkv','g',1,'1','hash','{}',1);
             INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('clock','alice','session','film','file','g','','playing','direct','{}',100,1,1);
             INSERT INTO playback_activity_clocks VALUES ('clock',12345,67.5);
             INSERT INTO playback_history(playback_id,user_id,media_id,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES ('local','alice','film','','Browser',100,200,200,100,100,90,'stopped');
             INSERT INTO youtube_videos(user_id,video_id,title,channel_title,is_short,broadcast) VALUES ('alice','video','Video','Channel',1,'none');
             INSERT INTO youtube_history VALUES ('shared','alice','video','Video','Browser',300,400,30,60,20,'stopped');
             INSERT INTO live_history VALUES ('shared','alice','twitch:42','Old live stream','Browser',500,600,20,20,'stopped');",
        )?;

        db.execute_batch(include_str!("../migrations/037.sql"))?;

        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM playback_history", [], |r| r
                .get::<_, i64>(0))?,
            3
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM playback_history WHERE playback_id='shared'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT platform||':'||content_type FROM playback_history WHERE media_id='youtube:video'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "youtube:short"
        );
        assert_eq!(
            db.query_row(
                "SELECT reported_at_ms FROM playback_sessions WHERE id='clock'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            12345
        );
        assert_eq!(
            db.query_row(
                "SELECT client_active_seconds FROM playback_sessions WHERE id='clock'",
                [],
                |r| r.get::<_, f64>(0)
            )?,
            67.5
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('youtube_history','live_history','playback_activity_clocks')",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        db.execute("DELETE FROM media WHERE id='film'", [])?;
        assert_eq!(
            db.query_row(
                "SELECT media_title FROM playback_history WHERE playback_id='local'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "Film"
        );
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        Ok(())
    }
    #[test]
    fn timezone_migration_preserves_personal_choices_and_resolves_server_defaults() -> Result<()> {
        let db = Connection::open_in_memory()?;
        for version in 1..=29 {
            let path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("migrations/{version:03}.sql"));
            db.execute_batch(&std::fs::read_to_string(path)?)?;
        }
        db.execute_batch(
            "INSERT INTO users VALUES ('default','default','unused','user','UTC',1);
             INSERT INTO users VALUES ('personal','personal','unused','user','Asia/Tokyo',1);
             INSERT INTO settings VALUES ('timezone','Europe/Paris');",
        )?;
        db.execute_batch(include_str!("../migrations/030.sql"))?;
        let profile = |user: &str| {
            db.query_row(
                "SELECT timezone,timezone_override FROM user_profiles WHERE id=?1",
                [user],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
        };
        assert_eq!(profile("default")?, ("Europe/Paris".into(), None));
        assert_eq!(
            profile("personal")?,
            ("Asia/Tokyo".into(), Some("Asia/Tokyo".into()))
        );
        db.execute(
            "UPDATE settings SET value='America/New_York' WHERE key='timezone'",
            [],
        )?;
        assert_eq!(profile("default")?, ("America/New_York".into(), None));
        assert_eq!(
            profile("personal")?,
            ("Asia/Tokyo".into(), Some("Asia/Tokyo".into()))
        );
        db.execute(
            "INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('new','new','unused','user',1)",
            [],
        )?;
        assert_eq!(profile("new")?, ("America/New_York".into(), None));
        Ok(())
    }
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

    #[tokio::test]
    async fn integration_kinds_are_unique() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("main.db"))?;
        db.call(|c| {
            c.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('manager-1','Radarr','radarr','container-1',7878,'g1',X'00','/media','1',1)",[])?;
            assert!(c.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('manager-2','Other Radarr','radarr','container-2',7878,'g2',X'00','/media','1',1)",[]).is_err());
            c.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('manager-2','Sonarr','sonarr','container-2',8989,'g2',X'00','/media','1',1)",[])?;

            c.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('support-1','NZBGet','nzbget','container-3',6789,'g1',X'00','/media','1',1)",[])?;
            assert!(c.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('support-2','Other NZBGet','nzbget','container-4',6789,'g2',X'00','/media','1',1)",[]).is_err());
            c.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('support-2','Prowlarr','prowlarr','container-4',9696,'g2',X'00','','1',1)",[])?;
            Ok(())
        }).await
    }

    #[test]
    fn singleton_migration_reconciles_legacy_duplicate_integrations() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        for version in 1..=35 {
            let path =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("migrations/{version:03}.sql"));
            db.execute_batch(&std::fs::read_to_string(path)?)?;
        }
        db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('admin','admin','unused','admin',1)",[])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('managed-radarr','Managed','radarr','container-1',7878,'g1',X'00','/media','1',10)",[])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('newer-radarr','Newer','radarr','container-2',7878,'g2',X'00','/media','2',20)",[])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('request','admin','newer-radarr','g2','1','Movie','pending',1,1)",[])?;
        db.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,native_url,version,checked_at) VALUES ('older-nzbget','Older','nzbget','container-3',6789,'g1',X'00','/media','','1',10)",[])?;
        db.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,native_url,version,checked_at) VALUES ('newer-nzbget','Newer','nzbget','container-4',6789,'g2',X'00','/media','','2',20)",[])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,created_at,updated_at) VALUES ('provision','radarr','admin',17878,X'00','complete','container-1','managed-radarr',1,1)",[])?;

        db.execute_batch(include_str!("../migrations/036.sql"))?;

        assert_eq!(
            db.query_row(
                "SELECT id FROM manager_services WHERE kind='radarr'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "managed-radarr"
        );
        assert_eq!(
            db.query_row(
                "SELECT id FROM support_services WHERE kind='nzbget'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "newer-nzbget"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM acquisition_requests", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert!(db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('duplicate','Duplicate','radarr','container-5',7878,'g3',X'00','/media','3',30)",[]).is_err());
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
        Ok(())
    }
}
