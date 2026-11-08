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

/// A media item's shown title, as `media_by_title` indexes it.
const TITLE: &str =
    "COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title)";
/// What a music page shows beside each item: a track's length, format, album and artist; an
/// album's track count and length; an artist's album and track counts.
const MUSIC: &str = "CASE m.kind WHEN 'track' THEN (SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1 ORDER BY f.id LIMIT 1)
                WHEN 'album' THEN (SELECT SUM((SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=t.id AND f.present=1 ORDER BY f.id LIMIT 1)) FROM media t WHERE t.parent_id=m.id)
                WHEN 'artist' THEN (SELECT SUM((SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=t.id AND f.present=1 ORDER BY f.id LIMIT 1)) FROM media a JOIN media t ON t.parent_id=a.id WHERE a.parent_id=m.id) END,
            CASE m.kind WHEN 'album' THEN (SELECT COUNT(*) FROM media t WHERE t.parent_id=m.id) WHEN 'artist' THEN (SELECT COUNT(*) FROM media t JOIN media a ON t.parent_id=a.id WHERE a.parent_id=m.id) END,
            CASE m.kind WHEN 'artist' THEN (SELECT COUNT(*) FROM media a WHERE a.parent_id=m.id) END,
            CASE m.kind WHEN 'track' THEN (SELECT json_object('codec',json_extract(st.value,'$.codec_name'),'sample_rate',CAST(json_extract(st.value,'$.sample_rate') AS INTEGER),'bit_depth',NULLIF(COALESCE(CAST(json_extract(st.value,'$.bits_per_raw_sample') AS INTEGER),CAST(json_extract(st.value,'$.bits_per_sample') AS INTEGER),0),0)) FROM media_sources s JOIN media_files f ON f.id=s.file_id, json_each(f.probe,'$.streams') st WHERE s.media_id=m.id AND f.present=1 AND json_extract(st.value,'$.codec_type')='audio' ORDER BY f.id LIMIT 1) END,
            CASE WHEN m.kind IN ('track','album') THEN (SELECT COALESCE(json_extract(p.metadata,'$.title'),json_extract(p.metadata,'$.name'),p.title) FROM media p WHERE p.id=m.parent_id) END,
            CASE WHEN m.kind='track' THEN (SELECT COALESCE(json_extract(g.metadata,'$.title'),json_extract(g.metadata,'$.name'),g.title) FROM media p JOIN media g ON g.id=p.parent_id WHERE p.id=m.parent_id) END,
            CASE WHEN m.kind='track' THEN (SELECT p.parent_id FROM media p WHERE p.id=m.parent_id) END";

/// A page of the matching items and how many match.
pub(super) async fn browse(
    db: &Database,
    input: Browse,
    limit: u32,
) -> anyhow::Result<(Vec<Value>, u32)> {
    db.read("library.browse", move |db| browse_page(db, input, limit))
        .await
}

fn browse_page(
    db: &rusqlite::Connection,
    input: Browse,
    limit: u32,
) -> anyhow::Result<(Vec<Value>, u32)> {
    let (count, page, mut filter) = browse_queries(&input);
    let total = db.query_row(&count, rusqlite::params_from_iter(&filter), |r| r.get(0))?;
    filter.extend([limit.into(), input.offset.into()]);
    let items = db
        .prepare(&page)?
        .query_map(rusqlite::params_from_iter(&filter), music_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((items, total))
}

/// How a browsed list is counted and paged, and the values both read. The page reads its
/// limit and offset after them.
fn browse_queries(input: &Browse) -> (String, String, Vec<rusqlite::types::Value>) {
    // Only the filters asked for, so an index can find and order the matches.
    let mut conditions = Vec::new();
    let mut filter = Vec::<rusqlite::types::Value>::new();
    let mut bind = |value: rusqlite::types::Value| {
        filter.push(value);
        format!("?{}", filter.len())
    };
    if let Some(kind) = &input.kind {
        // A parent's children are found by `media_parent`; the unary plus keeps SQLite,
        // which has no statistics, from reading every item of the kind instead.
        let indexed = input.parent.is_none() && input.ancestor.is_none();
        conditions.push(format!(
            "{}m.kind={}",
            if indexed { "" } else { "+" },
            bind(kind.clone().into())
        ));
    }
    if let Some(parent) = &input.parent {
        conditions.push(format!("m.parent_id={}", bind(parent.clone().into())));
    }
    if let Some(q) = &input.q {
        // The indexed value, so the index alone answers it. LIKE folds case either way.
        conditions.push(format!(
            "{TITLE} COLLATE NOCASE LIKE '%'||{}||'%'",
            bind(q.clone().into())
        ));
    }
    if let Some(collection) = input.collection {
        conditions.push(format!(
            "json_extract(m.metadata,'$.belongs_to_collection.id')={}",
            bind(collection.into())
        ));
    }
    if let Some(ancestor) = &input.ancestor {
        let ancestor = bind(ancestor.clone().into());
        conditions.push(format!("(m.parent_id={ancestor} OR m.parent_id IN (SELECT id FROM media WHERE parent_id={ancestor}))"));
    }
    let matching = if conditions.is_empty() {
        "FROM media m".to_owned()
    } else {
        format!("FROM media m WHERE {}", conditions.join(" AND "))
    };
    let order_by = order_by(input);
    // The page is chosen before what its items show is read, which costs the most.
    let page = format!("LIMIT ?{} OFFSET ?{}", filter.len() + 1, filter.len() + 2);
    (
        format!("SELECT COUNT(*) {matching}"),
        format!(
            "WITH page AS MATERIALIZED (SELECT m.id {matching} ORDER BY {order_by} {page}) SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1),{MUSIC} FROM page JOIN media m ON m.id=page.id ORDER BY {order_by}"
        ),
        filter,
    )
}

/// The order of a browsed list, ending in the id so that every page cuts it the same way.
fn order_by(input: &Browse) -> String {
    let direction = match input.order {
        Order::Asc => "ASC",
        Order::Desc => "DESC",
    };
    match input.sort {
        Some(Sort::Year) => format!("m.year {direction} NULLS LAST,{TITLE} COLLATE NOCASE,m.id"),
        Some(Sort::Added) => format!("m.created_at {direction},{TITLE} COLLATE NOCASE,m.id"),
        Some(_) => format!("{TITLE} COLLATE NOCASE {direction},m.id"),
        // An artist's tracks come album by album.
        None if input.ancestor.is_some() => format!(
            "(SELECT p.year FROM media p WHERE p.id=m.parent_id) NULLS LAST,(SELECT COALESCE(json_extract(p.metadata,'$.title'),json_extract(p.metadata,'$.name'),p.title) FROM media p WHERE p.id=m.parent_id) COLLATE NOCASE,m.sort_number,{TITLE} COLLATE NOCASE,m.id"
        ),
        None => format!("m.sort_number,{TITLE} COLLATE NOCASE,m.id"),
    }
}

/// A playlist's tracks in their manual order, without what only their own page shows, so a
/// long playlist stays a small response.
pub(super) async fn playlist_tracks(db: &Database, playlist: String) -> anyhow::Result<Vec<Value>> {
    db.read("library.playlist_tracks", move |db| {
        const TITLE: &str =
            "COALESCE(json_extract(?.metadata,'$.title'),json_extract(?.metadata,'$.name'),?.title)";
        let title = |alias: &str| TITLE.replace('?', alias);
        let mut query = db.prepare(&format!(
            "SELECT m.id,m.kind,{},m.parent_id,m.year,m.sort_number,'{{}}',
                EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1),
                (SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1 ORDER BY f.id LIMIT 1),
                NULL,NULL,NULL,{},{},album.parent_id,i.added_at
            FROM playlist_items i JOIN media m ON m.id=i.media_id
            LEFT JOIN media album ON album.id=m.parent_id
            LEFT JOIN media artist ON artist.id=album.parent_id
            WHERE i.playlist_id=?1 ORDER BY i.position",
            title("m"),
            title("album"),
            title("artist"),
        ))?;
        Ok(query
            .query_map([playlist], |r| {
                let mut item = music_row(r)?;
                item["added_at"] = json!(r.get::<_, i64>(15)?);
                Ok(item)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
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

/// What a movie or show list sorts and filters by, out of metadata that often holds
/// kilobytes per title. The rest is read for the page returned.
const LISTED: [&str; 9] = [
    "title",
    "name",
    "release_date",
    "first_air_date",
    "vote_average",
    "genres",
    "runtime",
    "status",
    "belongs_to_collection",
];
/// Keys `Value` reads as serde_json's own wrappers rather than as fields.
const PRIVATE: &str = "$serde_json::private::";

/// The listed fields, read without building the rest. It fails wherever reading the
/// whole metadata into a `Value` could fail or read it differently.
struct ListedMetadata(Value);
impl<'de> serde::Deserialize<'de> for ListedMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = ListedMetadata;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut fields: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut listed = serde_json::Map::new();
                while let Some(Key(key)) = fields.next_key()? {
                    if LISTED.contains(&key.as_str()) {
                        listed.insert(key, fields.next_value()?);
                    } else {
                        fields.next_value::<Unread>()?;
                    }
                }
                Ok(ListedMetadata(Value::Object(listed)))
            }
        }
        deserializer.deserialize_map(Fields)
    }
}

/// An object key that `Value` reads as a field.
struct Key(String);
impl<'de> serde::Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        if key.starts_with(PRIVATE) {
            return Err(serde::de::Error::custom("serde_json wrapper"));
        }
        Ok(Key(key))
    }
}

/// A value checked as reading it into a `Value` would check it, without keeping it.
struct Unread;
impl<'de> serde::Deserialize<'de> for Unread {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(Unread)
    }
}
impl<'de> serde::de::Visitor<'de> for Unread {
    type Value = Unread;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_unit<E>(self) -> std::result::Result<Self, E> {
        Ok(Unread)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut items: A,
    ) -> std::result::Result<Self, A::Error> {
        while items.next_element::<Unread>()?.is_some() {}
        Ok(Unread)
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut fields: A,
    ) -> std::result::Result<Self, A::Error> {
        while fields.next_entry::<Key, Unread>()?.is_some() {}
        Ok(Unread)
    }
}

/// A `media_row` card holding only the listed metadata.
fn listed_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let stored = r.get::<_, String>(6)?;
    let metadata = match serde_json::from_str::<ListedMetadata>(&stored) {
        Ok(listed) => listed.0,
        // What the quick read can't vouch for is read as a page reads it.
        Err(_) => match whole_metadata(&stored) {
            Value::Object(mut fields) => {
                fields.retain(|key, _| LISTED.contains(&key.as_str()));
                Value::Object(fields)
            }
            other => other,
        },
    };
    card_row(r, metadata)
}

/// Every top-level movie or show in a fixed order, with when it joined the library and
/// what its files weigh. A show is available once any of its episodes is. Each card holds
/// only the metadata its list needs.
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
                    listed_row(r)?,
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
    card_row(r, whole_metadata(&r.get::<_, String>(6)?))
}

pub(super) fn whole_metadata(stored: &str) -> Value {
    serde_json::from_str::<Value>(stored).unwrap_or_else(|_| json!({}))
}

fn card_row(r: &rusqlite::Row<'_>, metadata: Value) -> rusqlite::Result<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A browsed list as it was read before only the asked filters were applied and the
    /// page was chosen first.
    fn browsed_before(db: &rusqlite::Connection, input: &Browse, limit: u32) -> (Vec<Value>, u32) {
        let matching = "FROM media m WHERE (?1 IS NULL OR m.kind=?1) AND (?2 IS NULL OR m.parent_id=?2) AND (?3 IS NULL OR COALESCE(json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) LIKE '%'||?3||'%') AND (?4 IS NULL OR json_extract(m.metadata,'$.belongs_to_collection.id')=?4) AND (?5 IS NULL OR m.parent_id=?5 OR m.parent_id IN (SELECT id FROM media WHERE parent_id=?5))";
        let filter = rusqlite::params![
            input.kind,
            input.parent,
            input.q,
            input.collection,
            input.ancestor
        ];
        let total = db
            .query_row(&format!("SELECT COUNT(*) {matching}"), filter, |r| r.get(0))
            .unwrap();
        let items = db
            .prepare(&format!("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1),{MUSIC} {matching} ORDER BY {} LIMIT ?6 OFFSET ?7", order_by(input)))
            .unwrap()
            .query_map(
                rusqlite::params![
                    input.kind,
                    input.parent,
                    input.q,
                    input.collection,
                    input.ancestor,
                    limit,
                    input.offset
                ],
                music_row,
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (items, total)
    }

    /// Every list reads as it did, whatever its filters, sort and page, among titles that
    /// tie, differ in case, come from metadata or are not text.
    #[tokio::test]
    async fn browsed_lists_read_as_before() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::open(temp.path().join("db.sqlite3")).unwrap();
        db.write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/music'),('movies','Movies','movies','/movies')", [])?;
            for (id, root, kind, parent, title, metadata, sort, year, created) in [
                ("ar1", "music", "artist", None, "Alpha", "{}", None, None, 1),
                ("ar2", "music", "artist", None, "alpha", "{}", None, None, 1),
                ("ar3", "music", "artist", None, "Zed", r#"{"name":"Beta"}"#, None, None, 2),
                ("al1", "music", "album", Some("ar1"), "One", "{}", None, Some(2001), 5),
                ("al2", "music", "album", Some("ar1"), "one", "{}", None, None, 5),
                ("al3", "music", "album", Some("ar2"), "Two", "{}", None, Some(1999), 3),
                ("al4", "music", "album", Some("ar3"), "x", r#"{"title":"Alpha Album"}"#, None, Some(2001), 9),
                ("t1", "music", "track", Some("al1"), "Intro", "{}", Some(1), None, 6),
                ("t2", "music", "track", Some("al1"), "intro", "{}", Some(2), None, 6),
                ("t3", "music", "track", Some("al1"), "Song", "{}", None, None, 7),
                ("t4", "music", "track", Some("al2"), "Song", "{}", Some(1), None, 6),
                ("t5", "music", "track", Some("al3"), "Number", r#"{"title":7}"#, Some(1), None, 8),
                ("t6", "music", "track", Some("al3"), "Stored", r#"{"title":null,"name":"Named"}"#, Some(2), None, 8),
                ("t7", "music", "track", Some("al4"), "Zulu", "{}", Some(1), None, 10),
                ("t8", "music", "track", Some("al4"), "alpha", "{}", Some(1), None, 10),
                ("t9", "music", "track", None, "Loose", "{}", None, None, 11),
                ("m1", "movies", "movie", None, "Movie", r#"{"title":"Film","belongs_to_collection":{"id":10}}"#, None, Some(2001), 4),
                ("m2", "movies", "movie", None, "Other", r#"{"belongs_to_collection":{"id":"10"}}"#, None, None, 4),
                ("m3", "movies", "movie", None, "intro", "{}", None, Some(1999), 12),
            ] {
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,metadata,sort_number,year,created_at) VALUES (?1,?2,?3,?4,?1,?5,?6,?7,?8,?9)", rusqlite::params![id, root, kind, parent, title, metadata, sort, year, created])?;
            }
            for (file, media, duration) in [("f1", "t1", "200.5"), ("f1b", "t1", "300"), ("f2", "t2", "100"), ("f7", "t7", "60")] {
                let probe = json!({"format":{"duration":duration},"streams":[{"codec_type":"audio","codec_name":"flac","sample_rate":"44100","bits_per_raw_sample":"16"}]}).to_string();
                db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,'music',?1,'g',1,'m',?1,?2,1)", rusqlite::params![file, probe])?;
                db.execute("INSERT INTO media_sources(media_id,file_id) VALUES (?1,?2)", rusqlite::params![media, file])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let compared = db
            .read("test.browse", |db| {
                let mut compared = 0;
                for kind in [None, Some("track"), Some("album"), Some("artist"), Some("movie")] {
                    for parent in [None, Some("al1"), Some("ar1")] {
                        for ancestor in [None, Some("ar1")] {
                            for q in [None, Some(""), Some("al"), Some("INTRO"), Some("7")] {
                                for collection in [None, Some(10)] {
                                    for sort in [None, Some(Sort::Title), Some(Sort::Year), Some(Sort::Added)] {
                                        for order in [Order::Asc, Order::Desc] {
                                            for (limit, offset) in [(500, 0), (2, 1), (0, 0)] {
                                                let input = || Browse {
                                                    kind: kind.map(Into::into),
                                                    parent: parent.map(Into::into),
                                                    ancestor: ancestor.map(Into::into),
                                                    q: q.map(Into::into),
                                                    collection,
                                                    sort,
                                                    order,
                                                    offset,
                                                    ..Default::default()
                                                };
                                                assert_eq!(
                                                    browse_page(db, input(), limit)?,
                                                    browsed_before(db, &input(), limit),
                                                    "{kind:?} {parent:?} {ancestor:?} {q:?} {collection:?} {limit} {offset}"
                                                );
                                                compared += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(compared)
            })
            .await
            .unwrap();
        assert_eq!(compared, 5 * 3 * 2 * 5 * 2 * 4 * 2 * 3);
    }

    /// How SQLite, without statistics as in production, chooses each list's page.
    #[tokio::test]
    async fn browsed_pages_are_chosen_by_index() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::open(temp.path().join("db.sqlite3")).unwrap();
        let plans = db
            .read("test.plan", |db| {
                let plan = |input: Browse| {
                    let (count, page, _) = browse_queries(&input);
                    let steps = |sql: &str| {
                        let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                        let unbound =
                            vec![rusqlite::types::Value::Null; statement.parameter_count()];
                        statement
                            .query_map(rusqlite::params_from_iter(unbound), |r| {
                                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get(3)?))
                            })?
                            .collect::<rusqlite::Result<Vec<(i64, i64, String)>>>()
                    };
                    // What chooses the page, below its materialization.
                    let mut chosen = Vec::new();
                    let mut ids = Vec::new();
                    for (id, parent, detail) in steps(&page)? {
                        if detail.starts_with("MATERIALIZE page") || ids.contains(&parent) {
                            ids.push(id);
                            chosen.push(detail);
                        }
                    }
                    let counted = steps(&count)?.into_iter().map(|step| step.2).collect();
                    Ok::<_, anyhow::Error>((chosen, counted))
                };
                let track = || Browse {
                    kind: Some("track".into()),
                    sort: Some(Sort::Title),
                    ..Default::default()
                };
                Ok([
                    plan(track())?,
                    plan(Browse {
                        q: Some("q".into()),
                        ..track()
                    })?,
                    plan(Browse {
                        ancestor: Some("artist".into()),
                        sort: None,
                        ..track()
                    })?,
                ])
            })
            .await
            .unwrap();
        // One kind in title order, found by its title too, is paged and counted from the
        // index alone, without sorting.
        let covered = "SEARCH m USING COVERING INDEX media_by_title (kind=?)".to_owned();
        for (page, count) in &plans[..2] {
            assert!(page.contains(&covered), "{page:?}");
            assert!(
                !page.iter().any(|step| step.contains("TEMP B-TREE")),
                "{page:?}"
            );
            assert_eq!(count, std::slice::from_ref(&covered));
        }
        // An artist's tracks are found from the artist.
        let (page, count) = &plans[2];
        for steps in [page, count] {
            assert!(
                steps.contains(&"SEARCH m USING INDEX media_parent (parent_id=?)".into())
                    && !steps.iter().any(|step| step.contains("media_by_title")),
                "{steps:?}"
            );
        }
    }

    /// A list reads the same title and year from the fields it keeps as a page does from
    /// the whole metadata, whatever those fields hold.
    #[test]
    fn listed_cards_match_whole_cards() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for metadata in [
            r#"{"title":"Own","name":"Other","overview":"Kept","release_date":"2001-02-03"}"#,
            r#"{"title":null,"name":"Other","first_air_date":"2002-03-04"}"#,
            r#"{"title":7,"name":"Other","release_date":"unknown"}"#,
            r#"{"name":"Other","first_air_date":null,"release_date":"20"}"#,
            r#"{"title":"Escaped","extra":{"title":"Nested"},"genres":[{"name":"Drama"}]}"#,
            r#"{"title":"First","title":"Last"}"#,
            r#"{"title":"Own","release_date":"2001-02-03","overview":"\uD800"}"#,
            r#"{"title":"Own","release_date":"2001-02-03","popularity":1e400}"#,
            r#"{"title":"Own","release_date":"2001-02-03","extra":{"\uDC00":1}}"#,
            r#"{"$serde_json::private::RawValue":"{\"title\":\"Wrapped\",\"release_date\":\"2001-01-01\"}"}"#,
            r#"{"\u0024serde_json::private::RawValue":"{\"title\":\"Wrapped\",\"genres\":[]}"}"#,
            r#"{"$serde_json::private::RawValue":"[1]"}"#,
            r#"{"$serde_json::private::RawValue":"not json","title":"Own"}"#,
            r#"{"title":"Own","extra":{"$serde_json::private::RawValue":"not json"}}"#,
            r#"{"title":"Own","extra":[{"$serde_json::private::RawValue":"{}"}]}"#,
            r#"{"title":"Own","$serde_json::private::RawValue":"{}"}"#,
            &format!(
                r#"{{"title":"Own","release_date":"2001-02-03","extra":{}1{}}}"#,
                "[".repeat(200),
                "]".repeat(200)
            ),
            &format!(
                r#"{{"title":"Own","release_date":"2001-02-03","extra":{}1{}}}"#,
                "[".repeat(100),
                "]".repeat(100)
            ),
            r#"[{"title":"Not an object"}]"#,
            r#""text""#,
            "{",
            "",
        ] {
            let row = |r: &rusqlite::Row<'_>| Ok((listed_row(r)?, media_row(r)?));
            let (listed, whole) = db
                .query_row(
                    "SELECT 'id','movie','Stored',NULL,1999,NULL,?1,1",
                    [metadata],
                    row,
                )
                .unwrap();
            assert_eq!(listed["title"], whole["title"], "{metadata}");
            assert_eq!(listed["year"], whole["year"], "{metadata}");
            let mut kept = whole["metadata"].clone();
            if let Some(fields) = kept.as_object_mut() {
                fields.retain(|key, _| LISTED.contains(&key.as_str()));
            }
            assert_eq!(listed["metadata"], kept, "{metadata}");
        }
    }

    #[test]
    fn listed_cards_leave_out_what_only_a_page_shows() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let listed = db
            .query_row(
                "SELECT 'id','movie','Stored',NULL,1999,NULL,?1,1",
                [r#"{"title":"Own","overview":"Long","credits":{"cast":[]},"runtime":90}"#],
                listed_row,
            )
            .unwrap();
        assert_eq!(listed["overview"], Value::Null);
        assert_eq!(listed["metadata"], json!({"title":"Own","runtime":90}));
    }
}
