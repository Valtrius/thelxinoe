//! Database operations for segments.analysis.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn idle(db: &Database) -> anyhow::Result<bool> {
    db.read("segments.analysis.idle", |db|Ok(!db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running' OR (state='queued' AND available_at<=?1))",[now()],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn run_write_segment_analysis(db: &Database) -> anyhow::Result<()> {
    db.write("segments.analysis.run_write_segment_analysis", |db| {
        db.execute(
            "UPDATE segment_analysis SET state='queued' WHERE state='running'",
            [],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn claim_analysis(
    db: &Database,
) -> anyhow::Result<Option<(String, String, String)>> {
    db.write("segments.analysis.claim_analysis", |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT OR IGNORE INTO segment_analysis(media_id,file_id,generation,state,requested_at) SELECT m.id,f.id,f.generation,'queued',m.created_at FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id WHERE m.kind='episode' AND f.present=1 AND NOT EXISTS(SELECT 1 FROM segment_analysis a WHERE a.media_id=m.id AND a.file_id=f.id AND a.generation=f.generation) AND NOT EXISTS(SELECT 1 FROM media_sources other WHERE other.file_id=f.id AND other.media_id<>m.id) ORDER BY m.created_at DESC LIMIT 1000",[])?;
        let work=tx.query_row("UPDATE segment_analysis SET state='running',error=NULL WHERE (media_id,file_id,generation)=(SELECT a.media_id,a.file_id,a.generation FROM segment_analysis a JOIN media_files f ON f.id=a.file_id AND f.generation=a.generation AND f.present=1 WHERE a.state='queued' ORDER BY a.requested_at DESC LIMIT 1) RETURNING media_id,file_id,generation",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        tx.commit()?;Ok(work)
    }).await
}

pub(super) async fn finish_analysis(
    interrupted: bool,
    message: Option<String>,
    media: String,
    file: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("segments.analysis.finish_analysis", move|db|{db.execute("UPDATE segment_analysis SET state=?1,completed_at=?2,error=?3 WHERE media_id=?4 AND file_id=?5 AND generation=?6",params![if interrupted{"queued"}else if message.is_some(){"failed"}else{"complete"},now(),message,media,file,generation])?;Ok(())}).await
}

pub(super) async fn analyze_read_media(
    mid: String,
    fid: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("segments.analysis.analyze_read_media", move |db| {
        let episode = db
            .query_row(
                "SELECT parent_id FROM media WHERE id=?1 AND kind='episode'",
                [mid],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let shared = db.query_row(
            "SELECT COUNT(*) FROM media_sources WHERE file_id=?1",
            [fid],
            |r| r.get::<_, i64>(0),
        )? > 1;
        Ok(if shared { None } else { episode })
    })
    .await
}

pub(super) async fn analyze_write_segment_fingerprints(
    season: &str,
    kind: &'static str,
    offset: f64,
    saved: Source,
    serialized: String,
    db: &Database,
) -> anyhow::Result<Vec<(String, String, String, f64, Vec<u32>)>> {
    let season = season.to_owned();

    db.write("segments.analysis.analyze_write_segment_fingerprints", {let season=season.clone();move|db|{
        db.execute("INSERT INTO segment_fingerprints VALUES (?1,?2,?3,?4,?5) ON CONFLICT(file_id,generation,window) DO UPDATE SET hashes=excluded.hashes,offset=excluded.offset",params![saved.id,saved.generation,kind,offset,serialized])?;
        Ok(db.prepare("SELECT m.id,f.id,f.generation,p.offset,p.hashes FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id AND f.present=1 JOIN segment_fingerprints p ON p.file_id=f.id AND p.generation=f.generation WHERE m.parent_id=?1 AND m.id<>?2 AND f.id<>?3 AND p.window=?4 ORDER BY m.created_at DESC LIMIT 8")?.query_map(params![season,saved.media_id,saved.id,kind],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,f64>(3)?,serde_json::from_str::<Vec<u32>>(&r.get::<_,String>(4)?).unwrap_or_default())))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }}).await
}

pub(super) async fn store(
    src: Source,
    db: &Database,
    segments: Vec<Segment>,
    origin: &'static str,
    kind: Option<&'static str>,
) -> anyhow::Result<()> {
    db.write("segments.analysis.store", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT present AND generation=?2 FROM media_files WHERE id=?1",params![src.id,src.generation],|r|r.get::<_,bool>(0))?;
        anyhow::ensure!(current,"Media generation changed during analysis");
        tx.execute("DELETE FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 AND source=?4 AND (?5 IS NULL OR kind=?5)",params![src.media_id,src.id,src.generation,origin,kind])?;
        for segment in segments {insert(&tx,&src,&segment,origin)?;}
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn external(
    media: String,
    db: &Database,
) -> anyhow::Result<Option<(String, Option<i64>, i64, i64)>> {
    db.read("segments.analysis.external", move|db|{
        Ok(db.query_row("SELECT b.external_id,NULLIF(json_extract(show.metadata,'$.tmdb_id'),0),e.season_number,e.episode_number FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id AND b.refreshed_at=e.refreshed_at JOIN media show ON show.id=b.media_id JOIN manager_services s ON s.id=b.service_id AND s.kind='sonarr' AND s.enabled=1 AND s.generation=b.service_generation WHERE m.media_id=?1 AND m.state='confirmed' AND b.external_id GLOB '[0-9]*' AND (SELECT COUNT(*) FROM manager_episode_mappings WHERE media_id=?1)=1",[media],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?)
    }).await
}
