//! Portable authenticated archives. Restore only into a new private staging directory.
use anyhow::{Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};
const MAX_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 1_000_000;
pub fn encrypt(source: &Path, destination: &Path, passphrase: String) -> Result<()> {
    ensure!(
        (16..=1024).contains(&passphrase.len()),
        "Backup passphrase must have 16 to 1024 bytes"
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(destination)?;
    let encryptor = age::Encryptor::with_user_passphrase(passphrase.into());
    let output = encryptor.wrap_output(file)?;
    let mut archive = tar::Builder::new(output);
    let mut count = 0;
    let mut size = 0;
    append(&mut archive, source, source, &mut count, &mut size)?;
    archive.into_inner()?.finish()?.sync_all()?;
    Ok(())
}
fn append<W: Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    path: &Path,
    count: &mut usize,
    size: &mut u64,
) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() || meta.is_file(),
        "Backup contains an unsupported entry"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.is_dir() || meta.nlink() == 1,
            "Backup contains a hard link"
        );
    }
    *count += 1;
    *size = size
        .checked_add(meta.len())
        .ok_or_else(|| anyhow::anyhow!("Backup size overflow"))?;
    ensure!(
        *count <= MAX_FILES && *size <= MAX_BYTES,
        "Backup limit exceeded"
    );
    let name = path.strip_prefix(root)?;
    if meta.is_dir() {
        if !name.as_os_str().is_empty() {
            archive.append_dir(name, path)?;
        }
        for entry in fs::read_dir(path)? {
            append(archive, root, &entry?.path(), count, size)?;
        }
    } else {
        archive.append_file(name, &mut File::open(path)?)?;
    }
    Ok(())
}
pub fn decrypt(source: &Path, destination: &Path, passphrase: String) -> Result<()> {
    ensure!(
        destination.is_dir() && fs::read_dir(destination)?.next().is_none(),
        "Restore staging must be empty"
    );
    ensure!(
        fs::metadata(source)?.len() <= MAX_BYTES + MAX_BYTES / 100,
        "Archive limit exceeded"
    );
    let identity = age::scrypt::Identity::new(passphrase.into());
    let decryptor = age::Decryptor::new(File::open(source)?)?;
    ensure!(
        decryptor.is_scrypt(),
        "Expected a passphrase-encrypted backup"
    );
    let reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
    let mut archive = tar::Archive::new(reader);
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        ensure!(
            !name.as_os_str().is_empty()
                && name.components().all(|c| matches!(c, Component::Normal(_))),
            "Unsafe archive path"
        );
        ensure!(
            seen.insert(name.clone()) && seen.len() <= MAX_FILES,
            "Duplicate or excessive archive entries"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Archive links and special files are unsupported"
        );
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
        ensure!(total <= MAX_BYTES, "Expanded archive limit exceeded");
        let output = destination.join(&name);
        if kind.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            fs::create_dir_all(output.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            std::io::copy(&mut entry, &mut file)?;
            file.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::os::unix::fs::chown(
                &output,
                Some(u32::try_from(entry.header().uid()?)?),
                Some(u32::try_from(entry.header().gid()?)?),
            )?;
            fs::set_permissions(
                &output,
                fs::Permissions::from_mode(entry.header().mode()? & 0o777),
            )?;
        }
    }
    // TAR's end markers are not the authenticated end of the age stream.
    let mut tail = archive.into_inner().take(1024 * 1024);
    let mut remaining = Vec::new();
    tail.read_to_end(&mut remaining)?;
    ensure!(
        remaining.len() < 1024 * 1024 && remaining.iter().all(|v| *v == 0),
        "Unexpected archive trailer"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_archive_traversal_links_and_duplicate_entries() {
        for kind in ["traversal", "link", "duplicate"] {
            let root = tempfile::tempdir().unwrap();
            let output = root.path().join("staging");
            fs::create_dir(&output).unwrap();
            let file = root.path().join("fixture.age");
            let encrypted = age::Encryptor::with_user_passphrase("test archive password".into())
                .wrap_output(File::create(&file).unwrap())
                .unwrap();
            let mut builder = tar::Builder::new(encrypted);
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o600);
            header.set_uid(0);
            header.set_gid(0);
            if kind == "link" {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_link_name("../../outside").unwrap();
                header.set_cksum();
                builder
                    .append_data(&mut header, "link", std::io::empty())
                    .unwrap();
            } else if kind == "traversal" {
                header.set_size(1);
                header.as_mut_bytes()[..10].copy_from_slice(b"../escape\0");
                header.set_cksum();
                builder.append(&header, &b"x"[..]).unwrap();
            } else {
                header.set_size(1);
                header.set_cksum();
                builder.append_data(&mut header, "same", &b"x"[..]).unwrap();
                builder.append_data(&mut header, "same", &b"y"[..]).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
            assert!(
                decrypt(&file, &output, "test archive password".into()).is_err(),
                "{kind}"
            );
            assert!(!root.path().join("escape").exists());
        }
    }
    #[test]
    fn portable_roundtrip_rejects_wrong_password_and_corruption() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("master.key"), b"private marker").unwrap();
        let archive = root.path().join("backup.age");
        let password = "a long test only passphrase";
        encrypt(&input, &archive, password.into()).unwrap();
        assert!(
            !fs::read(&archive)
                .unwrap()
                .windows(14)
                .any(|w| w == b"private marker")
        );
        let output = root.path().join("output");
        fs::create_dir(&output).unwrap();
        assert!(decrypt(&archive, &output, "incorrect passphrase".into()).is_err());
        decrypt(&archive, &output, password.into()).unwrap();
        assert_eq!(
            fs::read(output.join("master.key")).unwrap(),
            b"private marker"
        );
        let mut bytes = fs::read(&archive).unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 1;
        fs::write(&archive, bytes).unwrap();
        let corrupted = root.path().join("corrupt");
        fs::create_dir(&corrupted).unwrap();
        assert!(decrypt(&archive, &corrupted, password.into()).is_err());
    }
}
