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
