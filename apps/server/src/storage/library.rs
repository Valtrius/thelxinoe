//! Database operations for library.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn add_root(
    db: &Database,
    input: NewRoot,
    p: thelxinoe_core::Principal,
    rid: String,
    path: String,
) -> anyhow::Result<bool> {
    db.write("library.add_root", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Vec<String> = tx
            .prepare("SELECT path FROM library_roots")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        if existing.iter().any(|other| {
            std::path::Path::new(&path).starts_with(other)
                || std::path::Path::new(other).starts_with(&path)
        }) {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",
            params![rid, input.name, input.kind, path],
        )?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'library.add',?2,?3)",
            params![p.user.id, rid, now()],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await
}

pub(super) async fn enqueue_scan(root_id: String, db: &Database) -> anyhow::Result<String> {
    db.write("library.enqueue_scan", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(queued)=tx.query_row("SELECT id FROM jobs WHERE kind='library.scan' AND state='queued' AND json_extract(payload,'$.root_id')=?1",[&root_id],|r|r.get::<_,String>(0)).optional()?{return Ok(queued);}
        let job_id=id();tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'library.scan',?2,?1,'queued',?3,?3)",params![job_id,json!({"root_id":root_id}).to_string(),now()])?;tx.commit()?;Ok(job_id)
    }).await
}

pub(super) async fn browse(db: &Database, input: Browse) -> anyhow::Result<Vec<Value>> {
    db.read("library.browse", move|db|{
        let mut query=db.prepare("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE (?1 IS NULL OR m.kind=?1) AND (?2 IS NULL OR m.parent_id=?2) AND (?3 IS NULL OR COALESCE(json_extract(m.overrides,'$.title'),json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) LIKE '%'||?3||'%') AND (?4 IS NULL OR json_extract(m.metadata,'$.belongs_to_collection.id')=?4) ORDER BY m.sort_number,m.title LIMIT 500")?;
        Ok(query.query_map(params![input.kind,input.parent,input.q,input.collection],media_row)?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await
}

pub(super) async fn decorate_cards(
    ids: String,
    db: &Database,
) -> std::prelude::v1::Result<
    std::collections::HashMap<String, (Option<i64>, Option<String>)>,
    anyhow::Error,
> {
    db.read("library.decorate_cards", move |db| {
        let mut query = db.prepare("SELECT m.id,m.year,CASE WHEN json_extract(m.metadata,'$.artwork_cached')=1 THEN m.id WHEN json_extract(p.metadata,'$.artwork_cached')=1 THEN p.id WHEN json_extract(g.metadata,'$.artwork_cached')=1 THEN g.id END FROM media m LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media g ON g.id=p.parent_id WHERE m.id IN (SELECT value FROM json_each(?1))")?;
        Ok(query.query_map([ids], |r| Ok((r.get::<_,String>(0)?, (r.get::<_,Option<i64>>(1)?,r.get::<_,Option<String>>(2)?))))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?)
    }).await
}

pub(super) async fn detail(db: &Database, media_id: String) -> anyhow::Result<Option<Value>> {
    db.read("library.detail", move|db|{
        let item=db.query_row("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE m.id=?1",[&media_id],media_row).optional()?;
        let files=db.prepare("SELECT f.id,f.edition,f.probe,f.present FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1")?.query_map([&media_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"edition":r.get::<_,String>(1)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or_default(),"present":r.get::<_,bool>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let trailers=db.prepare("SELECT f.id,f.probe FROM local_trailers t JOIN media_files f ON f.id=t.file_id WHERE t.media_id=?1 AND f.present=1")?.query_map([&media_id],|r|Ok(json!({"file_id":r.get::<_,String>(0)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default()})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(item.map(|mut item|{item["files"]=json!(files);item["local_trailers"]=json!(trailers);item}))
    }).await
}

pub(super) async fn overrides(
    db: &Database,
    media_id: String,
    value: Value,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<usize> {
    db.write("library.overrides", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let n=tx.execute("UPDATE media SET overrides=?1 WHERE id=?2",params![value.to_string(),media_id])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.override',?2,?3)",params![p.user.id,media_id,now()])?;tx.commit()?;Ok(n)}).await
}

pub(super) fn media_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let metadata =
        serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or_else(|_| json!({}));
    let overrides =
        serde_json::from_str::<Value>(&r.get::<_, String>(7)?).unwrap_or_else(|_| json!({}));
    let title = overrides
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .get("title")
                .or_else(|| metadata.get("name"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .unwrap_or(r.get::<_, String>(2)?);
    let year = overrides
        .get("year")
        .and_then(Value::as_i64)
        .or_else(|| {
            metadata
                .get("release_date")
                .or_else(|| metadata.get("first_air_date"))
                .and_then(Value::as_str)
                .and_then(|s| s.get(..4))
                .and_then(|s| s.parse().ok())
        })
        .or(r.get::<_, Option<i64>>(4)?);
    let overview = overrides
        .get("overview")
        .or_else(|| metadata.get("overview"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(
        json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"title":title,"overview":overview,"parent_id":r.get::<_,Option<String>>(3)?,"year":year,"sort_number":r.get::<_,Option<i64>>(5)?,"metadata":metadata,"overrides":overrides,"available":r.get::<_,bool>(8)?}),
    )
}
