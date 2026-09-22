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

pub(super) async fn scan_with_progress_write_media_files(
    indexed: Vec<IndexedFile>,
    current_paths: std::collections::HashSet<String>,
    fingerprint_counts: std::collections::HashMap<String, usize>,
    db: &Database,
    root: Root,
) -> anyhow::Result<()> {
    db.write("catalog.scan_with_progress_write_media_files", move|c|{
        let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let missing:Vec<(String,String)>=tx.prepare("SELECT id,path,fingerprint FROM media_files WHERE root_id=?1 AND present=1")?.query_map([&root.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?.into_iter().filter(|(_,path,_)|!current_paths.contains(path)).map(|(id,_,hash)|(id,hash)).collect();
        tx.execute("UPDATE media_files SET present=0 WHERE root_id=?1",[&root.id])?;
        for file in indexed {
            let old=tx.query_row("SELECT id,fingerprint,generation FROM media_files WHERE path=?1",[&file.path],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
            let file_id=old.as_ref().map(|o|o.0.clone()).unwrap_or_else(id);
            let generation=old.as_ref().filter(|o|o.1==file.fingerprint).map(|o|o.2.clone()).unwrap_or_else(id);
            // A unique exact-content move preserves logical identity. Ambiguous identical files
            // are kept distinct instead of merging unrelated entries by a guessed title.
            let candidates:Vec<_>=missing.iter().filter(|(_,hash)|hash==&file.fingerprint).collect();
            let source_file=if old.is_some(){Some(&file_id)}else if candidates.len()==1 && fingerprint_counts.get(&file.fingerprint)==Some(&1){Some(&candidates[0].0)}else{None};
            let moved:Vec<String>=if let Some(source)=source_file{tx.prepare("SELECT media_id FROM media_sources WHERE file_id=?1")?.query_map([source],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?}else{Vec::new()};
            tx.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,edition,present,scanned_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10) ON CONFLICT(path) DO UPDATE SET generation=excluded.generation,size=excluded.size,modified=excluded.modified,fingerprint=excluded.fingerprint,probe=excluded.probe,edition=excluded.edition,present=1,scanned_at=excluded.scanned_at,ownership=CASE WHEN generation=excluded.generation THEN ownership ELSE 'unresolved' END",params![file_id,root.id,file.path,generation,file.size,file.modified,file.fingerprint,file.probe.to_string(),file.edition,now()])?;
            tx.execute("DELETE FROM media_sources WHERE file_id=?1",[&file_id])?;
            tx.execute("DELETE FROM local_trailers WHERE file_id=?1",[&file_id])?;
            if !moved.is_empty() {for media_id in moved {tx.execute("INSERT OR IGNORE INTO media_sources VALUES (?1,?2)",params![media_id,file_id])?;}continue;}
            for item in file.identities {
                let parent=if let Some((kind,key))=&item.parent{tx.query_row("SELECT id FROM media WHERE root_id=?1 AND kind=?2 AND evidence_key=?3",params![root.id,kind,key],|r|r.get::<_,String>(0)).optional()?}else{None};
                tx.execute("INSERT OR IGNORE INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,year,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id(),root.id,item.kind,parent,item.key,item.title,item.number,item.year,now()])?;
                let media_id:String=tx.query_row("SELECT id FROM media WHERE root_id=?1 AND kind=?2 AND evidence_key=?3",params![root.id,item.kind,item.key],|r|r.get(0))?;
                if item.playable{if file.probe["thelxinoe_local_trailer"].as_bool()==Some(true){tx.execute("INSERT OR IGNORE INTO local_trailers VALUES (?1,?2)",params![media_id,file_id])?;}else{tx.execute("INSERT OR IGNORE INTO media_sources VALUES (?1,?2)",params![media_id,file_id])?;}}
            }
        }
        tx.execute("UPDATE library_roots SET last_scan=?1,scan_error=NULL WHERE id=?2",params![now(),root.id])?;tx.commit()?;Ok(())
    }).await
}
