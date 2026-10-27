#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;
use anyhow::Result;
use rusqlite::params;
use thelxinoe_catalog::{Root, approved_path, scan};
use thelxinoe_database::Database;

fn wav(seconds: u32) -> Vec<u8> {
    let bytes = seconds * 8000 * 2;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + bytes).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(bytes.to_le_bytes());
    out.resize(44 + bytes as usize, 0);
    out
}
#[tokio::test]
async fn scan_replacement_move_removal_and_failed_scan_preserve_logical_state() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let media = temp.path().join("music");
    std::fs::create_dir(&media)?;
    let db = Database::open(temp.path().join("db"))?;
    let root = Root {
        id: "root".into(),
        name: "Music".into(),
        kind: "music".into(),
        path: media.to_string_lossy().to_string(),
        last_scan: None,
        scan_error: None,
    };
    let r = root.clone();
    db.write("test.fixture", move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        Ok(())
    })
    .await?;
    std::fs::write(media.join("Track.wav"), wav(1))?;
    let tool_root = tempfile::tempdir()?;
    let tools = thelxinoe_tools::Runtime::new(tool_root.path().into());
    tool_runtime::install(&tools, tool_root.path());
    assert_eq!(scan(&db, root.clone(), tools.media()?).await?, 1);
    let first=db.write("test.fixture", |c|Ok(c.query_row("SELECT m.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await?;
    std::fs::write(media.join("Track.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let second=db.write("test.fixture", |c|Ok(c.query_row("SELECT m.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await?;
    assert_eq!(first.0, second.0);
    assert_ne!(first.1, second.1);
    std::fs::rename(media.join("Track.wav"), media.join("Renamed.wav"))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let moved=db.write("test.fixture", |c|Ok(c.query_row("SELECT m.id FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|r.get::<_,String>(0))?)).await?;
    assert_eq!(moved, first.0);
    scan(&db, root.clone(), tools.media()?).await?;
    let stable=db.write("test.fixture", |c|Ok(c.query_row("SELECT m.id FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|r.get::<_,String>(0))?)).await?;
    assert_eq!(
        stable, first.0,
        "Repeated scans of a renamed file must retain the logical identity"
    );
    std::fs::write(media.join("Distinct copy.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT COUNT(*) FROM media WHERE kind='track'",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        2,
        "An identical copy is not a move while its source still exists"
    );
    std::fs::remove_file(media.join("Distinct copy.wav"))?;
    scan(&db, root.clone(), tools.media()?).await?;
    // One unreadable file is reported and retried without blocking the rest of the library.
    std::fs::write(media.join("Broken.wav"), b"incomplete download")?;
    assert_eq!(scan(&db, root.clone(), tools.media()?).await?, 1);
    let warning = db
        .write("test.fixture", |c| {
            Ok(
                c.query_row("SELECT scan_error FROM library_roots", [], |r| {
                    r.get::<_, Option<String>>(0)
                })?,
            )
        })
        .await?
        .unwrap_or_default();
    assert!(
        warning.starts_with("1 file couldn't be read") && warning.contains("Broken.wav"),
        "{warning}"
    );
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT count(*) FROM media_files WHERE present=1",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        1
    );
    // A known file that becomes unreadable keeps its entry and identity until it recovers.
    let known = db
        .write("test.fixture", |c| {
            Ok(c.query_row(
                "SELECT generation FROM media_files WHERE path LIKE '%Renamed.wav' AND present=1",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await?;
    let original = std::fs::read(media.join("Renamed.wav"))?;
    std::fs::write(
        media.join("Renamed.wav"),
        b"truncated while a copy is in progress",
    )?;
    scan(&db, root.clone(), tools.media()?).await?;
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT generation FROM media_files WHERE path LIKE '%Renamed.wav' AND present=1",
            [],
            |r| r.get::<_, String>(0)
        )?))
        .await?,
        known
    );
    std::fs::write(media.join("Renamed.wav"), original)?;
    std::fs::remove_file(media.join("Broken.wav"))?;
    scan(&db, root.clone(), tools.media()?).await?;
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT scan_error FROM library_roots",
            [],
            |r| r.get::<_, Option<String>>(0)
        )?))
        .await?,
        None
    );
    std::fs::write(media.join("Broken.wav"), b"incomplete download")?;
    std::fs::remove_file(media.join("Broken.wav"))?;
    std::fs::remove_file(media.join("Renamed.wav"))?;
    scan(&db, root, tools.media()?).await?;
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT count(*) FROM media_files WHERE present=1",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        0
    );
    assert_eq!(
        db.write("test.fixture", |c| Ok(c.query_row(
            "SELECT count(*) FROM media WHERE kind='track'",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        2
    );
    assert!(approved_path(temp.path(), &media).is_err());
    Ok(())
}

// CI runs as root, which ignores permission bits, so the folder is made unreadable by
// nesting it past Linux's path length limit instead.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn an_unreadable_folder_is_skipped_and_keeps_its_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let media = temp.path().join("music");
    std::fs::create_dir(&media)?;
    let media = media.canonicalize()?;
    let db = Database::open(temp.path().join("db"))?;
    let root = Root {
        id: "root".into(),
        name: "Music".into(),
        kind: "music".into(),
        path: media.to_string_lossy().to_string(),
        last_scan: None,
        scan_error: None,
    };
    std::fs::write(media.join("Track.wav"), wav(1))?;
    // Each half stays under the limit for the calls that create it; joined, they exceed it.
    let segment = "d".repeat(200);
    let staging = temp.path().join("staging");
    let (mut outer, mut inner) = (media.join("Hidden"), staging.clone());
    for _ in 0..12 {
        outer.push(&segment);
        inner.push(&segment);
    }
    std::fs::create_dir_all(&outer)?;
    std::fs::create_dir_all(&inner)?;
    std::fs::rename(staging.join(&segment), outer.join(&segment))?;
    let hidden = outer.join(inner.strip_prefix(&staging)?).join("Song.wav");
    assert!(hidden.as_os_str().len() > 4096);
    let (r, kept) = (root.clone(), hidden.to_string_lossy().to_string());
    let gone = media.join("Gone.wav").to_string_lossy().to_string();
    db.write("test.fixture", move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        for (id, path) in [("kept", kept), ("gone", gone)] {
            c.execute(
                "INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,present,scanned_at) VALUES (?1,'root',?2,?1,1,'1',?1,'{}',1,1)",
                params![id, path],
            )?;
        }
        Ok(())
    })
    .await?;
    let tool_root = tempfile::tempdir()?;
    let tools = thelxinoe_tools::Runtime::new(tool_root.path().into());
    tool_runtime::install(&tools, tool_root.path());
    assert_eq!(scan(&db, root, tools.media()?).await?, 1);
    let (warning, present) = db
        .write("test.fixture", |c| {
            Ok((
                c.query_row("SELECT scan_error FROM library_roots", [], |r| {
                    r.get::<_, Option<String>>(0)
                })?,
                c.prepare("SELECT id FROM media_files WHERE present=1 ORDER BY id")?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            ))
        })
        .await?;
    let warning = warning.unwrap_or_default();
    assert!(
        warning.starts_with("1 folder couldn't be read") && warning.contains("Hidden"),
        "{warning}"
    );
    // The skipped folder keeps its entry; files gone from readable folders are removed.
    assert_eq!(present.len(), 2, "{present:?}");
    assert!(present.contains(&"kept".to_owned()));
    assert!(!present.contains(&"gone".to_owned()));
    Ok(())
}

#[test]
fn fingerprints_cover_small_files_and_sample_large_ones() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let fingerprint = |bytes: &[u8]| -> Result<String> {
        let path = temp.path().join("file");
        std::fs::write(&path, bytes)?;
        Ok(thelxinoe_catalog::fingerprint(&mut std::fs::File::open(
            &path,
        )?)?)
    };
    let small = vec![1u8; 1000];
    let mut edited = small.clone();
    edited[500] = 2;
    assert_ne!(fingerprint(&small)?, fingerprint(&edited)?);
    let large = vec![7u8; 8 << 20];
    let base = fingerprint(&large)?;
    // The start, middle and end samples and the size identify large files.
    for offset in [0, 4 << 20, (8 << 20) - 1] {
        let mut edited = large.clone();
        edited[offset] ^= 1;
        assert_ne!(fingerprint(&edited)?, base, "{offset}");
    }
    let mut longer = large.clone();
    longer.push(7);
    assert_ne!(fingerprint(&longer)?, base);
    // Between samples, replacement is detected by size, modification time and handle.
    let mut between = large;
    between[2 << 20] ^= 1;
    assert_eq!(fingerprint(&between)?, base);
    Ok(())
}

// Probing holds no lease, so a file can be deleted between probing and the commit; the
// commit then writes it as gone instead of bringing it back.
#[tokio::test]
async fn a_file_deleted_after_probing_is_not_committed_as_present() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let media = temp.path().join("music");
    std::fs::create_dir(&media)?;
    let db = Database::open(temp.path().join("db"))?;
    let root = Root {
        id: "root".into(),
        name: "Music".into(),
        kind: "music".into(),
        path: media.to_string_lossy().to_string(),
        last_scan: None,
        scan_error: None,
    };
    let r = root.clone();
    db.write("test.fixture", move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        Ok(())
    })
    .await?;
    for name in ["Kept.wav", "Deleted.wav"] {
        std::fs::write(media.join(name), wav(1))?;
    }
    let tool_root = tempfile::tempdir()?;
    let tools = thelxinoe_tools::Runtime::new(tool_root.path().into());
    tool_runtime::install(&tools, tool_root.path());
    let scanned =
        thelxinoe_catalog::index_with_progress(&db, root, tools.media()?, |_, _| async { Ok(()) })
            .await?;
    std::fs::remove_file(media.join("Deleted.wav"))?;
    assert_eq!(scanned.commit(&db).await?, 1);
    let present: Vec<String> = db
        .read("test.present", |c| {
            Ok(c.prepare("SELECT path FROM media_files WHERE present=1")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    assert_eq!(present.len(), 1);
    assert!(present[0].ends_with("Kept.wav"));
    Ok(())
}

// Rescanning an unchanged library rewrites no file rows; a changed file is rewritten.
#[tokio::test]
async fn an_unchanged_rescan_writes_no_file_rows() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let media = temp.path().join("music");
    std::fs::create_dir(&media)?;
    let db = Database::open(temp.path().join("db"))?;
    let root = Root {
        id: "root".into(),
        name: "Music".into(),
        kind: "music".into(),
        path: media.to_string_lossy().to_string(),
        last_scan: None,
        scan_error: None,
    };
    let r = root.clone();
    db.write("test.fixture", move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        Ok(())
    })
    .await?;
    for name in ["One.wav", "Two.wav"] {
        std::fs::write(media.join(name), wav(1))?;
    }
    let tool_root = tempfile::tempdir()?;
    let tools = thelxinoe_tools::Runtime::new(tool_root.path().into());
    tool_runtime::install(&tools, tool_root.path());
    scan(&db, root.clone(), tools.media()?).await?;
    // scanned_at moves only when a row is written.
    db.write("test.mark", |c| {
        c.execute("UPDATE media_files SET scanned_at=0", [])?;
        Ok(())
    })
    .await?;
    std::fs::write(media.join("Two.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let written = || {
        db.read("test.written", |c| {
            Ok(
                c.prepare("SELECT path,scanned_at>0 FROM media_files ORDER BY path")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<Vec<(String, bool)>>>()?,
            )
        })
    };
    let rows = written().await?;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].0.ends_with("One.wav") && !rows[0].1);
    assert!(rows[1].0.ends_with("Two.wav") && rows[1].1);
    // A file that goes missing records when, so maintenance can wait before pruning it.
    std::fs::remove_file(media.join("One.wav"))?;
    scan(&db, root, tools.media()?).await?;
    assert!(written().await?[0].1);
    Ok(())
}

/// A one-second WAV with RIFF INFO tags, which FFprobe reports as format tags.
fn tagged_wav(tags: &[(&[u8; 4], &str)]) -> Vec<u8> {
    let mut info = b"INFO".to_vec();
    for (key, value) in tags {
        let mut value = format!("{value}\0").into_bytes();
        let length = u32::try_from(value.len()).unwrap();
        value.resize(value.len().next_multiple_of(2), 0);
        info.extend(*key);
        info.extend(length.to_le_bytes());
        info.extend(value);
    }
    let plain = wav(1);
    let mut out = plain[..36].to_vec();
    // The tags go between the format and data chunks.
    out.extend(b"LIST");
    out.extend(u32::try_from(info.len()).unwrap().to_le_bytes());
    out.extend(info);
    out.extend(&plain[36..]);
    let size = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}

/// One empty library of `kind` with a user to hold watch state, and the runner's FFmpeg.
async fn library(
    temp: &std::path::Path,
    kind: &str,
) -> Result<(Database, Root, std::path::PathBuf, thelxinoe_tools::Runtime)> {
    let media = temp.join(kind);
    std::fs::create_dir(&media)?;
    let db = Database::open(temp.join("db"))?;
    let root = Root {
        id: "root".into(),
        name: kind.into(),
        kind: kind.into(),
        path: media.to_string_lossy().to_string(),
        last_scan: None,
        scan_error: None,
    };
    let r = root.clone();
    db.write("test.fixture", move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        c.execute(
            "INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('alice','alice','unused','user',1)",
            [],
        )?;
        Ok(())
    })
    .await?;
    let tools = thelxinoe_tools::Runtime::new(temp.join("tools"));
    tool_runtime::install(&tools, &temp.join("tools"));
    Ok((db, root, media, tools))
}

async fn count(db: &Database, sql: &'static str) -> Result<i64> {
    db.read("test.count", move |c| {
        Ok(c.query_row(sql, [], |r| r.get(0))?)
    })
    .await
}

// A library removed during its scan stops it: probing ends at the next progress check, and
// a scan probed before the removal writes nothing.
#[tokio::test]
async fn a_scan_of_a_removed_library_stops_without_writing() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (db, root, media, tools) = library(temp.path(), "music").await?;
    std::fs::write(media.join("Song.wav"), wav(1))?;
    let probed =
        thelxinoe_catalog::index_with_progress(&db, root.clone(), tools.media()?, |_, _| async {
            Ok(())
        })
        .await?;
    db.write("test.remove", |c| {
        c.execute("DELETE FROM library_roots", [])?;
        Ok(())
    })
    .await?;
    let committed = probed.commit(&db).await.map_err(|error| error.to_string());
    assert_eq!(committed, Err("Library was removed".into()));
    let probing =
        thelxinoe_catalog::index_with_progress(&db, root, tools.media()?, |_, _| async { Ok(()) })
            .await;
    assert_eq!(
        probing.err().map(|error| error.to_string()).as_deref(),
        Some("Library was removed")
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM media_files").await?, 0);
    Ok(())
}

// The file keeps its logical entry, so watch state stays with what was watched, while its
// title and place follow the file: an episode renamed S01E02 to S01E03 becomes episode 3.
#[tokio::test]
async fn a_renamed_episode_keeps_its_entry_and_watch_state_at_its_new_number() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (db, root, media, tools) = library(temp.path(), "shows").await?;
    let season = media.join("Show/Season 1");
    std::fs::create_dir_all(&season)?;
    std::fs::write(season.join("Show S01E01.wav"), wav(1))?;
    std::fs::write(season.join("Show S01E02.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let episode = db
        .write("test.fixture", |c| {
            let episode: String = c.query_row(
                "SELECT id FROM media WHERE evidence_key='show:s1:e2'",
                [],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT INTO media_state(user_id,media_id,watched,favorite,updated_at) VALUES ('alice',?1,1,1,1)",
                [&episode],
            )?;
            // The show's matched metadata stays; what was derived from the old number goes.
            c.execute(
                "INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('sonarr','Sonarr','sonarr','sonarr',8989,'g',x'00','/media','4',1)",
                [],
            )?;
            c.execute(
                "INSERT INTO manager_episodes(service_id,service_generation,series_external_id,manager_episode_id,season_number,episode_number,refreshed_at,metadata) VALUES ('sonarr','g','series',2,1,2,1,'{}')",
                [],
            )?;
            c.execute(
                "INSERT INTO manager_episode_mappings VALUES (?1,'sonarr','g',2,'confirmed')",
                [&episode],
            )?;
            c.execute(
                "UPDATE media SET metadata='{\"title\":\"The second episode\"}' WHERE id=?1",
                [&episode],
            )?;
            c.execute(
                "INSERT INTO metadata_bindings(media_id,service_id,service_generation,external_id) SELECT id,'sonarr','g','series' FROM media WHERE kind='show'",
                [],
            )?;
            Ok(episode)
        })
        .await?;
    std::fs::rename(
        season.join("Show S01E02.wav"),
        season.join("Show S01E03.wav"),
    )?;
    scan(&db, root, tools.media()?).await?;
    let moved = db
        .read("test.moved", move |c| {
            Ok(c.query_row(
                "SELECT m.evidence_key,m.sort_number,m.title,m.metadata,s.watched AND s.favorite,(SELECT COUNT(*) FROM manager_episode_mappings WHERE media_id=m.id) FROM media m JOIN media_state s ON s.media_id=m.id AND s.user_id='alice' WHERE m.id=?1",
                [&episode],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, bool>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )?)
        })
        .await?;
    assert_eq!(
        moved,
        (
            "show:s1:e3".into(),
            3,
            "Show · S01E03".into(),
            "{}".into(),
            true,
            0
        )
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM media WHERE kind='episode'").await?,
        2,
        "no entry is left behind at the old number"
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM metadata_bindings").await?,
        1
    );
    Ok(())
}

// Files named before and after a naming change are one show, and a show an earlier rule split
// off is regrouped although its files are unchanged: episodes keep their entries and watch
// state, and the emptied show goes.
#[tokio::test]
async fn a_series_folder_regroups_its_unchanged_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (db, root, media, tools) = library(temp.path(), "shows").await?;
    let series = media.join("Show (2019) {imdb-tt1}");
    std::fs::create_dir_all(series.join("Season 1"))?;
    std::fs::create_dir_all(series.join("Season 01"))?;
    std::fs::write(series.join("Season 1/Show - S01E01.wav"), wav(1))?;
    std::fs::write(series.join("Season 01/Show (2019) - S01E02.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let shows = || {
        count(
            &db,
            "SELECT COUNT(*) FROM media WHERE kind IN ('show','season')",
        )
    };
    assert_eq!(shows().await?, 2);
    // The first episode as the file-name rule placed it, under a show of its own.
    let episode = db
        .write("test.fixture", |c| {
            let episode: String = c.query_row(
                "SELECT id FROM media WHERE evidence_key='show (2019):s1:e1'",
                [],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('old','root','show','show','Show',1)",
                [],
            )?;
            c.execute(
                "INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('old-s1','root','season','old','show:s1','Season 1',1,1)",
                [],
            )?;
            c.execute(
                "UPDATE media SET evidence_key='show:s1:e1',parent_id='old-s1' WHERE id=?1",
                [&episode],
            )?;
            c.execute(
                "INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice',?1,1,1)",
                [&episode],
            )?;
            Ok(episode)
        })
        .await?;
    scan(&db, root, tools.media()?).await?;
    let regrouped = db
        .read("test.regrouped", move |c| {
            Ok(c.query_row(
                "SELECT e.evidence_key,sh.title,u.watched FROM media e JOIN media s ON s.id=e.parent_id JOIN media sh ON sh.id=s.parent_id JOIN media_state u ON u.media_id=e.id AND u.user_id='alice' WHERE e.id=?1",
                [&episode],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)),
            )?)
        })
        .await?;
    assert_eq!(
        regrouped,
        ("show (2019):s1:e1".into(), "Show (2019)".into(), true)
    );
    assert_eq!(shows().await?, 2, "the split-off show and season go");
    Ok(())
}

// Retagging a track's album moves the same entry under the new album, and the album left
// without tracks goes.
#[tokio::test]
async fn a_retagged_track_moves_to_its_new_album_with_its_watch_state() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (db, root, media, tools) = library(temp.path(), "music").await?;
    let tagged = |album: &str| {
        tagged_wav(&[
            (b"IART", "Artist"),
            (b"IPRD", album),
            (b"INAM", "Song"),
            (b"ITRK", "1"),
        ])
    };
    std::fs::write(media.join("Song.wav"), tagged("First"))?;
    std::fs::write(media.join("Other.wav"), wav(2))?;
    scan(&db, root.clone(), tools.media()?).await?;
    let track = db
        .write("test.fixture", |c| {
            let track: String = c.query_row(
                "SELECT id FROM media WHERE kind='track' AND title='Song'",
                [],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT INTO media_state(user_id,media_id,watched,favorite,updated_at) VALUES ('alice',?1,1,1,1)",
                [&track],
            )?;
            Ok(track)
        })
        .await?;
    std::fs::write(media.join("Song.wav"), tagged("Second album"))?;
    scan(&db, root, tools.media()?).await?;
    let (album, watched) = db
        .read("test.moved", move |c| {
            Ok(c.query_row(
                "SELECT a.title,s.watched AND s.favorite FROM media m JOIN media a ON a.id=m.parent_id JOIN media_state s ON s.media_id=m.id AND s.user_id='alice' WHERE m.id=?1",
                [&track],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )?)
        })
        .await?;
    assert_eq!(album, "Second album");
    assert!(watched, "watch state stays with the retagged file");
    let albums: Vec<String> = db
        .read("test.albums", |c| {
            Ok(
                c.prepare("SELECT title FROM media WHERE kind='album' ORDER BY title")?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?,
            )
        })
        .await?;
    // The untagged track keeps its folder's album; the emptied "First" goes.
    assert_eq!(albums, ["Second album", "music"]);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM media WHERE kind IN ('track','artist')"
        )
        .await?,
        4
    );
    Ok(())
}
