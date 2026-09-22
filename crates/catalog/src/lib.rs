#[path = "storage.rs"]
mod storage;

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
    storage::roots(db).await
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
        let old = storage::scan_with_progress_read_media_files(lookup, db).await?;
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
    storage::scan_with_progress_write_media_files(
        indexed,
        current_paths,
        fingerprint_counts,
        db,
        root,
    )
    .await?;
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
