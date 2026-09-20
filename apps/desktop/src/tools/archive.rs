use crate::error::{AppError, AppResult};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_EXPANDED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 30_000;

pub fn safe_relative(name: &str) -> AppResult<PathBuf> {
    let normal = name.replace('\\', "/");
    let path = PathBuf::from(&normal);
    if normal.is_empty()
        || normal.starts_with('/')
        || normal.contains(':')
        || normal.contains('\0')
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::validation("An archive contains an unsafe path."));
    }
    for part in normal.trim_end_matches('/').split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part.ends_with(['.', ' '])
            || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit())
        {
            return Err(AppError::validation(
                "An archive contains a reserved Windows path.",
            ));
        }
    }
    Ok(path)
}

struct Extraction {
    root: PathBuf,
    seen: HashSet<String>,
    total: u64,
}
impl Extraction {
    fn entry(
        &mut self,
        name: &str,
        size: u64,
        directory: bool,
        reader: &mut dyn Read,
    ) -> AppResult<()> {
        let relative = safe_relative(name.trim_end_matches(['/', '\\']))?;
        let key = relative.to_string_lossy().to_lowercase();
        if self.seen.len() >= MAX_FILES || !self.seen.insert(key) {
            return Err(AppError::validation(
                "Archive has duplicate paths or too many files.",
            ));
        }
        self.total = self
            .total
            .checked_add(size)
            .ok_or_else(|| AppError::validation("Archive is too large."))?;
        if self.total > MAX_EXPANDED {
            return Err(AppError::validation(
                "Archive expands beyond the tool size limit.",
            ));
        }
        let target = self.root.join(relative);
        if directory {
            fs::create_dir_all(target)?;
            return Ok(());
        }
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| AppError::validation("Missing archive directory."))?,
        )?;
        let mut file = File::options().write(true).create_new(true).open(target)?;
        let copied = io::copy(&mut reader.take(size + 1), &mut file)?;
        if copied != size {
            return Err(AppError::validation(
                "Archive entry size does not match its header.",
            ));
        }
        file.flush()?;
        Ok(())
    }
}

pub fn extract(archive: &Path, destination: &Path, format: &str, entry: &str) -> AppResult<()> {
    fs::create_dir(destination)?;
    let mut extraction = Extraction {
        root: destination.to_path_buf(),
        seen: HashSet::new(),
        total: 0,
    };
    match format {
        "file" => {
            let mut file = File::open(archive)?;
            let size = file.metadata()?.len();
            extraction.entry(entry, size, false, &mut file)?;
        }
        "zip" => {
            let mut zip = zip::ZipArchive::new(File::open(archive)?)
                .map_err(|e| AppError::validation(e.to_string()))?;
            if zip.len() > MAX_FILES {
                return Err(AppError::validation("Too many archive entries."));
            }
            for i in 0..zip.len() {
                let mut item = zip
                    .by_index(i)
                    .map_err(|e| AppError::validation(e.to_string()))?;
                if item.is_symlink()
                    || item
                        .unix_mode()
                        .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000 | 0o040000))
                {
                    return Err(AppError::validation(
                        "Archive links and special files are not supported.",
                    ));
                }
                let name = item.name().to_string();
                let size = item.size();
                let dir = item.is_dir();
                extraction.entry(&name, size, dir, &mut item)?;
            }
        }
        "7z" => {
            let mut reader =
                sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
                    .map_err(|e| AppError::validation(e.to_string()))?;
            reader
                .for_each_entries(|entry, reader| {
                    if entry.windows_attributes() & 0x400 != 0 {
                        return Err(sevenz_rust2::Error::Other(
                            "Archive reparse points are not supported".into(),
                        ));
                    }
                    extraction
                        .entry(entry.name(), entry.size(), entry.is_directory(), reader)
                        .map_err(|e| sevenz_rust2::Error::Other(e.message.into()))?;
                    Ok(true)
                })
                .map_err(|e| AppError::validation(e.to_string()))?;
        }
        _ => return Err(AppError::validation("Unsupported archive format.")),
    }
    Ok(())
}

pub fn find_entry(root: &Path, entry: &str) -> AppResult<PathBuf> {
    let wanted = safe_relative(entry)?;
    if root.join(&wanted).is_file() {
        return Ok(root.join(wanted));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut found = Vec::new();
    let mut count = 0;
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(dir)? {
            let item = item?;
            count += 1;
            if count > MAX_FILES {
                return Err(AppError::validation("Too many package files."));
            }
            let kind = item.file_type()?;
            if kind.is_symlink() {
                return Err(AppError::validation("Package contains a link."));
            }
            if kind.is_dir() {
                pending.push(item.path());
            } else if item.path().ends_with(&wanted) {
                found.push(item.path());
            }
        }
    }
    if found.len() != 1 {
        return Err(AppError::validation(format!(
            "The package must contain exactly one {entry}."
        )));
    }
    Ok(found.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_archive_paths_cannot_escape() {
        for name in [
            "../a", "a/../b", "C:/a", "//host/a", "a:stream", "CON.txt", "a/PRN", "a.", "a/../",
            "a\\..\\b",
        ] {
            assert!(safe_relative(name).is_err(), "{name}");
        }
        assert!(safe_relative("scripts/uosc/main.lua").is_ok());
    }
    #[test]
    fn extraction_rejects_wrong_size_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut ex = Extraction {
            root: dir.path().into(),
            seen: HashSet::new(),
            total: 0,
        };
        assert!(ex.entry("ok", 2, false, &mut &b"abc"[..]).is_err());
        assert!(ex.entry("OK", 0, false, &mut &b""[..]).is_err());
    }
}
