use super::*;

impl ToolManager {
    pub fn config_document(&self, name: &str) -> AppResult<ConfigDocument> {
        let path = self.config_root().join(document_path(name)?);
        let text = read_text(&path)?;
        Ok(ConfigDocument {
            name: name.into(),
            revision: revision(&text),
            text,
            has_backup: backup_path(&path).is_file(),
        })
    }

    pub fn config_files(&self) -> AppResult<Vec<String>> {
        let mut files = vec![
            "mpv.conf".into(),
            "input.conf".into(),
            "script-opts/uosc.conf".into(),
            "script-opts/thumbfast.conf".into(),
            "script-opts/sub_select.conf".into(),
            "script-opts/sub-select.json".into(),
        ];
        let dir = self.config_root().join("script-opts");
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let name = format!("script-opts/{}", entry.file_name().to_string_lossy());
                if document_path(&name).is_ok() && !files.contains(&name) {
                    files.push(name);
                }
            }
        }
        Ok(files)
    }

    pub async fn save_config_document(
        &self,
        name: &str,
        text: &str,
        expected: &str,
        settings: &AppSettings,
    ) -> AppResult<ConfigDocument> {
        let _guard = self.configuration.lock().await;
        self.save_document_locked(name, text, expected, settings)
            .await
    }

    pub(in crate::tools) async fn save_document_locked(
        &self,
        name: &str,
        text: &str,
        expected: &str,
        settings: &AppSettings,
    ) -> AppResult<ConfigDocument> {
        if text.len() > MAX_TEXT as usize || text.contains('\0') {
            return Err(AppError::validation(
                "Configuration must be UTF-8 text under 1 MiB without NUL characters.",
            ));
        }
        let path = self.config_root().join(document_path(name)?);
        let old = read_text(&path)?;
        if revision(&old) != expected {
            return Err(AppError::validation(
                "This file changed outside the editor. Reload it before saving.",
            ));
        }
        if name.ends_with(".json") {
            serde_json::from_str::<Value>(text)
                .map_err(|e| AppError::validation(format!("Invalid JSON: {e}")))?;
        }
        if name == "mpv.conf" {
            let staging = self
                .root
                .join("staging")
                .join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&staging)?;
            let validation = async {
                copy_tree(&self.config_root(), &staging)?;
                fs::write(staging.join("mpv.conf"), text)?;
                let (resolved, _lease) =
                    self.resolve_launch(settings, &[ToolId::Mpv], false).await?;
                let check = self.probe("mpv", resolved.mpv_path.as_deref()).await;
                let mpv = check.path.filter(|_| check.detected).ok_or_else(|| {
                    AppError::validation(
                        "Select a working MPV executable to validate this configuration.",
                    )
                })?;
                let mut probe =
                    Probe::start(&mpv, &[format!("--config-dir={}", staging.display())]).await?;
                probe
                    .request(json!(["get_property", "mpv-version"]))
                    .await?;
                let log = probe.finish().await?;
                if log.lines().any(|l| {
                    l.contains("Error parsing")
                        || l.contains("Error reading config")
                        || l.contains("Invalid value")
                        || l.contains("option not found")
                }) {
                    return Err(AppError::validation(format!(
                        "MPV rejected this configuration: {}",
                        log.chars().take(2000).collect::<String>()
                    )));
                }
                Ok(())
            }
            .await;
            let _ = self.remove_owned(&staging);
            validation?;
        }
        // Both checks matter: validation yields while another program may edit the file.
        if revision(&read_text(&path)?) != expected {
            return Err(AppError::validation(
                "The configuration changed during validation. Reload it before saving.",
            ));
        }
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| AppError::internal("Missing config parent"))?,
        )?;
        atomic_write(&backup_path(&path), old.as_bytes())?;
        atomic_write(&path, text.as_bytes())?;
        self.config_document(name)
    }

    pub async fn restore_config_document(
        &self,
        name: &str,
        expected: &str,
        settings: &AppSettings,
    ) -> AppResult<ConfigDocument> {
        let _guard = self.configuration.lock().await;
        let path = backup_path(&self.config_root().join(document_path(name)?));
        if !path.is_file() {
            return Err(AppError::validation(
                "There is no saved backup for this file.",
            ));
        }
        self.save_document_locked(name, &read_text(&path)?, expected, settings)
            .await
    }

    pub async fn import_config(&self, source: &str) -> AppResult<()> {
        let _guard = self.configuration.lock().await;
        let source = Path::new(source).canonicalize()?;
        let root = self.root.canonicalize()?;
        if source.starts_with(&root) || root.starts_with(&source) {
            return Err(AppError::validation(
                "Choose a configuration directory outside managed tool storage.",
            ));
        }
        let staging = self
            .root
            .join("staging")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&staging)?;
        let result = (|| {
            copy_tree(&source, &staging)?;
            let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
            let config = self.config_root();
            fs::create_dir_all(
                config
                    .parent()
                    .ok_or_else(|| AppError::internal("Missing config parent"))?,
            )?;
            let backup = config.with_file_name(format!("mpv-backup-{}", uuid::Uuid::new_v4()));
            if config.exists() {
                fs::rename(&config, &backup)?;
            }
            if let Err(e) = fs::rename(&staging, &config) {
                if backup.exists() {
                    let _ = fs::rename(backup, config);
                }
                return Err(e.into());
            }
            self.edit(|s| {
                s.mpv.source = MpvConfigSource::Managed;
                Ok(())
            })
        })();
        if staging.exists() {
            let _ = self.remove_owned(&staging);
        }
        result
    }

    pub async fn import_plugin(&self, source: &str) -> AppResult<()> {
        let _guard = self.configuration.lock().await;
        let source = Path::new(source);
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::validation("Select a Lua or JavaScript plugin."))?;
        if !matches!(
            source.extension().and_then(|e| e.to_str()),
            Some("lua" | "js")
        ) {
            return Err(AppError::validation(
                "Local plugins must be .lua or .js files. Import a whole configuration folder for plugins with supporting files.",
            ));
        }
        archive::safe_relative(name)?;
        if fs::symlink_metadata(source)?.file_type().is_symlink()
            || fs::metadata(source)?.len() > 16 * 1024 * 1024
        {
            return Err(AppError::validation(
                "The selected plugin is too large or is a link.",
            ));
        }
        let target = self.config_root().join("scripts").join(name);
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| AppError::internal("Missing script parent"))?,
        )?;
        let mut out = fs::File::options()
            .write(true)
            .create_new(true)
            .open(target)?;
        std::io::copy(&mut fs::File::open(source)?, &mut out)?;
        Ok(())
    }
}

pub(super) fn document_path(name: &str) -> AppResult<PathBuf> {
    let p = archive::safe_relative(name)?;
    let parts = name.split('/').collect::<Vec<_>>();
    if !(matches!(parts.as_slice(), ["mpv.conf"] | ["input.conf"])
        || (parts.len() == 2
            && parts[0] == "script-opts"
            && (parts[1].ends_with(".conf") || parts[1] == "sub-select.json")))
    {
        return Err(AppError::validation(
            "Only MPV configuration files and plugin settings can be edited.",
        ));
    }
    Ok(p)
}

pub(super) fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".previous");
    PathBuf::from(name)
}

pub(super) fn revision(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) fn read_text(path: &Path) -> AppResult<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.len() > MAX_TEXT {
        return Err(AppError::validation(
            "The configuration file is a link or exceeds 1 MiB.",
        ));
    }
    Ok(fs::read_to_string(path)?)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = fs::File::options()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        std::io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub(super) fn copy_tree(source: &Path, target: &Path) -> AppResult<()> {
    if !source.exists() {
        return Ok(());
    }
    let mut queue = vec![(source.to_path_buf(), target.to_path_buf())];
    let mut total = 0u64;
    let mut count = 0;
    while let Some((source, target)) = queue.pop() {
        let meta = fs::symlink_metadata(&source)?;
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(AppError::validation(
                "Configuration imports cannot contain symbolic links or junctions.",
            ));
        }
        if meta.is_dir() {
            fs::create_dir_all(&target)?;
            for e in fs::read_dir(source)? {
                let e = e?;
                archive::safe_relative(&e.file_name().to_string_lossy())?;
                queue.push((e.path(), target.join(e.file_name())));
            }
        } else {
            count += 1;
            total = total.saturating_add(meta.len());
            if count > 5000 || total > 256 * 1024 * 1024 {
                return Err(AppError::validation(
                    "Configuration imports are limited to 5,000 files and 256 MiB.",
                ));
            }
            fs::copy(source, target)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_paths_are_confined() {
        for name in ["mpv.conf", "input.conf", "script-opts/uosc.conf"] {
            assert!(document_path(name).is_ok());
        }
        for name in [
            "../mpv.conf",
            "scripts/run.lua",
            "script-opts/../mpv.conf",
            "C:/mpv.conf",
            "script-opts/x.conf/extra",
        ] {
            assert!(document_path(name).is_err());
        }
    }
    #[test]
    fn raw_document_roundtrip_preserves_unknown_options_and_comments() {
        let text = "# comment\r\nunknown-future-option=yes\r\n[my profile]\r\nvolume=42\r\n";
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let path = root.join("mpv.conf");
        atomic_write(&path, text.as_bytes()).unwrap();
        assert_eq!(read_text(&path).unwrap(), text);
        assert_ne!(revision(text), revision(&format!("{text}# new")));
        fs::remove_dir_all(root).unwrap();
    }
}
