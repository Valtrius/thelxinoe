//! Database operations for trickplay.
use thelxinoe_database::Database;

/// Present movie and episode files with a video stream, as a media item, the file and its
/// version, the most recently added or changed first.
pub(super) async fn candidates(db: &Database) -> anyhow::Result<Vec<(String, String, String)>> {
    db.read("trickplay.candidates", |db| {
        Ok(db.prepare("SELECT MIN(s.media_id),f.id,f.generation FROM media_files f JOIN media_sources s ON s.file_id=f.id JOIN media m ON m.id=s.media_id WHERE f.present=1 AND m.kind IN ('movie','episode') AND EXISTS(SELECT 1 FROM json_each(f.probe,'$.streams') st WHERE json_extract(st.value,'$.codec_type')='video' AND COALESCE(json_extract(st.value,'$.disposition.attached_pic'),0)!=1) GROUP BY f.id ORDER BY f.scanned_at DESC,f.id")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// Every present library file version.
pub(super) async fn current(db: &Database) -> anyhow::Result<Vec<(String, String)>> {
    db.read("trickplay.current", |db| {
        Ok(db
            .prepare("SELECT id,generation FROM media_files WHERE present=1")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// The present files of these media items, as the item, the file and its version.
pub(super) async fn files(
    db: &Database,
    media: Vec<String>,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let media = serde_json::to_string(&media)?;
    db.read("trickplay.files", move |db| {
        Ok(db.prepare("SELECT s.media_id,f.id,f.generation FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE f.present=1 AND s.media_id IN (SELECT value FROM json_each(?1))")?.query_map([media], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}
