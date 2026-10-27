//! Embedded text subtitles, extracted once per file version into the cache. FFmpeg reads the
//! whole file to reach a subtitle track, minutes for a large file on a slow disk, so one pass
//! writes every text track of the file, and a background worker extracts the library ahead
//! of playback.

#[path = "storage/subtitles.rs"]
mod storage;
#[cfg(test)]
#[path = "subtitles_tests.rs"]
mod tests;

use crate::AppState;
use anyhow::Context;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    sync::LazyLock,
    time::Duration,
};
use thelxinoe_core::resource_locks::ResourceLocks;
use thelxinoe_playback::{Source, Track};
use tokio::sync::Semaphore;

/// Whole files read at once, by requests and the worker together.
static PASSES: Semaphore = Semaphore::const_new(2);
/// One extraction per file version.
static LOCKS: LazyLock<ResourceLocks> = LazyLock::new(ResourceLocks::default);
const COMPLETE: &str = "complete";
const FAILED: &str = "failed";
const PARTIAL: &str = ".partial-";
/// A failed extraction is attempted again after this long. Unused extractions and those a
/// crash left half written are removed after it too.
const DAY: Duration = Duration::from_secs(86400);
const SPACE: u64 = 512 * 1024 * 1024;

fn root(state: &AppState) -> PathBuf {
    state.config.cache.join("subtitles")
}
fn name(source: &str, generation: &str) -> String {
    thelxinoe_auth::digest(&format!("{source}:{generation}"))
}
fn entry(state: &AppState, source: &Source) -> PathBuf {
    root(state).join(name(&source.id, &source.generation))
}
fn embedded(track: &Track) -> bool {
    track.kind == "subtitle" && track.supported && track.path.is_none() && track.index.is_some()
}
/// The cached file of a track and the encoder that writes it. Styled and WebVTT tracks keep
/// their format; the others become SRT.
fn output(track: &Track) -> (String, &'static str) {
    let index = track.index.unwrap_or_default();
    match track.codec.as_str() {
        "ass" | "ssa" => (format!("{index}.ass"), "ass"),
        "webvtt" => (format!("{index}.vtt"), "webvtt"),
        _ => (format!("{index}.srt"), "srt"),
    }
}
/// A file read at no less than 4 MiB/s, plus a minute.
fn limit(size: u64) -> Duration {
    Duration::from_secs(60 + size / (4 * 1024 * 1024))
}

/// The cached copy of an embedded text track of `source`. The first request for a file version
/// extracts all of its text tracks, and that extraction completes even if the client stops
/// waiting.
pub(crate) async fn track(
    state: &AppState,
    source: &Source,
    track: &Track,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(embedded(track), "Not an embedded text subtitle");
    let task = {
        let state = state.clone();
        let source = source.clone();
        tokio::spawn(async move { ensure(&state, &source, false).await })
    };
    let dir = task
        .await??
        .context("Subtitle extraction was interrupted")?;
    let path = dir.join(output(track).0);
    anyhow::ensure!(
        tokio::fs::try_exists(&path).await?,
        "The extraction has no copy of this track"
    );
    Ok(path)
}

/// Whether the extraction in `dir` is complete, or failed within a day.
async fn settled(dir: &Path) -> Option<bool> {
    if tokio::fs::try_exists(dir.join(COMPLETE)).await.ok()? {
        return Some(true);
    }
    let failed = tokio::fs::metadata(dir.join(FAILED)).await.ok()?;
    let age = failed.modified().ok()?.elapsed().unwrap_or_default();
    (age < DAY).then_some(false)
}

/// The directory of this file version's extracted tracks, extracting them first when needed.
/// A background extraction yields to playback: it stops, keeps nothing and returns `None`.
async fn ensure(
    state: &AppState,
    source: &Source,
    background: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let dir = entry(state, source);
    let key = dir.to_string_lossy().into_owned();
    let _lock = LOCKS.write(&[&key]).await;
    match settled(&dir).await {
        Some(true) => return Ok(Some(dir)),
        Some(false) => anyhow::bail!("The last extraction of this file failed less than a day ago"),
        None => {}
    }
    let _pass = PASSES.acquire().await?;
    let tools = state.tools.runtime.media()?;
    let root = root(state);
    tokio::fs::create_dir_all(&root).await?;
    anyhow::ensure!(
        fs2::available_space(&root)? >= SPACE,
        "Insufficient subtitle cache space"
    );
    let partial = tempfile::Builder::new().prefix(PARTIAL).tempdir_in(&root)?;
    match extract(state, source, &tools, partial.path(), background).await {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => {
            // A failure is recorded so the file isn't read again before it is retried.
            let marker = async {
                if tokio::fs::try_exists(&dir).await? {
                    tokio::fs::remove_dir_all(&dir).await?;
                }
                tokio::fs::create_dir_all(&dir).await?;
                tokio::fs::write(dir.join(FAILED), error.to_string()).await
            };
            if let Err(marker) = marker.await {
                tracing::warn!("Failed subtitle extraction could not be recorded: {marker:#}");
            }
            return Err(error);
        }
    }
    tokio::fs::write(partial.path().join(COMPLETE), []).await?;
    if tokio::fs::try_exists(&dir).await? {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    tokio::fs::rename(partial.keep(), &dir).await?;
    Ok(Some(dir))
}

/// Reads the file once and writes each of its text tracks to `dir`. Returns false when it
/// stopped for playback.
async fn extract(
    state: &AppState,
    source: &Source,
    tools: &thelxinoe_tools::MediaTools,
    dir: &Path,
    background: bool,
) -> anyhow::Result<bool> {
    let tracks = source
        .tracks()
        .await?
        .into_iter()
        .filter(embedded)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !tracks.is_empty(),
        "The file has no embedded text subtitles"
    );
    source.validate().await?;
    tools.ffmpeg.verify_unchanged().await?;
    #[cfg(unix)]
    let mut command = {
        let mut c = tokio::process::Command::new("nice");
        c.args(["-n", "15"]).arg(&tools.ffmpeg.path);
        c
    };
    #[cfg(not(unix))]
    let mut command = tokio::process::Command::new(&tools.ffmpeg.path);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&source.path);
    for track in &tracks {
        let (file, encoder) = output(track);
        command
            .args(["-map", &format!("0:{}", track.index.unwrap_or_default())])
            .args(["-c:s", encoder, "-f", encoder])
            .arg(dir.join(file));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let deadline = tokio::time::sleep(limit(source.size));
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            status = child.wait() => {
                anyhow::ensure!(status?.success(), "FFmpeg could not extract the subtitles");
                break;
            }
            _ = &mut deadline => anyhow::bail!("Subtitle extraction timed out"),
            _ = check.tick(), if background => {
                if !crate::playback::idle(state).await? {
                    return Ok(false);
                }
            }
        }
    }
    // A file replaced while it was read gives a new version; this one is never used.
    source.validate().await?;
    Ok(true)
}

/// Extracts the subtitles of library files, the most recently added or changed first, one
/// file at a time while nothing plays.
pub async fn run(state: AppState) -> anyhow::Result<()> {
    state.tools.wait_ready().await;
    loop {
        let more = next(&state).await?;
        tokio::time::sleep(Duration::from_secs(if more { 5 } else { 60 })).await;
    }
}

/// Extracts the first library file that has none yet. Returns false when no file is left,
/// or none can be extracted for now.
async fn next(state: &AppState) -> anyhow::Result<bool> {
    if !crate::playback::idle(state).await? {
        return Ok(true);
    }
    let root = root(state);
    for (media, file, generation) in storage::candidates(&state.db).await? {
        let dir = root.join(name(&file, &generation));
        if settled(&dir).await.is_some() {
            continue;
        }
        let source = match crate::playback::source(state, &media, Some(&file)).await {
            Ok(source) if source.generation == generation => source,
            // Changed since it was listed; the next turn lists it again.
            _ => return Ok(true),
        };
        return match ensure(state, &source, true).await {
            Ok(_) => Ok(true),
            Err(error) => {
                tracing::warn!(%file, "Subtitle extraction failed: {error:#}");
                // A failure that wasn't recorded, such as missing space or tools, would stop
                // at the same file again; wait before the next attempt.
                Ok(settled(&dir).await.is_some())
            }
        };
    }
    Ok(false)
}

/// Removes, after a day, the extractions of file versions that are gone and those a crash
/// left half written.
pub async fn prune(state: &AppState) -> anyhow::Result<()> {
    let keep = storage::current(&state.db)
        .await?
        .into_iter()
        .map(|(source, generation)| name(&source, &generation))
        .collect();
    sweep(&root(state), &keep, DAY).await
}
async fn sweep(root: &Path, keep: &HashSet<String>, age: Duration) -> anyhow::Result<()> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(item) = entries.next_entry().await? {
        let meta = item.metadata().await?;
        let old = meta.modified()?.elapsed().unwrap_or_default() >= age;
        if old && !keep.contains(&*item.file_name().to_string_lossy()) {
            if meta.is_dir() {
                tokio::fs::remove_dir_all(item.path()).await?;
            } else {
                tokio::fs::remove_file(item.path()).await?;
            }
        }
    }
    Ok(())
}
