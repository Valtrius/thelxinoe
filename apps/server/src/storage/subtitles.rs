//! Database operations for subtitles.
use thelxinoe_database::Database;

/// Present library files with an embedded text subtitle, as a media item, the file and its
/// version, the most recently added or changed first.
pub(super) async fn candidates(db: &Database) -> anyhow::Result<Vec<(String, String, String)>> {
    let codecs = serde_json::to_string(&thelxinoe_playback::TEXT_SUBTITLES)?;
    db.read("subtitles.candidates", move |db| {
        Ok(db.prepare("SELECT MIN(s.media_id),f.id,f.generation FROM media_files f JOIN media_sources s ON s.file_id=f.id WHERE f.present=1 AND EXISTS(SELECT 1 FROM json_each(f.probe,'$.streams') st WHERE json_extract(st.value,'$.codec_type')='subtitle' AND json_extract(st.value,'$.codec_name') IN (SELECT value FROM json_each(?1))) GROUP BY f.id ORDER BY f.scanned_at DESC,f.id")?.query_map([codecs], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// Every file version playback can read: present library files and ready downloads.
pub(super) async fn current(db: &Database) -> anyhow::Result<Vec<(String, String)>> {
    db.read("subtitles.current", |db| {
        Ok(db.prepare("SELECT id,generation FROM media_files WHERE present=1 UNION ALL SELECT video_id,generation FROM youtube_downloads WHERE state='ready'")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}
