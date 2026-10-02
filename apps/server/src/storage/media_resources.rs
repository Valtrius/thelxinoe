use thelxinoe_database::Database;

pub(crate) async fn for_media(db: &Database, media: &str) -> anyhow::Result<Vec<String>> {
    let media = media.to_owned();
    db.read("media_resources.media", move |db| {
        Ok(db.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT DISTINCT 'file:'||file_id FROM media_sources WHERE media_id IN (SELECT id FROM tree)")?.query_map([media], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}
pub(crate) async fn for_playback(
    db: &Database,
    session: &str,
    principal: &thelxinoe_core::Principal,
) -> anyhow::Result<Vec<String>> {
    let session = session.to_owned();
    let principal = principal.clone();
    db.read("media_resources.playback", move |db| {
        Ok(db.prepare("SELECT 'file:'||file_id FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3 AND file_id IS NOT NULL")?.query_map(rusqlite::params![session,principal.user.id,principal.session_id], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}
pub(crate) async fn for_root(db: &Database, root: &str) -> anyhow::Result<Vec<String>> {
    let root = root.to_owned();
    db.read("media_resources.root", move |db| {
        Ok(db
            .prepare("SELECT 'file:'||id FROM media_files WHERE root_id=?1")?
            .query_map([root], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}
