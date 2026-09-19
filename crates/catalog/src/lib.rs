mod identity;
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use thelxinoe_core::{id, now};
use thelxinoe_database::Database;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Root {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub last_scan: Option<i64>,
    pub scan_error: Option<String>,
}
pub async fn roots(db: &Database) -> Result<Vec<Root>> {
    db.call(|c| {
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
pub fn approved_path(path: &Path, allowed: &Path) -> Result<PathBuf> {
    let root = allowed.canonicalize()?;
    let candidate = path.canonicalize()?;
    if !candidate.starts_with(&root) || !candidate.is_dir() {
        bail!("Library must be a directory inside the configured media mount");
    }
    Ok(candidate)
}
struct IndexedFile {
    path: String,
    size: i64,
    modified: String,
    fingerprint: String,
    probe: Value,
    identities: Vec<identity::Identity>,
    edition: String,
}
pub async fn scan(db: &Database, root: Root) -> Result<usize> {
    scan_with_progress(db, root, |_, _| async { Ok(()) }).await
}
pub async fn scan_with_progress<F, Fut>(db: &Database, root: Root, mut progress: F) -> Result<usize>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let base = PathBuf::from(&root.path)
        .canonicalize()
        .context("Library is unavailable")?;
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&base).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_file() && supported(entry.path()) {
                let path = entry.path().canonicalize()?;
                if !path.starts_with(&base) {
                    bail!("File escapes library root");
                }
                entries.push(path);
            }
        }
        Ok(entries)
    })
    .await??;
    let total = entries.len();
    progress(0, total).await?;
    let mut last_progress = std::time::Instant::now();
    let mut indexed = Vec::new();
    for path in entries {
        let path_string = path.to_string_lossy().to_string();
        let metadata = tokio::fs::metadata(&path).await?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        let size = i64::try_from(metadata.len())?;
        let lookup = path_string.clone();
        let old = db
            .call(move |c| {
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
            .await?;
        let (fingerprint, probe) = if let Some(old) = old.filter(|o| o.0 == size && o.1 == modified)
        {
            (old.2, serde_json::from_str(&old.3)?)
        } else {
            let mut command = tokio::process::Command::new("ffprobe");
            command
                .args([
                    "-v",
                    "error",
                    "-show_format",
                    "-show_streams",
                    "-show_chapters",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
                .await
                .context("Media probe timed out")??;
            if !output.status.success() {
                bail!("FFprobe could not read a media file; scan kept the previous catalog");
            }
            let probe: Value = serde_json::from_slice(&output.stdout)?;
            let hash_path = path.clone();
            let fingerprint = tokio::task::spawn_blocking(move || -> Result<String> {
                let mut file = std::fs::File::open(hash_path)?;
                let mut hash = Sha256::new();
                let mut buf = [0u8; 128 * 1024];
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buf[..n]);
                }
                Ok(hex::encode(hash.finalize()))
            })
            .await??;
            let after = tokio::fs::metadata(&path).await?;
            if after.len() != metadata.len() || after.modified()? != metadata.modified()? {
                bail!("File changed during scan; retry after the copy finishes");
            }
            (fingerprint, probe)
        };
        let mut probe = probe;
        let trailer_suffix = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.to_ascii_lowercase().ends_with("-trailer"));
        let identity_path = if root.kind == "movies"
            && let Some(stem) = trailer_suffix
        {
            probe["thelxinoe_local_trailer"] = serde_json::json!(true);
            path.with_file_name(format!(
                "{}.{}",
                &stem[..stem.len() - 8],
                path.extension().and_then(|s| s.to_str()).unwrap_or("mp4")
            ))
        } else {
            path.clone()
        };
        let (identities, edition) = identity::identify(&root.kind, &identity_path, &probe)?;
        indexed.push(IndexedFile {
            path: path_string,
            size,
            modified,
            fingerprint,
            probe,
            identities,
            edition,
        });
        if indexed.len() == total || last_progress.elapsed() >= std::time::Duration::from_secs(1) {
            progress(indexed.len(), total).await?;
            last_progress = std::time::Instant::now();
        }
    }
    let count = indexed.len();
    let current_paths: std::collections::HashSet<_> =
        indexed.iter().map(|f| f.path.clone()).collect();
    let mut fingerprint_counts = std::collections::HashMap::new();
    for file in &indexed {
        *fingerprint_counts
            .entry(file.fingerprint.clone())
            .or_insert(0usize) += 1;
    }
    db.call(move|c|{
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
                for (provider,external_id) in item.provider_ids{tx.execute("INSERT OR IGNORE INTO provider_ids(media_id,provider,external_id,mapping_state) VALUES (?1,?2,?3,'exact')",params![media_id,provider,external_id])?;}
            }
        }
        tx.execute("UPDATE library_roots SET last_scan=?1,scan_error=NULL WHERE id=?2",params![now(),root.id])?;tx.commit()?;Ok(())
    }).await?;
    Ok(count)
}
fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mp4"
                | "mkv"
                | "avi"
                | "mov"
                | "m4v"
                | "webm"
                | "mp3"
                | "flac"
                | "m4a"
                | "ogg"
                | "opus"
                | "wav"
                | "aac"
        )
    })
}
