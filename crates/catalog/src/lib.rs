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
/// Identifies a media file from its size and three 1 MiB samples (start, middle and
/// end) rather than every byte, so a first scan never reads whole multi-gigabyte files.
/// Renames keep their identity, and deletion still checks size, modification time and
/// the open handle, which detect any replacement.
pub fn fingerprint(file: &mut std::fs::File) -> std::io::Result<String> {
    use std::io::{Seek, SeekFrom};
    const SAMPLE: u64 = 1 << 20;
    let size = file.metadata()?.len();
    let mut hash = Sha256::new();
    hash.update(size.to_le_bytes());
    let (offsets, length) = if size <= 3 * SAMPLE {
        (vec![0], size)
    } else {
        (vec![0, size / 2 - SAMPLE / 2, size - SAMPLE], SAMPLE)
    };
    let mut buffer = vec![0; usize::try_from(length).unwrap_or(usize::MAX)];
    for offset in offsets {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buffer)?;
        hash.update(&buffer);
    }
    Ok(hex::encode(hash.finalize()))
}
/// One file's scan outcome. A changing or unreadable file keeps any entry it had.
enum Indexed {
    File(Box<IndexedFile>),
    Changing,
    Gone,
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
pub async fn scan(db: &Database, root: Root, tools: thelxinoe_tools::MediaTools) -> Result<usize> {
    scan_with_progress(db, root, tools, |_, _| async { Ok(()) }).await
}
pub async fn scan_with_progress<F, Fut>(
    db: &Database,
    root: Root,
    tools: thelxinoe_tools::MediaTools,
    progress: F,
) -> Result<usize>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    index_with_progress(db, root, tools, progress)
        .await?
        .commit(db)
        .await
}
/// A root that was walked and probed but not written yet. Probing can take minutes, so a
/// caller serializing against deletions holds its lease for `commit` only.
pub struct Scanned {
    root: Root,
    indexed: Vec<IndexedFile>,
    current_paths: std::collections::HashSet<String>,
    retained: std::collections::HashSet<String>,
    warning: Option<String>,
}
impl Scanned {
    /// Write the scan. A file that disappeared since it was probed, for example deleted
    /// while the caller's lease was free, is no longer present.
    pub async fn commit(mut self, db: &Database) -> Result<usize> {
        let mut indexed = Vec::with_capacity(self.indexed.len());
        for file in self.indexed {
            if tokio::fs::try_exists(&file.path).await.unwrap_or(false) {
                indexed.push(file);
            } else {
                self.current_paths.remove(&file.path);
            }
        }
        let mut fingerprint_counts = std::collections::HashMap::new();
        for file in &indexed {
            *fingerprint_counts
                .entry(file.fingerprint.clone())
                .or_insert(0usize) += 1;
        }
        let count = indexed.len();
        storage::scan_with_progress_write_media_files(
            indexed,
            self.current_paths,
            self.retained,
            fingerprint_counts,
            self.warning,
            db,
            self.root,
        )
        .await?;
        Ok(count)
    }
}
/// Walk and probe a root without writing it; see [`Scanned::commit`].
pub async fn index_with_progress<F, Fut>(
    db: &Database,
    root: Root,
    tools: thelxinoe_tools::MediaTools,
    mut progress: F,
) -> Result<Scanned>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    tools.ffprobe.verify_unchanged().await?;
    let base = PathBuf::from(&root.path)
        .canonicalize()
        .context("Library is unavailable")?;
    let walked = base.clone();
    let (entries, unreadable) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut entries = Vec::new();
        // A folder the server can't read, such as lost+found or #recycle, is skipped and
        // keeps what it had. An unreadable root or an unknown location still fails the scan.
        let mut unreadable = Vec::new();
        for entry in walkdir::WalkDir::new(&walked).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => match error.path().filter(|_| error.depth() > 0) {
                    Some(path) => {
                        let reason = error
                            .io_error()
                            .map_or_else(|| error.to_string(), ToString::to_string);
                        unreadable.push((path.to_path_buf(), reason));
                        continue;
                    }
                    None => return Err(error.into()),
                },
            };
            if entry.file_type().is_file() && supported(entry.path()) {
                let path = entry.path().canonicalize()?;
                if !path.starts_with(&walked) {
                    bail!("File escapes library root");
                }
                entries.push(path);
            }
        }
        Ok((entries, unreadable))
    })
    .await??;
    let total = entries.len();
    progress(0, total).await?;
    let mut last_progress = std::time::Instant::now();
    let mut indexed = Vec::new();
    // One unreadable or still-changing file must not block the rest of the library.
    let mut retained = std::collections::HashSet::new();
    let mut failures = Vec::new();
    if !unreadable.is_empty() {
        retained.extend(
            storage::scan_with_progress_read_present_paths(root.id.clone(), db)
                .await?
                .into_iter()
                .filter(|path| {
                    unreadable
                        .iter()
                        .any(|(folder, _)| Path::new(path).starts_with(folder))
                }),
        );
    }
    for (done, path) in entries.into_iter().enumerate() {
        let path_string = path.to_string_lossy().to_string();
        let old = storage::scan_with_progress_read_media_files(path_string.clone(), db).await?;
        match index(&path, &base, old, &root, &tools).await {
            Ok(Indexed::File(file)) => indexed.push(*file),
            Ok(Indexed::Gone) => {}
            Ok(Indexed::Changing) => {
                retained.insert(path_string);
            }
            Err(error) => {
                failures.push(format!("{path_string} ({error:#})"));
                retained.insert(path_string);
            }
        }
        if done + 1 == total || last_progress.elapsed() >= std::time::Duration::from_secs(1) {
            // A library removed during its scan stops probing; the commit checks again.
            if !storage::root_exists(root.id.clone(), db).await? {
                bail!(storage::REMOVED);
            }
            progress(done + 1, total).await?;
            last_progress = std::time::Instant::now();
        }
    }
    let mut current_paths: std::collections::HashSet<_> =
        indexed.iter().map(|f| f.path.clone()).collect();
    current_paths.extend(retained.iter().cloned());
    let counted = |count: usize, noun: &str| {
        (count > 0).then(|| format!("{count} {noun}{}", if count == 1 { "" } else { "s" }))
    };
    let first = unreadable
        .first()
        .map(|(path, error)| format!("{} ({error})", path.display()))
        .or_else(|| failures.first().cloned());
    let warning = first.map(|first| {
        let what = [
            counted(failures.len(), "file"),
            counted(unreadable.len(), "folder"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" and ");
        format!(
            "{what} couldn't be read and will be retried; the rest are up to date. First: {first}"
        )
    });
    Ok(Scanned {
        root,
        indexed,
        current_paths,
        retained,
        warning,
    })
}
async fn index(
    path: &Path,
    base: &Path,
    old: Option<(i64, String, String, String)>,
    root: &Root,
    tools: &thelxinoe_tools::MediaTools,
) -> Result<Indexed> {
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Indexed::Gone),
        Err(error) => return Err(error.into()),
    };
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let size = i64::try_from(metadata.len())?;
    let (hash, probe) = if let Some(old) = old.filter(|o| o.0 == size && o.1 == modified) {
        (old.2, serde_json::from_str(&old.3)?)
    } else {
        let mut command = tokio::process::Command::new(&tools.ffprobe.path);
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
            .arg(path)
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
            .await
            .context("Media probe timed out")??;
        if !output.status.success() {
            bail!("FFprobe could not read it");
        }
        let probe: Value = serde_json::from_slice(&output.stdout)?;
        let hash_path = path.to_path_buf();
        let hash =
            tokio::task::spawn_blocking(move || fingerprint(&mut std::fs::File::open(hash_path)?))
                .await??;
        let after = match tokio::fs::metadata(path).await {
            Ok(after) => after,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Indexed::Gone);
            }
            Err(error) => return Err(error.into()),
        };
        // A copy in progress is indexed once it settles; the next change triggers a scan.
        if after.len() != metadata.len() || after.modified()? != metadata.modified()? {
            return Ok(Indexed::Changing);
        }
        (hash, probe)
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
        path.to_path_buf()
    };
    let (identities, edition) = identity::identify(&root.kind, base, &identity_path, &probe)?;
    Ok(Indexed::File(Box::new(IndexedFile {
        path: path.to_string_lossy().to_string(),
        size,
        modified,
        fingerprint: hash,
        probe,
        identities,
        edition,
    })))
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
