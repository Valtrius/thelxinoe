//! Trusted disposable worker. Mounts and paths are fixed by the controller.
use anyhow::Context;
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_FILES: u64 = 500_000;
/// Restores stage beside the live tree, on its filesystem, then swap entries by rename.
const WORKSPACE: &str = ".thelxinoe-restore";
// Restore journal markers, each durable before the step that follows it.
const STAGED: &str = "staged";
const VACATED: &str = "vacated";
const RETURNING: &str = "returning";
const SWAPPED: &str = "swapped";

/// Copy or restore a snapshot, reporting progress and failures to the controller.
pub fn snapshot(restore: bool) -> anyhow::Result<()> {
    let source = Path::new("/source");
    let filter = Filter::parse(
        source,
        &std::env::var("THELXINOE_SNAPSHOT_DISPOSABLE").unwrap_or_default(),
    );
    reported(|progress| {
        copy_tree(
            source,
            Path::new("/destination"),
            restore,
            &filter,
            progress,
        )
    })
}

/// Paths their owner deletes at startup, such as interrupted tool installs. A stopped
/// install can hold transient links, so copies leave these out rather than fail.
/// Rules are comma-separated `parent/prefix*suffix`, relative to the copied tree.
/// The server deletes these at startup; see `recover_files` in its tools module.
pub const SERVER_DISPOSABLE: &str =
    "tools/.run-*,tools/packages/.stage-*,tools/packages/.seed-*,tools/artifacts/*.part";
#[derive(Default)]
struct Filter {
    root: PathBuf,
    rules: Vec<(PathBuf, String, String)>,
}
impl Filter {
    fn parse(root: &Path, rules: &str) -> Self {
        let rules = rules
            .split(',')
            .filter_map(|rule| {
                let rule = rule.trim();
                let (parent, name) = rule.rsplit_once('/').unwrap_or(("", rule));
                let (prefix, suffix) = name.split_once('*')?;
                (!suffix.contains('*'))
                    .then(|| (PathBuf::from(parent), prefix.to_owned(), suffix.to_owned()))
            })
            .collect();
        Self {
            root: root.to_path_buf(),
            rules,
        }
    }
    fn skips(&self, path: &Path) -> bool {
        let (Ok(relative), Some(name)) = (
            path.strip_prefix(&self.root),
            path.file_name().and_then(|name| name.to_str()),
        ) else {
            return false;
        };
        let parent = relative.parent().unwrap_or(Path::new(""));
        self.rules.iter().any(|(directory, prefix, suffix)| {
            parent == directory
                && name.len() >= prefix.len() + suffix.len()
                && name.starts_with(prefix.as_str())
                && name.ends_with(suffix.as_str())
        })
    }
}

pub fn import_recyclarr() -> anyhow::Result<()> {
    reported(|progress| {
        use sha2::{Digest, Sha256};
        let actual = format!("{:x}", Sha256::digest(fs::read("/source/recyclarr.yml")?));
        anyhow::ensure!(
            std::env::var("THELXINOE_RECYCLARR_IMPORT_HASH")? == actual,
            "Recyclarr configuration changed after review"
        );
        copy_tree(
            Path::new("/source"),
            Path::new("/destination"),
            false,
            &Filter::default(),
            progress,
        )?;
        let metadata = Path::new("/destination/.thelxinoe");
        if metadata.exists() {
            fs::remove_dir_all(metadata)?;
        }
        let path = Path::new("/destination/recyclarr.yml");
        let mut config: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(path)?)?;
        for kind in ["radarr", "sonarr"] {
            for (_, instance) in config
                .get_mut(kind)
                .and_then(|v| v.as_mapping_mut())
                .into_iter()
                .flatten()
            {
                if let Some(map) = instance.as_mapping_mut() {
                    map.insert("api_key".into(), "REDACTED_REGENERATED_AT_SYNC".into());
                }
            }
        }
        fs::write(path, serde_yaml_ng::to_string(&config)?)?;
        Ok(())
    })
}

pub fn remove() -> anyhow::Result<()> {
    let destination = Path::new("/destination");
    anyhow::ensure!(
        fs::symlink_metadata(destination)?.is_dir(),
        "Destination mount missing"
    );
    // Only a controller-selected private appdata directory is mounted here.
    // Removing entries never follows a link into a different filesystem tree.
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::File::open(destination)?.sync_all()?;
    Ok(())
}

fn reported(work: impl FnOnce(&mut Progress) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut progress = Progress::open(Path::new("/progress"));
    let result = work(&mut progress);
    progress.finish(result.as_ref().err().map(|error| format!("{error:#}")));
    result
}

/// Advancing counters let the controller tell a large copy from a stalled one.
struct Progress {
    report: Option<PathBuf>,
    phase: &'static str,
    entries: u64,
    bytes: u64,
    written: Option<Instant>,
}
impl Progress {
    fn open(directory: &Path) -> Self {
        Self {
            report: directory.is_dir().then(|| directory.join("progress.json")),
            phase: "validating",
            entries: 0,
            bytes: 0,
            written: None,
        }
    }
    fn phase(&mut self, phase: &'static str) {
        self.phase = phase;
        self.write(None);
    }
    fn entry(&mut self) {
        self.entries += 1;
        self.tick();
    }
    fn bytes(&mut self, count: usize) {
        self.bytes += count as u64;
        self.tick();
    }
    fn tick(&mut self) {
        if self
            .written
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
        {
            self.write(None);
        }
    }
    fn finish(&mut self, error: Option<String>) {
        self.phase = if error.is_some() {
            "failed"
        } else {
            "complete"
        };
        self.write(error);
    }
    fn write(&mut self, error: Option<String>) {
        self.written = Some(Instant::now());
        let Some(report) = &self.report else {
            return;
        };
        let value = serde_json::json!({"phase":self.phase,"entries":self.entries,
            "bytes":self.bytes,"error":error});
        let temporary = report.with_extension("tmp");
        // Reporting is advisory; it never fails the copy itself.
        if fs::write(&temporary, value.to_string()).is_ok() {
            let _ = fs::rename(&temporary, report);
        }
    }
}

#[derive(Default)]
struct Budget {
    files: u64,
    bytes: u64,
    allocated: u64,
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    restore: bool,
    filter: &Filter,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let mut entries = source_entries(source)?;
    entries.retain(|entry| !filter.skips(entry));
    let mut budget = Budget::default();
    for entry in &entries {
        validate(entry, filter, &mut budget, progress)?;
    }
    anyhow::ensure!(destination.is_dir(), "Destination mount missing");
    if restore {
        return restore_into(source, &entries, destination, filter, &budget, progress);
    }
    anyhow::ensure!(
        fs::read_dir(destination)?.next().is_none(),
        "Snapshot destination is not empty"
    );
    progress.phase("copying");
    copy_entries(&entries, destination, filter, progress)?;
    owner(source, destination)?;
    sync(destination)
}

/// Live configuration can hold the workspace of an interrupted restore; never copy a half-swapped tree.
fn source_entries(source: &Path) -> anyhow::Result<Vec<PathBuf>> {
    anyhow::ensure!(
        fs::symlink_metadata(source)?.is_dir(),
        "Source mount missing"
    );
    let workspace = source.join(WORKSPACE);
    anyhow::ensure!(
        !workspace.join(STAGED).is_file() || workspace.join(SWAPPED).is_file(),
        "An interrupted restore must be recovered before this configuration can be copied"
    );
    children(source, true)
}

/// Stage the complete copy beside live state, then swap top-level entries by rename.
/// Live state stays whole until staging has finished; an interrupted swap is undone first.
fn restore_into(
    source: &Path,
    entries: &[PathBuf],
    destination: &Path,
    filter: &Filter,
    budget: &Budget,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let workspace = destination.join(WORKSPACE);
    progress.phase("recovering");
    recover(destination, &workspace, progress)?;
    // Both copies coexist until the swap completes.
    let needed = budget.allocated.saturating_add(16 << 20);
    if let Ok(available) = fs2::available_space(destination) {
        anyhow::ensure!(
            available >= needed,
            "Restoring needs {} MiB free on the service's storage; {} MiB are available",
            needed >> 20,
            available >> 20
        );
    }
    fs::create_dir(&workspace)?;
    let incoming = workspace.join("incoming");
    fs::create_dir(&incoming)?;
    progress.phase("staging");
    copy_entries(entries, &incoming, filter, progress)
        .context("Could not stage the restored configuration")?;
    sync(&incoming)?;
    mark(&workspace, STAGED)?;
    progress.phase("swapping");
    let outgoing = workspace.join("outgoing");
    fs::create_dir(&outgoing)?;
    for entry in children(destination, true)? {
        rename_into(&entry, &outgoing, progress)?;
    }
    sync(&outgoing)?;
    sync(destination)?;
    mark(&workspace, VACATED)?;
    for entry in children(&incoming, false)? {
        rename_into(&entry, destination, progress)?;
    }
    sync(destination)?;
    mark(&workspace, SWAPPED)?;
    owner(source, destination)?;
    progress.phase("cleaning");
    discard(&workspace, progress)?;
    sync(destination)
}

/// Return to one consistent tree: the originals, unless the swap had completed.
fn recover(destination: &Path, workspace: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    match fs::symlink_metadata(workspace) {
        Ok(meta) => anyhow::ensure!(
            meta.is_dir(),
            "Unexpected restore workspace in the service configuration"
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let marked = |name: &str| workspace.join(name).is_file();
    if marked(STAGED) && !marked(SWAPPED) {
        if marked(VACATED) && !marked(RETURNING) {
            // Every original was moved aside, so live entries are partially restored ones.
            for entry in children(destination, true)? {
                remove_tree(&entry, progress)?;
            }
            sync(destination)?;
            mark(workspace, RETURNING)?;
        }
        let outgoing = workspace.join("outgoing");
        if fs::symlink_metadata(&outgoing).is_ok_and(|meta| meta.is_dir()) {
            for entry in children(&outgoing, false)? {
                rename_into(&entry, destination, progress)?;
            }
            sync(destination)?;
        }
    }
    discard(workspace, progress)?;
    sync(destination)
}

/// Drop the staged marker first, so an interrupted cleanup never looks like an unfinished swap.
fn discard(workspace: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    match fs::remove_file(workspace.join(STAGED)) {
        Ok(()) => sync(workspace)?,
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_tree(workspace, progress)
}

fn mark(workspace: &Path, name: &str) -> anyhow::Result<()> {
    let mut file = fs::File::create(workspace.join(name))?;
    file.write_all(b"1")?;
    file.sync_all()?;
    sync(workspace)
}

fn rename_into(entry: &Path, directory: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    let target = directory.join(
        entry
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid appdata entry"))?,
    );
    anyhow::ensure!(
        fs::symlink_metadata(&target).is_err(),
        "Restore found a conflicting configuration entry"
    );
    fs::rename(entry, target)?;
    progress.entry();
    Ok(())
}

/// Never follows links: a link is removed, not its target.
fn remove_tree(path: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            remove_tree(&entry?.path(), progress)?;
        }
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    progress.entry();
    Ok(())
}

fn children(directory: &Path, skip_workspace: bool) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !(skip_workspace && entry.file_name() == WORKSPACE) {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

fn sync(directory: &Path) -> anyhow::Result<()> {
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn owner(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(source)?;
    std::os::unix::fs::chown(destination, Some(meta.uid()), Some(meta.gid()))?;
    fs::set_permissions(destination, fs::Permissions::from_mode(meta.mode() & 0o777))?;
    Ok(())
}

/// Name an entry relative to its worker mount, so errors point at the service's own file.
fn shown(path: &Path) -> String {
    ["/source", "/destination"]
        .iter()
        .find_map(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}
fn validate(
    path: &Path,
    filter: &Filter,
    budget: &mut Budget,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let link = meta.file_type().is_symlink();
    anyhow::ensure!(
        meta.is_dir() || meta.is_file() || link,
        "Unsupported appdata entry: {}",
        shown(path)
    );
    let counted = if link {
        let target = sibling_link(path).with_context(|| format!("Can't copy {}", shown(path)))?;
        fs::symlink_metadata(path.parent().unwrap().join(target))?
    } else {
        meta.clone()
    };
    anyhow::ensure!(
        meta.nlink() == 1 || meta.is_dir(),
        "Hard-linked appdata is unsupported: {}",
        shown(path)
    );
    budget.files += 1;
    budget.bytes = budget
        .bytes
        .checked_add(counted.len())
        .ok_or_else(|| anyhow::anyhow!("Appdata too large"))?;
    budget.allocated = budget.allocated.saturating_add(counted.blocks() * 512);
    anyhow::ensure!(
        budget.files <= MAX_FILES && budget.bytes <= MAX_BYTES,
        "Appdata snapshot limit exceeded"
    );
    progress.entry();
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?.path();
            if !filter.skips(&entry) {
                validate(&entry, filter, budget, progress)?;
            }
        }
    }
    Ok(())
}

// Rotating loggers (including Seerr) create links to a file in the same folder.
// Materialize those files so snapshots also fit the link-free backup format.
// Reject directories, chains,
// absolute paths, traversal, and missing targets before copying any data.
fn sibling_link(path: &Path) -> anyhow::Result<PathBuf> {
    let target = fs::read_link(path)?;
    let mut parts = target.components();
    anyhow::ensure!(
        matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none(),
        "Appdata link must target a sibling file"
    );
    let sibling = fs::symlink_metadata(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("Missing link parent"))?
            .join(&target),
    )?;
    anyhow::ensure!(
        sibling.is_file() && sibling.nlink() == 1,
        "Appdata link must target a regular file"
    );
    Ok(target)
}
fn copy_entries(
    entries: &[PathBuf],
    destination: &Path,
    filter: &Filter,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    for entry in entries {
        let name = entry
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid appdata entry"))?;
        copy(entry, &destination.join(name), filter, progress)?;
    }
    Ok(())
}
fn copy(
    source: &Path,
    destination: &Path,
    filter: &Filter,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(source)?;
    let sibling;
    let source = if meta.file_type().is_symlink() {
        sibling = source.parent().unwrap().join(sibling_link(source)?);
        sibling.as_path()
    } else {
        source
    };
    let meta = fs::symlink_metadata(source)?;
    if meta.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if !filter.skips(&entry.path()) {
                copy(
                    &entry.path(),
                    &destination.join(entry.file_name()),
                    filter,
                    progress,
                )?;
            }
        }
    } else {
        anyhow::ensure!(
            meta.is_file() && meta.nlink() == 1,
            "Appdata changed during copy"
        );
        copy_file(source, destination, progress)?;
    }
    std::os::unix::fs::chown(destination, Some(meta.uid()), Some(meta.gid()))?;
    fs::set_permissions(destination, fs::Permissions::from_mode(meta.mode() & 0o777))?;
    if meta.is_dir() {
        sync(destination)?;
    }
    progress.entry();
    Ok(())
}
/// Chunked so a single large database still shows progress.
fn copy_file(source: &Path, destination: &Path, progress: &mut Progress) -> anyhow::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        output.write_all(&buffer[..read])?;
        progress.bytes(read);
    }
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn quiet() -> Progress {
        Progress::open(Path::new("/nonexistent-progress"))
    }
    fn tree(root: &Path, files: &[(&str, &str)]) {
        for (path, content) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }
    fn contents(root: &Path) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        fs::read_to_string(&path).unwrap(),
                    ));
                }
            }
        }
        found.sort();
        found
    }
    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut pairs: Vec<_> = pairs
            .iter()
            .map(|(path, content)| (path.to_string(), content.to_string()))
            .collect();
        pairs.sort();
        pairs
    }
    #[test]
    fn rejects_links_and_preserves_regular_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        fs::create_dir_all(root.join("source")).unwrap();
        fs::create_dir_all(root.join("copy")).unwrap();
        fs::write(root.join("source/db"), "consistent state").unwrap();
        copy_tree(
            &root.join("source"),
            &root.join("copy"),
            false,
            &Filter::default(),
            &mut quiet(),
        )
        .unwrap();
        assert_eq!(fs::read(root.join("copy/db")).unwrap(), b"consistent state");
        std::os::unix::fs::symlink("/etc", root.join("source/escape")).unwrap();
        assert!(
            validate(
                &root.join("source"),
                &Filter::default(),
                &mut Budget::default(),
                &mut quiet()
            )
            .is_err()
        );
    }
    #[test]
    fn snapshots_logger_links_without_following_other_links() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("dated.log"), "log content").unwrap();
        std::os::unix::fs::symlink("dated.log", source.join("current.log")).unwrap();
        validate(
            &source,
            &Filter::default(),
            &mut Budget::default(),
            &mut quiet(),
        )
        .unwrap();
        copy(
            &source,
            &root.join("copy"),
            &Filter::default(),
            &mut quiet(),
        )
        .unwrap();
        assert!(
            fs::symlink_metadata(root.join("copy/current.log"))
                .unwrap()
                .is_file()
        );
        assert_eq!(
            fs::read(root.join("copy/current.log")).unwrap(),
            b"log content"
        );
        for target in ["/etc/passwd", "../outside", "missing", "current.log", "."] {
            std::os::unix::fs::symlink(target, source.join("invalid")).unwrap();
            assert!(
                validate(
                    &source,
                    &Filter::default(),
                    &mut Budget::default(),
                    &mut quiet()
                )
                .is_err(),
                "accepted {target}"
            );
            fs::remove_file(source.join("invalid")).unwrap();
        }
    }
    #[test]
    fn restore_replaces_live_state_and_leaves_no_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let (source, live) = (directory.path().join("s"), directory.path().join("l"));
        tree(&source, &[("db", "archived"), ("logs/a.txt", "old log")]);
        tree(&live, &[("db", "current"), ("added-later", "x")]);
        let report = directory.path().join("progress");
        fs::create_dir(&report).unwrap();
        let mut progress = Progress::open(&report);
        copy_tree(&source, &live, true, &Filter::default(), &mut progress).unwrap();
        progress.finish(None);
        assert_eq!(
            contents(&live),
            owned(&[("db", "archived"), ("logs/a.txt", "old log")])
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(report.join("progress.json")).unwrap()).unwrap();
        assert_eq!(report["phase"], "complete");
        assert!(report["entries"].as_u64().unwrap() > 0);
    }
    #[test]
    fn interrupted_restores_recover_one_consistent_tree() {
        let originals = [("db", "current"), ("MediaCover/1.jpg", "cover")];
        let restored = [("db", "archived"), ("Backups/a.zip", "zip")];
        // Each case reproduces the disk state left at one interruption point.
        for (case, markers, expected) in [
            ("staging", &[][..], &originals[..]),
            ("vacating", &[STAGED][..], &originals[..]),
            ("filling", &[STAGED, VACATED][..], &originals[..]),
            (
                "returning",
                &[STAGED, VACATED, RETURNING][..],
                &originals[..],
            ),
            ("cleaning", &[STAGED, VACATED, SWAPPED][..], &restored[..]),
            ("discarding", &[VACATED, SWAPPED][..], &restored[..]),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let live = directory.path().join("live");
            let workspace = live.join(WORKSPACE);
            fs::create_dir_all(workspace.join("outgoing")).unwrap();
            for marker in markers {
                fs::write(workspace.join(marker), "1").unwrap();
            }
            match case {
                // A partial copy beside untouched originals.
                "staging" => {
                    tree(&live, &originals);
                    tree(&workspace.join("incoming"), &restored[..1]);
                }
                // Part of the originals moved aside.
                "vacating" => {
                    tree(&live, &originals[..1]);
                    tree(&workspace.join("outgoing"), &originals[1..]);
                    tree(&workspace.join("incoming"), &restored);
                }
                // All originals aside, part of the restored entries moved in.
                "filling" => {
                    tree(&live, &restored[..1]);
                    tree(&workspace.join("outgoing"), &originals);
                    tree(&workspace.join("incoming"), &restored[1..]);
                }
                // Partial entries already removed, part of the originals returned.
                "returning" => {
                    tree(&live, &originals[..1]);
                    tree(&workspace.join("outgoing"), &originals[1..]);
                }
                // The swap finished; only the obsolete originals remain aside.
                _ => {
                    tree(&live, &restored);
                    tree(&workspace.join("outgoing"), &originals[1..]);
                }
            }
            recover(&live, &workspace, &mut quiet()).unwrap();
            assert_eq!(contents(&live), owned(expected), "{case}");
            assert!(!workspace.exists(), "{case}");
        }
    }
    #[test]
    fn snapshots_refuse_a_half_swapped_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let (live, copy) = (directory.path().join("live"), directory.path().join("c"));
        tree(
            &live,
            &[("db", "current"), (".thelxinoe-restore/incoming/db", "x")],
        );
        fs::create_dir(&copy).unwrap();
        // A workspace that never finished staging is skipped.
        copy_tree(&live, &copy, false, &Filter::default(), &mut quiet()).unwrap();
        assert_eq!(contents(&copy), owned(&[("db", "current")]));
        fs::write(live.join(WORKSPACE).join(STAGED), "1").unwrap();
        fs::remove_dir_all(&copy).unwrap();
        fs::create_dir(&copy).unwrap();
        assert!(copy_tree(&live, &copy, false, &Filter::default(), &mut quiet()).is_err());
    }
    #[test]
    fn insufficient_space_leaves_live_state_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let (source, live) = (directory.path().join("s"), directory.path().join("l"));
        tree(&source, &[("db", "archived")]);
        tree(&live, &[("db", "current")]);
        let budget = Budget {
            allocated: u64::MAX / 2,
            ..Budget::default()
        };
        let entries = children(&source, true).unwrap();
        let error = restore_into(
            &source,
            &entries,
            &live,
            &Filter::default(),
            &budget,
            &mut quiet(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("MiB free"), "{error}");
        assert_eq!(contents(&live), owned(&[("db", "current")]));
        assert!(!live.join(WORKSPACE).exists());
    }
    #[test]
    fn server_snapshots_skip_interrupted_tool_installs() {
        let directory = tempfile::tempdir().unwrap();
        let (live, copy) = (directory.path().join("live"), directory.path().join("c"));
        tree(
            &live,
            &[
                ("thelxinoe.sqlite3", "db"),
                ("tools/packages/0123/ffmpeg", "installed"),
                ("tools/packages/.stage-1/env/lib/site.py", "partial"),
                ("tools/artifacts/ffmpeg.tar.xz.part", "partial"),
                ("tools/artifacts/ffmpeg.tar.xz", "complete"),
                // Only the listed parent is disposable.
                ("other/.stage-2/kept", "kept"),
            ],
        );
        // A stopped venv install leaves its lib64 alias to a directory.
        std::os::unix::fs::symlink("lib", live.join("tools/packages/.stage-1/env/lib64")).unwrap();
        fs::create_dir(&copy).unwrap();
        let error = copy_tree(&live, &copy, false, &Filter::default(), &mut quiet()).unwrap_err();
        assert!(
            format!("{error:#}").contains("tools/packages/.stage-1/env/lib64"),
            "{error:#}"
        );
        fs::remove_dir_all(&copy).unwrap();
        fs::create_dir(&copy).unwrap();
        let filter = Filter::parse(&live, SERVER_DISPOSABLE);
        copy_tree(&live, &copy, false, &filter, &mut quiet()).unwrap();
        assert_eq!(
            contents(&copy),
            owned(&[
                ("other/.stage-2/kept", "kept"),
                ("thelxinoe.sqlite3", "db"),
                ("tools/artifacts/ffmpeg.tar.xz", "complete"),
                ("tools/packages/0123/ffmpeg", "installed"),
            ])
        );
    }
}
