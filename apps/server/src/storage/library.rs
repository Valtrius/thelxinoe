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

pub(super) async fn rename_root(
    db: &Database,
    p: thelxinoe_core::Principal,
    root_id: String,
    name: String,
) -> anyhow::Result<bool> {
    db.write("library.rename_root", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE library_roots SET name=?2 WHERE id=?1",
            params![root_id, name],
        )? == 0
        {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'library.rename',?2,?3)",
            params![p.user.id, root_id, now()],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await
}

pub(super) async fn remove_root(
    db: &Database,
    p: thelxinoe_core::Principal,
    root_id: String,
) -> anyhow::Result<bool> {
    db.write("library.remove_root", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM library_roots WHERE id=?1)",
            [&root_id],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(false);
        }
        // Queued work for the library has nothing left to act on. A running scan finds the
        // library gone and stops without writing.
        tx.execute(
            "UPDATE jobs SET state='complete',completed_at=?2 WHERE state='queued' AND ((kind='library.scan' AND subject=?1) OR (kind IN ('metadata.match','metadata.refresh') AND subject IN (SELECT id FROM media WHERE root_id=?1)))",
            params![root_id, now()],
        )?;
        // Rows that refer to the library's entries and files without cascading go first.
        // Everyone's state for the entries cascades with them; playback history keeps its
        // own copy of each title.
        for statement in [
            "DELETE FROM retention_candidates WHERE media_id IN (SELECT id FROM media WHERE root_id=?1)",
            "DELETE FROM media_operations WHERE media_id IN (SELECT id FROM media WHERE root_id=?1)",
            "DELETE FROM media_sources WHERE media_id IN (SELECT id FROM media WHERE root_id=?1)",
            "DELETE FROM local_trailers WHERE media_id IN (SELECT id FROM media WHERE root_id=?1)",
            "DELETE FROM manager_bindings WHERE file_id IN (SELECT id FROM media_files WHERE root_id=?1)",
            "DELETE FROM media WHERE root_id=?1",
            "DELETE FROM media_files WHERE root_id=?1",
            "DELETE FROM library_roots WHERE id=?1",
        ] {
            tx.execute(statement, [&root_id])?;
        }
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'library.remove',?2,?3)",
            params![p.user.id, root_id, now()],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await
}

pub(super) async fn enqueue_scan(root_id: String, db: &Database) -> anyhow::Result<String> {
    // A scan already waiting covers this request; one that is running may miss the change.
    db.write("library.enqueue_scan", move |db| {
        Ok(thelxinoe_jobs::enqueue_in(
            db,
            "library.scan",
            &root_id,
            &json!({ "root_id": root_id }),
            0,
        )?)
    })
    .await
}

pub(super) async fn browse(db: &Database, input: Browse) -> anyhow::Result<Vec<Value>> {
    db.read("library.browse", move|db|{
        let mut query=db.prepare("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE (?1 IS NULL OR m.kind=?1) AND (?2 IS NULL OR m.parent_id=?2) AND (?3 IS NULL OR COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) LIKE '%'||?3||'%') AND (?4 IS NULL OR json_extract(m.metadata,'$.belongs_to_collection.id')=?4) ORDER BY m.sort_number,COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) COLLATE NOCASE LIMIT 500")?;
        Ok(query.query_map(params![input.kind,input.parent,input.q,input.collection],media_row)?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await
}

pub(super) struct CardPresentation {
    pub(super) year: Option<i64>,
    pub(super) artwork_id: Option<String>,
    pub(super) watched: bool,
}

pub(super) async fn decorate_cards(
    ids: String,
    user: String,
    db: &Database,
) -> anyhow::Result<std::collections::HashMap<String, CardPresentation>> {
    db.read("library.decorate_cards", move |db| {
        let mut query = db.prepare(
            "WITH RECURSIVE descendants(card_id,id,kind) AS (
                SELECT id,id,kind FROM media WHERE id IN (SELECT value FROM json_each(?1))
                UNION ALL
                SELECT d.card_id,m.id,m.kind FROM descendants d JOIN media m ON m.parent_id=d.id WHERE d.kind IN ('show','season')
            ), watched AS (
                SELECT d.card_id,MIN(COALESCE(s.watched,0)) AS completed FROM descendants d
                LEFT JOIN media_state s ON s.media_id=d.id AND s.user_id=?2
                WHERE d.kind IN ('movie','episode') GROUP BY d.card_id
            )
            SELECT m.id,m.year,
                CASE WHEN json_extract(m.metadata,'$.artwork_cached')=1 THEN m.id WHEN json_extract(p.metadata,'$.artwork_cached')=1 THEN p.id WHEN json_extract(g.metadata,'$.artwork_cached')=1 THEN g.id END,
                COALESCE(w.completed,0)
            FROM media m LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media g ON g.id=p.parent_id
            LEFT JOIN watched w ON w.card_id=m.id WHERE m.id IN (SELECT value FROM json_each(?1))"
        )?;
        Ok(query.query_map(params![ids,user], |r| Ok((r.get::<_,String>(0)?, CardPresentation {
            year: r.get(1)?, artwork_id: r.get(2)?, watched: r.get(3)?,
        })))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?)
    }).await
}

pub(super) async fn detail(db: &Database, media_id: String) -> anyhow::Result<Option<Value>> {
    db.read("library.detail", move|db|{
        let item=db.query_row("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE m.id=?1",[&media_id],media_row).optional()?;
        let files=db.prepare("SELECT f.id,f.edition,f.probe,f.present FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1")?.query_map([&media_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"edition":r.get::<_,String>(1)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or_default(),"present":r.get::<_,bool>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let trailers=db.prepare("SELECT f.id,f.probe FROM local_trailers t JOIN media_files f ON f.id=t.file_id WHERE t.media_id=?1 AND f.present=1")?.query_map([&media_id],|r|Ok(json!({"file_id":r.get::<_,String>(0)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default()})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(item.map(|mut item|{item["files"]=json!(files);item["local_trailers"]=json!(trailers);item}))
    }).await
}

pub(super) struct Protection {
    pub(super) own: bool,
    /// The nearest season or series whose Keep covers this item.
    pub(super) by: Option<String>,
    /// Kept seasons and episodes inside this item, which also block its deletion.
    pub(super) within: i64,
}

pub(super) async fn keep(db: &Database, media_id: String) -> anyhow::Result<Protection> {
    db.read("library.keep", move |db| {
        let own = db.query_row(
            "SELECT COALESCE((SELECT keep FROM media_protection WHERE media_id=?1),0)",
            [&media_id],
            |r| r.get::<_, bool>(0),
        )?;
        let by = db.query_row("WITH RECURSIVE up(id,depth) AS (SELECT parent_id,1 FROM media WHERE id=?1 AND parent_id IS NOT NULL UNION ALL SELECT m.parent_id,u.depth+1 FROM media m JOIN up u ON m.id=u.id WHERE m.parent_id IS NOT NULL) SELECT m.kind FROM up JOIN media m ON m.id=up.id JOIN media_protection p ON p.media_id=up.id AND p.keep=1 ORDER BY up.depth LIMIT 1", [&media_id], |r| r.get::<_, String>(0)).optional()?;
        let within = db.query_row("WITH RECURSIVE down(id) AS (SELECT id FROM media WHERE parent_id=?1 UNION ALL SELECT m.id FROM media m JOIN down d ON m.parent_id=d.id) SELECT COUNT(*) FROM down JOIN media_protection p ON p.media_id=down.id AND p.keep=1", [&media_id], |r| r.get::<_, i64>(0))?;
        Ok(Protection { own, by, within })
    })
    .await
}

pub(super) fn media_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let metadata =
        serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or_else(|_| json!({}));
    let title = metadata
        .get("title")
        .or_else(|| metadata.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or(r.get::<_, String>(2)?);
    let year = metadata
        .get("release_date")
        .or_else(|| metadata.get("first_air_date"))
        .and_then(Value::as_str)
        .and_then(|s| s.get(..4))
        .and_then(|s| s.parse().ok())
        .or(r.get::<_, Option<i64>>(4)?);
    let overview = metadata.get("overview").cloned().unwrap_or(Value::Null);
    Ok(
        json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"title":title,"overview":overview,"parent_id":r.get::<_,Option<String>>(3)?,"year":year,"sort_number":r.get::<_,Option<i64>>(5)?,"metadata":metadata,"available":r.get::<_,bool>(7)?}),
    )
}
