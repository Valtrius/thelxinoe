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

/// A page of the matching items and how many match.
pub(super) async fn browse(
    db: &Database,
    input: Browse,
    limit: u32,
) -> anyhow::Result<(Vec<Value>, u32)> {
    db.read("library.browse", move|db|{
        const TITLE: &str = "COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title)";
        const MATCHING: &str = "FROM media m WHERE (?1 IS NULL OR m.kind=?1) AND (?2 IS NULL OR m.parent_id=?2) AND (?3 IS NULL OR COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) LIKE '%'||?3||'%') AND (?4 IS NULL OR json_extract(m.metadata,'$.belongs_to_collection.id')=?4) AND (?5 IS NULL OR m.parent_id=?5 OR m.parent_id IN (SELECT id FROM media WHERE parent_id=?5))";
        // What a music page shows beside each item: a track's length, format, album and
        // artist; an album's track count and length; an artist's album and track counts.
        const MUSIC: &str = "CASE m.kind WHEN 'track' THEN (SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1 ORDER BY f.id LIMIT 1)
                WHEN 'album' THEN (SELECT SUM(CAST(json_extract(f.probe,'$.format.duration') AS REAL)) FROM media t JOIN media_sources s ON s.media_id=t.id JOIN media_files f ON f.id=s.file_id WHERE t.parent_id=m.id AND f.present=1)
                WHEN 'artist' THEN (SELECT SUM(CAST(json_extract(f.probe,'$.format.duration') AS REAL)) FROM media a JOIN media t ON t.parent_id=a.id JOIN media_sources s ON s.media_id=t.id JOIN media_files f ON f.id=s.file_id WHERE a.parent_id=m.id AND f.present=1) END,
            CASE m.kind WHEN 'album' THEN (SELECT COUNT(*) FROM media t WHERE t.parent_id=m.id) WHEN 'artist' THEN (SELECT COUNT(*) FROM media t JOIN media a ON t.parent_id=a.id WHERE a.parent_id=m.id) END,
            CASE m.kind WHEN 'artist' THEN (SELECT COUNT(*) FROM media a WHERE a.parent_id=m.id) END,
            CASE m.kind WHEN 'track' THEN (SELECT json_object('codec',json_extract(st.value,'$.codec_name'),'sample_rate',CAST(json_extract(st.value,'$.sample_rate') AS INTEGER),'bit_depth',NULLIF(COALESCE(CAST(json_extract(st.value,'$.bits_per_raw_sample') AS INTEGER),CAST(json_extract(st.value,'$.bits_per_sample') AS INTEGER),0),0)) FROM media_sources s JOIN media_files f ON f.id=s.file_id, json_each(f.probe,'$.streams') st WHERE s.media_id=m.id AND f.present=1 AND json_extract(st.value,'$.codec_type')='audio' ORDER BY f.id LIMIT 1) END,
            CASE WHEN m.kind IN ('track','album') THEN (SELECT COALESCE(json_extract(p.metadata,'$.title'),json_extract(p.metadata,'$.name'),p.title) FROM media p WHERE p.id=m.parent_id) END,
            CASE WHEN m.kind='track' THEN (SELECT COALESCE(json_extract(g.metadata,'$.title'),json_extract(g.metadata,'$.name'),g.title) FROM media p JOIN media g ON g.id=p.parent_id WHERE p.id=m.parent_id) END,
            CASE WHEN m.kind='track' THEN (SELECT p.parent_id FROM media p WHERE p.id=m.parent_id) END";
        let filter = params![input.kind,input.parent,input.q,input.collection,input.ancestor];
        let total = db.query_row(&format!("SELECT COUNT(*) {MATCHING}"), filter, |r| r.get(0))?;
        let direction = match input.order { Order::Asc => "ASC", Order::Desc => "DESC" };
        let order_by = match input.sort {
            Some(Sort::Year) => format!("m.year {direction} NULLS LAST,{TITLE} COLLATE NOCASE,m.id"),
            Some(Sort::Added) => format!("m.created_at {direction},{TITLE} COLLATE NOCASE,m.id"),
            Some(_) => format!("{TITLE} COLLATE NOCASE {direction},m.id"),
            // An artist's tracks come album by album.
            None if input.ancestor.is_some() => format!("(SELECT p.year FROM media p WHERE p.id=m.parent_id) NULLS LAST,(SELECT COALESCE(json_extract(p.metadata,'$.title'),json_extract(p.metadata,'$.name'),p.title) FROM media p WHERE p.id=m.parent_id) COLLATE NOCASE,m.sort_number,{TITLE} COLLATE NOCASE,m.id"),
            None => format!("m.sort_number,{TITLE} COLLATE NOCASE,m.id"),
        };
        let mut query=db.prepare(&format!("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1),{MUSIC} {MATCHING} ORDER BY {order_by} LIMIT ?6 OFFSET ?7"))?;
        let items = query.query_map(params![input.kind,input.parent,input.q,input.collection,input.ancestor,limit,input.offset],music_row)?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok((items, total))
    }).await
}

/// A browsed row with the music facts its kind carries, read after `media_row`'s columns.
fn music_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut item = media_row(r)?;
    let kind = item["kind"].as_str().unwrap_or_default().to_owned();
    if !["artist", "album", "track"].contains(&kind.as_str()) {
        return Ok(item);
    }
    item["duration"] = json!(r.get::<_, Option<f64>>(8)?);
    match kind.as_str() {
        "track" => {
            item["audio"] = r
                .get::<_, Option<String>>(11)?
                .and_then(|audio| serde_json::from_str::<Value>(&audio).ok())
                .unwrap_or(Value::Null);
            item["album"] = json!(r.get::<_, Option<String>>(12)?);
            item["artist"] = json!(r.get::<_, Option<String>>(13)?);
            item["artist_id"] = json!(r.get::<_, Option<String>>(14)?);
        }
        "album" => {
            item["track_count"] = json!(r.get::<_, Option<i64>>(9)?.unwrap_or(0));
            item["artist"] = json!(r.get::<_, Option<String>>(12)?);
        }
        _ => {
            item["track_count"] = json!(r.get::<_, Option<i64>>(9)?.unwrap_or(0));
            item["album_count"] = json!(r.get::<_, Option<i64>>(10)?.unwrap_or(0));
        }
    }
    Ok(item)
}

/// Every top-level movie or show in a fixed order, with when it joined the library and
/// what its files weigh. A show is available once any of its episodes is.
pub(super) async fn titles(db: &Database, kind: String) -> anyhow::Result<Vec<(Value, Facts)>> {
    db.read("library.titles", move |db| {
        let mut query = db.prepare("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,
                CASE m.kind WHEN 'show' THEN EXISTS(SELECT 1 FROM media s JOIN media e ON e.parent_id=s.id JOIN media_sources ms ON ms.media_id=e.id JOIN media_files f ON f.id=ms.file_id WHERE s.parent_id=m.id AND f.present=1)
                    ELSE EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) END,
                m.created_at,
                (SELECT SUM(f.size) FROM media_files f WHERE f.present=1 AND f.id IN (
                    SELECT file_id FROM media_sources WHERE media_id=m.id
                    UNION SELECT ms.file_id FROM media s JOIN media e ON e.parent_id=s.id JOIN media_sources ms ON ms.media_id=e.id WHERE s.parent_id=m.id)),
                (SELECT COUNT(DISTINCT e.id) FROM media s JOIN media e ON e.parent_id=s.id JOIN media_sources ms ON ms.media_id=e.id JOIN media_files f ON f.id=ms.file_id WHERE s.parent_id=m.id AND f.present=1)
            FROM media m WHERE m.kind=?1 AND m.parent_id IS NULL ORDER BY m.id")?;
        Ok(query
            .query_map([kind], |r| {
                Ok((
                    media_row(r)?,
                    Facts {
                        added: Some(r.get(8)?),
                        size: r.get::<_, Option<i64>>(9)?.filter(|size| *size > 0),
                        episodes: Some(r.get::<_, i64>(10)?).filter(|count| *count > 0),
                        ..Facts::default()
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

pub(super) struct CardPresentation {
    pub(super) year: Option<i64>,
    pub(super) artwork_id: Option<String>,
    pub(super) watched: bool,
    /// Something in it is watched or has a resume position.
    pub(super) started: bool,
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
                SELECT d.card_id,MIN(COALESCE(s.watched,0)) AS completed,
                    MAX(COALESCE(s.watched,0) OR EXISTS(SELECT 1 FROM edition_progress e WHERE e.user_id=?2 AND e.media_id=d.id AND e.position>0)) AS started
                FROM descendants d
                LEFT JOIN media_state s ON s.media_id=d.id AND s.user_id=?2
                WHERE d.kind IN ('movie','episode') GROUP BY d.card_id
            )
            SELECT m.id,m.year,
                CASE WHEN json_extract(m.metadata,'$.artwork_cached')=1 THEN m.id WHEN json_extract(p.metadata,'$.artwork_cached')=1 THEN p.id WHEN json_extract(g.metadata,'$.artwork_cached')=1 THEN g.id END,
                COALESCE(w.completed,0),COALESCE(w.started,0)
            FROM media m LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media g ON g.id=p.parent_id
            LEFT JOIN watched w ON w.card_id=m.id WHERE m.id IN (SELECT value FROM json_each(?1))"
        )?;
        Ok(query.query_map(params![ids,user], |r| Ok((r.get::<_,String>(0)?, CardPresentation {
            year: r.get(1)?, artwork_id: r.get(2)?, watched: r.get(3)?, started: r.get(4)?,
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
