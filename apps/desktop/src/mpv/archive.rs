use anyhow::{Result, ensure};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

// Ported from YouTwitch's bounded archive importer, limited here to MPV's 7z builds.
fn relative(name: &str) -> Result<PathBuf> {
    let normalized = name.trim_end_matches(['/', '\\']).replace('\\', "/");
    let path = PathBuf::from(&normalized);
    ensure!(
        !normalized.is_empty()
            && !normalized.starts_with('/')
            && !normalized.contains([':', '\0'])
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe archive path"
    );
    for part in normalized.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !part.is_empty()
                && !part.ends_with(['.', ' '])
                && !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && base.as_bytes()[3].is_ascii_digit()),
            "Reserved archive path"
        );
    }
    Ok(path)
}
pub fn extract(package: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut archive = sevenz_rust2::ArchiveReader::open(package, sevenz_rust2::Password::empty())?;
    archive.for_each_entries(|entry, reader| {
        let result = (|| -> Result<()> {
            ensure!(
                entry.windows_attributes() & 0x400 == 0,
                "Archive reparse points are unsupported"
            );
            let path = relative(entry.name())?;
            ensure!(
                seen.len() < 30000 && seen.insert(path.to_string_lossy().to_lowercase()),
                "Duplicate archive path or too many files"
            );
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
            ensure!(
                total <= 2 * 1024 * 1024 * 1024,
                "Archive exceeds expanded size limit"
            );
            let target = destination.join(path);
            if entry.is_directory() {
                fs::create_dir_all(target)?;
            } else {
                fs::create_dir_all(target.parent().unwrap())?;
                let mut file = File::options().write(true).create_new(true).open(target)?;
                let copied = io::copy(&mut reader.take(entry.size() + 1), &mut file)?;
                ensure!(copied == entry.size(), "Archive entry size mismatch");
                file.sync_all()?;
            }
            Ok(())
        })();
        result.map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
        Ok(true)
    })?;
    ensure!(
        destination.join("mpv.exe").is_file(),
        "MPV executable is missing from the package"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_rejects_windows_aliases_and_traversal() {
        for name in [
            "../mpv.exe",
            "/mpv.exe",
            "C:\\mpv.exe",
            "x/../../mpv.exe",
            "mpv.exe:stream",
            "NUL.txt",
            "a/COM1",
            "a./b",
            "a /b",
        ] {
            assert!(relative(name).is_err(), "{name}");
        }
        assert_eq!(
            relative("doc/manual.pdf").unwrap(),
            PathBuf::from("doc/manual.pdf")
        );
    }
}
