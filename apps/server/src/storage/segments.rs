//! Database operations for segments.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn resolved(
    media: String,
    file: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<Vec<Segment>> {
    db.read("segments.resolved", move|db|{
        let manual=db.query_row("SELECT EXISTS(SELECT 1 FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3)",params![media,file,generation],|r|r.get::<_,bool>(0))?;
        let rows=db.prepare("SELECT id,kind,start,end,source,confidence FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 ORDER BY CASE source WHEN 'manual' THEN 0 WHEN 'theintrodb' THEN 1 ELSE 2 END,start")?.query_map(params![media,file,generation],|r|Ok(Segment{id:r.get(0)?,kind:r.get(1)?,start:r.get(2)?,end:r.get(3)?,source:r.get(4)?,confidence:r.get(5)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut selected=Vec::<Segment>::new();
        for s in rows {
            if manual && s.source!="manual" {continue;}
            if selected.iter().any(|other|other.source!=s.source && other.kind==s.kind){continue;}
            selected.push(s);
        }
        selected.sort_by(|a,b|a.start.total_cmp(&b.start));
        Ok(selected)
    }).await
}

pub(super) async fn for_jellyfin(
    user: String,
    auth: String,
    mid: String,
    db: &Database,
) -> anyhow::Result<(Option<String>, i64)> {
    db.read("segments.for_jellyfin", move|db|{
        let file=db.query_row("SELECT s.file_id FROM playback_sessions s JOIN media_files f ON f.id=s.file_id AND f.generation=s.generation AND f.present=1 WHERE s.media_id=?1 AND s.user_id=?2 AND s.auth_session_id=?3 AND s.state IN ('ready','playing','paused') ORDER BY s.created_at DESC,s.rowid DESC LIMIT 1",params![mid,user,auth],|r|r.get::<_,String>(0)).optional()?;
        let count=db.query_row("SELECT COUNT(*) FROM media_sources s JOIN media_files f ON f.id=s.file_id AND f.present=1 WHERE s.media_id=?1",[mid],|r|r.get::<_,i64>(0))?;
        Ok((file,count))
    }).await
}

pub(super) async fn preferences_for(user: String, db: &Database) -> anyhow::Result<Option<String>> {
    db.read("segments.preferences_for", move |db| {
        Ok(db
            .query_row(
                "SELECT value FROM segment_preferences WHERE user_id=?1",
                [user],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn save_preferences(
    db: &Database,
    input: Value,
    p: Principal,
) -> anyhow::Result<()> {
    db.write("segments.save_preferences", move|db|{db.execute("INSERT INTO segment_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![p.user.id,input.to_string()])?;Ok(())}).await
}

pub(super) async fn save(
    db: &Database,
    media: String,
    input: Edit,
    p: Principal,
    src: thelxinoe_playback::Source,
) -> anyhow::Result<()> {
    db.write("segments.save", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 AND source='manual'",params![media,src.id,src.generation])?;
        tx.execute("DELETE FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3",params![media,src.id,src.generation])?;
        if !input.reset {
            tx.execute("INSERT INTO segment_overrides VALUES (?1,?2,?3)",params![media,src.id,src.generation])?;
            for s in input.items {insert(&tx,&src,&s,"manual")?;}
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'segments.edit',?2,?3)",params![p.user.id,media,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn reanalyze_read_media(db: &Database, mid: String) -> anyhow::Result<bool> {
    db.read("segments.reanalyze_read_media", move |db| {
        Ok(
            db.query_row("SELECT kind='episode' FROM media WHERE id=?1", [mid], |r| {
                r.get::<_, bool>(0)
            })?,
        )
    })
    .await
}

pub(super) async fn reanalyze_write_segment_analysis(
    db: &Database,
    media: String,
    src: thelxinoe_playback::Source,
) -> anyhow::Result<()> {
    db.write("segments.reanalyze_write_segment_analysis", move|db|{db.execute("INSERT INTO segment_analysis(media_id,file_id,generation,state,requested_at) VALUES (?1,?2,?3,'queued',?4) ON CONFLICT(media_id,file_id,generation) DO UPDATE SET state=CASE WHEN state='running' THEN state ELSE 'queued' END,requested_at=excluded.requested_at,error=NULL",params![media,src.id,src.generation,now()])?;Ok(())}).await
}

pub(super) async fn config(db: &Database) -> anyhow::Result<Config> {
    db.read("segments.config", |db| {
        Ok(serde_json::from_str(&db.query_row(
            "SELECT value FROM settings WHERE key='segments.config'",
            [],
            |r| r.get::<_, String>(0),
        )?)?)
    })
    .await
}

pub(super) async fn configure(db: &Database, input: Config) -> anyhow::Result<()> {
    db.write("segments.configure", move |db| {
        db.execute(
            "UPDATE settings SET value=?1 WHERE key='segments.config'",
            [serde_json::to_string(&input)?],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn status(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("segments.status", |db|Ok(db.prepare("SELECT a.media_id,m.title,a.state,a.error,a.completed_at FROM segment_analysis a JOIN media m ON m.id=a.media_id ORDER BY a.requested_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"media_id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?,"completed_at":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) fn insert(
    db: &rusqlite::Connection,
    src: &thelxinoe_playback::Source,
    s: &Segment,
    origin: &'static str,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO media_segments VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id(),
            src.media_id,
            src.id,
            src.generation,
            s.kind,
            s.start,
            s.end,
            origin,
            if origin == "manual" {
                1.0
            } else {
                s.confidence
            },
            now()
        ],
    )?;
    Ok(())
}
