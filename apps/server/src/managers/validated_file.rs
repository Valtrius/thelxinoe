use super::*;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path, time::SystemTime};

#[derive(PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
    changed: i128,
}
fn stamp(file: &File) -> anyhow::Result<Stamp> {
    let metadata = file.metadata()?;
    anyhow::ensure!(metadata.is_file(), "Target is not a file");
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec())
    };
    #[cfg(windows)]
    let changed = {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandleEx},
        };
        let mut info = FILE_BASIC_INFO::default();
        // The owned file handle remains open and the buffer has the exact API layout.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileBasicInfo,
                (&mut info as *mut FILE_BASIC_INFO).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )?;
        }
        i128::from(info.ChangeTime)
    };
    Ok(Stamp {
        size: metadata.len(),
        modified: metadata.modified()?,
        changed,
    })
}
fn ordinary(target: &Target) -> anyhow::Result<()> {
    let path = Path::new(&target.path);
    let root = std::fs::canonicalize(&target.root)?;
    let actual = std::fs::canonicalize(path)?;
    anyhow::ensure!(
        actual == path
            && actual.starts_with(root)
            && !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Target path changed"
    );
    Ok(())
}

pub(super) struct ValidatedFile {
    target: Target,
    handle: same_file::Handle,
    stamp: Stamp,
}
impl ValidatedFile {
    pub(super) fn capture(target: Target) -> anyhow::Result<Self> {
        ordinary(&target)?;
        let mut file = File::open(&target.path)?;
        let before = stamp(&file)?;
        anyhow::ensure!(
            before.size == target.size
                && before
                    .modified
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
                    .to_string()
                    == target.modified,
            "File generation changed"
        );
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        let fingerprint = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        anyhow::ensure!(
            fingerprint == target.fingerprint && before == stamp(&file)?,
            "File changed during validation"
        );
        let proof = Self {
            target,
            handle: same_file::Handle::from_file(file)?,
            stamp: before,
        };
        proof.recheck()?;
        Ok(proof)
    }
    pub(super) fn recheck(&self) -> anyhow::Result<()> {
        ordinary(&self.target)?;
        anyhow::ensure!(
            self.handle == same_file::Handle::from_path(&self.target.path)?
                && self.stamp == stamp(self.handle.as_file())?,
            "File identity or content changed after validation"
        );
        Ok(())
    }
}
