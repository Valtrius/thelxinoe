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
    db.call(move |c| {
        c.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![r.id, r.name, r.kind, r.path],
        )?;
        Ok(())
    })
    .await?;
    std::fs::write(media.join("Track.wav"), wav(1))?;
    assert_eq!(scan(&db, root.clone()).await?, 1);
    let first=db.call(|c|Ok(c.query_row("SELECT m.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await?;
    std::fs::write(media.join("Track.wav"), wav(2))?;
    scan(&db, root.clone()).await?;
    let second=db.call(|c|Ok(c.query_row("SELECT m.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await?;
    assert_eq!(first.0, second.0);
    assert_ne!(first.1, second.1);
    std::fs::rename(media.join("Track.wav"), media.join("Renamed.wav"))?;
    scan(&db, root.clone()).await?;
    let moved=db.call(|c|Ok(c.query_row("SELECT m.id FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|r.get::<_,String>(0))?)).await?;
    assert_eq!(moved, first.0);
    scan(&db, root.clone()).await?;
    let stable=db.call(|c|Ok(c.query_row("SELECT m.id FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='track' AND f.present=1",[],|r|r.get::<_,String>(0))?)).await?;
    assert_eq!(
        stable, first.0,
        "Repeated scans of a renamed file must retain the logical identity"
    );
    std::fs::write(media.join("Distinct copy.wav"), wav(2))?;
    scan(&db, root.clone()).await?;
    assert_eq!(
        db.call(|c| Ok(
            c.query_row("SELECT COUNT(*) FROM media WHERE kind='track'", [], |r| r
                .get::<_, i64>(
                0
            ))?
        ))
        .await?,
        2,
        "An identical copy is not a move while its source still exists"
    );
    std::fs::remove_file(media.join("Distinct copy.wav"))?;
    scan(&db, root.clone()).await?;
    std::fs::write(media.join("Broken.wav"), b"incomplete download")?;
    assert!(scan(&db, root.clone()).await.is_err());
    assert_eq!(
        db.call(|c| Ok(c.query_row(
            "SELECT count(*) FROM media_files WHERE present=1",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        1
    );
    std::fs::remove_file(media.join("Broken.wav"))?;
    std::fs::remove_file(media.join("Renamed.wav"))?;
    scan(&db, root).await?;
    assert_eq!(
        db.call(|c| Ok(c.query_row(
            "SELECT count(*) FROM media_files WHERE present=1",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .await?,
        0
    );
    assert_eq!(
        db.call(|c| Ok(
            c.query_row("SELECT count(*) FROM media WHERE kind='track'", [], |r| r
                .get::<_, i64>(
                0
            ))?
        ))
        .await?,
        2
    );
    assert!(approved_path(temp.path(), &media).is_err());
    Ok(())
}
