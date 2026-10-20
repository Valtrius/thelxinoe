//! Database operations for catalog.
use super::*;

pub(super) async fn roots(db: &Database) -> anyhow::Result<Vec<Root>> {
    db.read("catalog.roots", |c| {
        Ok(c.prepare(
            "SELECT id,name,kind,path,last_scan,scan_error FROM library_roots ORDER BY name",
        )?
        .query_map([], |r| {
            Ok(Root {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                path: r.get(3)?,
                last_scan: r.get(4)?,
                scan_error: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
    })
    .await
}

pub(super) async fn scan_with_progress_read_media_files(
    lookup: String,
    db: &Database,
) -> anyhow::Result<Option<(i64, String, String, String)>> {
    db.read("catalog.scan_with_progress_read_media_files", move |c| {
        Ok(c.query_row(
            "SELECT size,modified,fingerprint,probe FROM media_files WHERE path=?1",
            [lookup],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?)
    })
    .await
}

pub(super) async fn scan_with_progress_read_present_paths(
    root: String,
    db: &Database,
) -> anyhow::Result<Vec<String>> {
    db.read("catalog.scan_with_progress_read_present_paths", move |c| {
        Ok(
            c.prepare("SELECT path FROM media_files WHERE root_id=?1 AND present=1")?
                .query_map([root], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?,
        )
    })
    .await
}

pub(super) async fn scan_with_progress_write_media_files(
    indexed: Vec<IndexedFile>,
    current_paths: std::collections::HashSet<String>,
    retained: std::collections::HashSet<String>,
    fingerprint_counts: std::collections::HashMap<String, usize>,
    warning: Option<String>,
    db: &Database,
    root: Root,
) -> anyhow::Result<()> {
    db.write("catalog.scan_with_progress_write_media_files", move|c|{
        let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let missing:Vec<(String,String)>=tx.prepare("SELECT id,path,fingerprint FROM media_files WHERE root_id=?1 AND present=1")?.query_map([&root.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?.into_iter().filter(|(_,path,_)|!current_paths.contains(path)).map(|(id,_,hash)|(id,hash)).collect();
        // Only files that are gone change; an unchanged library writes almost nothing. Their
        // scan time records when they went missing, which maintenance waits on before pruning.
        for (missing_id,_) in &missing {tx.execute("UPDATE media_files SET present=0,scanned_at=?2 WHERE id=?1",params![missing_id,now()])?;}
        // Files that couldn't be read this time keep their previous entry until a later scan.
        for path in &retained {tx.execute("UPDATE media_files SET present=1 WHERE root_id=?1 AND path=?2",params![root.id,path])?;}
        let mut vacated=Vec::new();
        for file in indexed {
            let old=tx.query_row("SELECT id,fingerprint,generation FROM media_files WHERE path=?1",[&file.path],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
            let file_id=old.as_ref().map(|o|o.0.clone()).unwrap_or_else(id);
            let generation=old.as_ref().filter(|o|o.1==file.fingerprint).map(|o|o.2.clone()).unwrap_or_else(id);
            // A unique exact-content move preserves logical identity. Ambiguous identical files
            // are kept distinct instead of merging unrelated entries by a guessed title.
            let candidates:Vec<_>=missing.iter().filter(|(_,hash)|hash==&file.fingerprint).collect();
            let source_file=if old.is_some(){Some(&file_id)}else if candidates.len()==1 && fingerprint_counts.get(&file.fingerprint)==Some(&1){Some(&candidates[0].0)}else{None};
            let previous:Vec<String>=if let Some(source)=source_file{tx.prepare("SELECT media_id FROM media_sources WHERE file_id=?1")?.query_map([source],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?}else{Vec::new()};
            let changed=tx.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,edition,present,scanned_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10) ON CONFLICT(path) DO UPDATE SET generation=excluded.generation,size=excluded.size,modified=excluded.modified,fingerprint=excluded.fingerprint,probe=excluded.probe,edition=excluded.edition,present=1,scanned_at=excluded.scanned_at,ownership=CASE WHEN generation=excluded.generation THEN ownership ELSE 'unresolved' END WHERE media_files.generation IS NOT excluded.generation OR media_files.size IS NOT excluded.size OR media_files.modified IS NOT excluded.modified OR media_files.fingerprint IS NOT excluded.fingerprint OR media_files.probe IS NOT excluded.probe OR media_files.edition IS NOT excluded.edition OR media_files.present=0",params![file_id,root.id,file.path,generation,file.size,file.modified,file.fingerprint,file.probe.to_string(),file.edition,now()])?;
            // An unchanged file keeps its row and its media links as they are.
            if changed==0 {continue;}
            tx.execute("DELETE FROM media_sources WHERE file_id=?1",[&file_id])?;
            tx.execute("DELETE FROM local_trailers WHERE file_id=?1",[&file_id])?;
            let trailer=file.probe["thelxinoe_local_trailer"].as_bool()==Some(true);
            let linked=place(&tx,&root.id,&file_id,&file.identities,&previous,trailer,&mut vacated)?;
            vacated.extend(previous.into_iter().filter(|media|!linked.contains(media)));
        }
        prune(&tx,vacated)?;
        tx.execute("UPDATE library_roots SET last_scan=?1,scan_error=?3 WHERE id=?2",params![now(),root.id,warning])?;tx.commit()?;Ok(())
    }).await
}

/// Places a changed file's identities, parents first, and links the file to the playable
/// ones. The entry the file stood for keeps its id, and with it watch state, favorites and
/// history, while its title and place follow the file; unless another present file still
/// stands for it, or its new identity already has an entry of its own.
fn place(
    tx: &rusqlite::Connection,
    root: &str,
    file: &str,
    identities: &[identity::Identity],
    previous: &[String],
    trailer: bool,
    vacated: &mut Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let mut carried = Vec::new();
    for media in previous {
        let (kind, key): (String, String) = tx.query_row(
            "SELECT kind,evidence_key FROM media WHERE id=?1",
            [media],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let shared = tx.query_row("SELECT EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 AND f.id<>?2)", params![media, file], |r| r.get::<_, bool>(0))?;
        let unchanged = identities
            .iter()
            .any(|item| item.kind == kind && item.key == key);
        if !trailer && !shared && !unchanged {
            carried.push((media.clone(), kind));
        }
    }
    let mut linked = Vec::new();
    for item in identities {
        let parent = match &item.parent {
            Some((kind, key)) => tx
                .query_row(
                    "SELECT id FROM media WHERE root_id=?1 AND kind=?2 AND evidence_key=?3",
                    params![root, kind, key],
                    |r| r.get::<_, String>(0),
                )
                .optional()?,
            None => None,
        };
        let existing = tx
            .query_row(
                "SELECT id FROM media WHERE root_id=?1 AND kind=?2 AND evidence_key=?3",
                params![root, item.kind, item.key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let carry = carried
            .iter()
            .position(|(_, kind)| item.playable && kind == &item.kind);
        let media = match (existing, carry) {
            (Some(media), _) => media,
            (None, Some(index)) => {
                let (media, _) = carried.remove(index);
                tx.execute(
                    "UPDATE media SET evidence_key=?2 WHERE id=?1",
                    params![media, item.key],
                )?;
                media
            }
            (None, None) => {
                let media = id();
                tx.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,year,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![media, root, item.kind, parent, item.key, item.title, item.number, item.year, now()])?;
                media
            }
        };
        let (was_parent, was_number): (Option<String>, Option<i64>) = tx.query_row(
            "SELECT parent_id,sort_number FROM media WHERE id=?1",
            [&media],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let moved = was_parent != parent || was_number != item.number;
        if was_parent != parent {
            vacated.extend(was_parent);
        }
        // Episode mappings and track details describe the old place; the next metadata
        // refresh derives them again, so deletion and auto-delete never act on stale ones.
        if moved && matches!(item.kind.as_str(), "episode" | "track") {
            tx.execute(
                "DELETE FROM manager_episode_mappings WHERE media_id=?1",
                [&media],
            )?;
            tx.execute("UPDATE media SET metadata='{}' WHERE id=?1", [&media])?;
        }
        tx.execute("UPDATE media SET title=?2,sort_number=?3,year=?4,parent_id=?5 WHERE id=?1 AND (title IS NOT ?2 OR sort_number IS NOT ?3 OR year IS NOT ?4 OR parent_id IS NOT ?5)", params![media, item.title, item.number, item.year, parent])?;
        if item.playable {
            tx.execute(
                if trailer {
                    "INSERT OR IGNORE INTO local_trailers VALUES (?1,?2)"
                } else {
                    "INSERT OR IGNORE INTO media_sources VALUES (?1,?2)"
                },
                params![media, file],
            )?;
            linked.push(media);
        }
    }
    Ok(linked)
}

/// Deletes entries a scan left with no file and no children, then the parents that leaves
/// empty. Entries an operation or auto-delete refers to stay, along with that history.
fn prune(tx: &rusqlite::Connection, mut vacated: Vec<String>) -> anyhow::Result<()> {
    while let Some(media) = vacated.pop() {
        let empty = tx.query_row("SELECT parent_id FROM media m WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM media_sources WHERE media_id=m.id) AND NOT EXISTS(SELECT 1 FROM local_trailers WHERE media_id=m.id) AND NOT EXISTS(SELECT 1 FROM media WHERE parent_id=m.id) AND NOT EXISTS(SELECT 1 FROM media_operations WHERE media_id=m.id) AND NOT EXISTS(SELECT 1 FROM retention_candidates WHERE media_id=m.id)", [&media], |r| r.get::<_, Option<String>>(0)).optional()?;
        if let Some(parent) = empty {
            tx.execute("DELETE FROM media WHERE id=?1", [&media])?;
            vacated.extend(parent);
        }
    }
    Ok(())
}
