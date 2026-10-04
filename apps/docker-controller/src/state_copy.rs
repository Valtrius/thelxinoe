//! Trusted disposable worker. Mounts and paths are fixed by the controller.
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_FILES: u64 = 500_000;

fn setting(text: &str, key: &str) -> anyhow::Result<Option<(std::ops::Range<usize>, String)>> {
    let document = roxmltree::Document::parse(text)?;
    let root = document.root_element();
    anyhow::ensure!(root.has_tag_name("Config"), "Invalid service configuration");
    let mut entries = root.children().filter(|node| node.has_tag_name(key));
    let entry = entries.next();
    anyhow::ensure!(entries.next().is_none(), "Duplicate {key} setting");
    Ok(entry.map(|entry| (entry.range(), entry.text().unwrap_or("").to_owned())))
}

fn set_setting(text: &mut String, key: &str, value: &str) -> anyhow::Result<()> {
    let range = if let Some((range, _)) = setting(text, key)? {
        range
    } else {
        let document = roxmltree::Document::parse(text)?;
        let end = text[..document.root_element().range().end]
            .rfind("</Config>")
            .ok_or_else(|| anyhow::anyhow!("Missing service configuration end"))?;
        end..end
    };
    let value = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    text.replace_range(range, &format!("<{key}>{value}</{key}>"));
    Ok(())
}

pub fn adopt(kind: &str, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(crate::templates::find(kind).is_some(), "Unknown service");
    if thelxinoe_core::service_gateway_auth(kind) {
        anyhow::ensure!(
            !name.is_empty()
                && name.len() <= 63
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
            "Invalid managed service hostname"
        );
    }
    if kind == "recyclarr" {
        use sha2::{Digest, Sha256};
        let actual = format!("{:x}", Sha256::digest(fs::read("/source/recyclarr.yml")?));
        anyhow::ensure!(
            std::env::var("THELXINOE_RECYCLARR_IMPORT_HASH")? == actual,
            "Recyclarr configuration changed after review"
        );
    }
    run(false)?;
    if kind == "nzbget" {
        let settings: crate::stack::adoption::Nzbget =
            crate::store::read(Path::new("/adoption.json"))?;
        let path = Path::new("/destination/nzbget.conf");
        let mut text = fs::read_to_string(path)?;
        nzbget_setting(&mut text, "ControlUsername", "thelxinoe")?;
        nzbget_setting(&mut text, "ControlPassword", &settings.secret)?;
        if settings.fixes.rotate_logs {
            nzbget_setting(&mut text, "WriteLog", "rotate")?;
            nzbget_setting(&mut text, "RotateLog", "3")?;
        }
        if settings.fixes.cert_check {
            nzbget_setting(
                &mut text,
                "CertStore",
                settings
                    .cert_store
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Missing certificate store"))?,
            )?;
            nzbget_setting(&mut text, "CertCheck", "yes")?;
        }
        fs::write(path, text)?;
        fs::File::open(path)?.sync_all()?;
        return Ok(());
    }
    if kind == "recyclarr" {
        // Ownership metadata belongs to this deployment, never to an imported
        // scheduler. Rebuild it from the explicitly reviewed import selections.
        let metadata = Path::new("/destination/.thelxinoe");
        if metadata.exists() {
            fs::remove_dir_all(metadata)?;
        }
        // Accepted imports retain state while plaintext API keys from the old
        // scheduler are removed from the managed copy before it can run.
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
        return Ok(());
    }
    let base = thelxinoe_core::service_url_base(kind);
    if base.is_empty() {
        return Ok(());
    }
    // The source mount is read-only. Only the stopped managed copy is changed.
    let path = Path::new("/destination").join(if kind == "bazarr" {
        "config/config.yaml"
    } else {
        "config.xml"
    });
    let mut text = fs::read_to_string(&path)?;
    if kind == "bazarr" {
        let mut config: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text)?;
        let general = config
            .get_mut("general")
            .and_then(|v| v.as_mapping_mut())
            .ok_or_else(|| anyhow::anyhow!("Missing Bazarr general settings"))?;
        general.insert("base_url".into(), base.into());
        text = serde_yaml_ng::to_string(&config)?;
    } else {
        set_setting(&mut text, "UrlBase", base)?;
        if thelxinoe_core::service_gateway_auth(kind) {
            set_setting(&mut text, "AuthenticationMethod", "External")?;
            set_setting(&mut text, "AuthenticationRequired", "Enabled")?;
            // Legacy Arr configurations can otherwise override External at startup.
            set_setting(&mut text, "AuthenticationEnabled", "False")?;
            if let Some((_, value)) = setting(&text, "AllowedHosts")? {
                let mut hosts: Vec<String> = value
                    .split([',', ';'])
                    .map(str::trim)
                    .filter(|host| !host.is_empty())
                    .map(str::to_owned)
                    .collect();
                let mut required: Vec<String> =
                    serde_json::from_str(&std::env::var("THELXINOE_ADOPTION_HOSTS")?)?;
                required.extend([
                    name.to_owned(),
                    format!("thelxinoe-{kind}"),
                    "localhost".into(),
                    "127.0.0.1".into(),
                ]);
                for host in required {
                    anyhow::ensure!(
                        !host.is_empty()
                            && host.len() <= 253
                            && host
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric()
                                    || matches!(b, b'-' | b'_' | b'.')),
                        "Invalid managed service alias"
                    );
                    if !hosts
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(&host))
                    {
                        hosts.push(host);
                    }
                }
                set_setting(&mut text, "AllowedHosts", &hosts.join(";"))?;
            }
        }
    }
    // Write in place to retain the copied file's owner and permissions. A failed
    // worker leaves the transfer blocked and its original available for recovery.
    fs::write(&path, text)?;
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

fn nzbget_setting(text: &mut String, key: &str, value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.contains(['\n', '\r', '\0']),
        "Invalid NZBGet setting"
    );
    let mut offset = 0;
    let mut found = None;
    for line in text.split_inclusive('\n') {
        if line
            .split_once('=')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        {
            anyhow::ensure!(found.is_none(), "Duplicate NZBGet setting");
            let end = offset + line.trim_end_matches(['\n', '\r']).len();
            found = Some(offset..end);
        }
        offset += line.len();
    }
    if let Some(range) = found {
        text.replace_range(range, &format!("{key}={value}"));
    } else {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("{key}={value}\n"));
    }
    Ok(())
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
    let link = meta.file_type().is_symlink();
    anyhow::ensure!(
        meta.is_dir() || meta.is_file() || link,
        "Unsupported appdata entry"
    );
    let size = if link {
        fs::symlink_metadata(path.parent().unwrap().join(sibling_link(path)?))?.len()
    } else {
        meta.len()
    };
    anyhow::ensure!(
        meta.nlink() == 1 || meta.is_dir(),
        "Hard-linked appdata is unsupported"
    );
    budget.0 += 1;
    budget.1 = budget
        .1
        .checked_add(size)
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
fn copy(source: &Path, destination: &Path) -> anyhow::Result<()> {
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
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(root.join("source/db"), "consistent state").unwrap();
        validate(&root.join("source"), &mut (0, 0)).unwrap();
        copy(&root.join("source"), &root.join("copy")).unwrap();
        assert_eq!(fs::read(root.join("copy/db")).unwrap(), b"consistent state");
        std::os::unix::fs::symlink("/etc", root.join("source/escape")).unwrap();
        assert!(validate(&root.join("source"), &mut (0, 0)).is_err());
    }
    #[test]
    fn snapshots_logger_links_without_following_other_links() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("dated.log"), "log content").unwrap();
        std::os::unix::fs::symlink("dated.log", source.join("current.log")).unwrap();
        validate(&source, &mut (0, 0)).unwrap();
        copy(&source, &root.join("copy")).unwrap();
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
            assert!(validate(&source, &mut (0, 0)).is_err(), "accepted {target}");
            fs::remove_file(source.join("invalid")).unwrap();
        }
    }
}
