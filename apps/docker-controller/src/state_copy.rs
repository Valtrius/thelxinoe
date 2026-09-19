//! Trusted disposable worker. Mounts and paths are fixed by the controller.
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

const MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_FILES: u64 = 500_000;

pub fn run(restore: bool) -> anyhow::Result<()> {
    let source = Path::new("/source");
    let destination = Path::new("/destination");
    let mut budget = (0, 0);
    validate(source, &mut budget)?;
    anyhow::ensure!(destination.is_dir(), "Destination mount missing");
    if restore {
        // The controller mounts only a stopped, verified managed service's config here.
        // remove_dir_all does not follow symlinks; never join user-supplied paths.
        for entry in fs::read_dir(destination)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    } else {
        anyhow::ensure!(
            fs::read_dir(destination)?.next().is_none(),
            "Snapshot destination is not empty"
        );
    }
    copy(source, destination)?;
    fs::File::open(destination)?.sync_all()?;
    Ok(())
}

fn validate(path: &Path, budget: &mut (u64, u64)) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    anyhow::ensure!(meta.is_dir() || meta.is_file(), "Unsupported appdata entry");
    anyhow::ensure!(
        meta.nlink() == 1 || meta.is_dir(),
        "Hard-linked appdata is unsupported"
    );
    budget.0 += 1;
    budget.1 = budget
        .1
        .checked_add(meta.len())
        .ok_or_else(|| anyhow::anyhow!("Appdata too large"))?;
    anyhow::ensure!(
        budget.0 <= MAX_FILES && budget.1 <= MAX_BYTES,
        "Appdata snapshot limit exceeded"
    );
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            validate(&entry?.path(), budget)?;
        }
    }
    Ok(())
}
fn copy(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(source)?;
    if meta.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        anyhow::ensure!(
            meta.is_file() && meta.nlink() == 1,
            "Appdata changed during copy"
        );
        fs::copy(source, destination)?;
        fs::File::open(destination)?.sync_all()?;
    }
    std::os::unix::fs::chown(destination, Some(meta.uid()), Some(meta.gid()))?;
    fs::set_permissions(destination, fs::Permissions::from_mode(meta.mode() & 0o777))?;
    if meta.is_dir() {
        fs::File::open(destination)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_links_and_preserves_regular_content() {
        let root = std::env::temp_dir().join(thelxinoe_core::id());
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(root.join("source/db"), "consistent state").unwrap();
        validate(&root.join("source"), &mut (0, 0)).unwrap();
        copy(&root.join("source"), &root.join("copy")).unwrap();
        assert_eq!(fs::read(root.join("copy/db")).unwrap(), b"consistent state");
        std::os::unix::fs::symlink("/etc", root.join("source/escape")).unwrap();
        assert!(validate(&root.join("source"), &mut (0, 0)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
