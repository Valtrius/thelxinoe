use super::*;

pub(super) fn select_package(
    s: &mut Registry,
    tool: ToolId,
    id: &str,
    rollback: bool,
) -> AppResult<()> {
    if !s
        .installed
        .iter()
        .any(|i| i.package.id == id && i.package.tool == tool)
    {
        return Err(AppError::validation("That version is not installed."));
    }
    let p = s.tools.entry(tool).or_default();
    let first = p.active.is_none();
    if p.active.as_deref() != Some(id) {
        if rollback && let Some(active) = &p.active {
            p.held_versions.push(active.clone());
        }
        p.previous = p.active.replace(id.into());
    }
    p.source = ToolSource::Managed;
    if first {
        p.enabled = tool.plugin();
        p.update_policy = UpdatePolicy::Automatic;
    }
    Ok(())
}

pub(super) fn record_installation(
    state: &mut Registry,
    installed: InstalledPackage,
    preserve_selection: bool,
) -> AppResult<Option<String>> {
    let id = installed.package.id.clone();
    let tool = installed.package.tool;
    let old = state
        .installed
        .iter()
        .find(|i| i.package.id == id)
        .map(|i| i.directory.clone());
    state.installed.retain(|i| i.package.id != id);
    state.installed.push(installed);
    // Repairs and downloads overtaken by a preference change only record the files.
    if !preserve_selection {
        select_package(state, tool, &id, false)?;
    }
    Ok(old)
}

impl ToolManager {
    pub async fn install(&self, id: &str, settings: &AppSettings) -> AppResult<()> {
        let _operation = self.operations.lock().await;
        self.install_locked(id, settings).await
    }

    pub(super) async fn install_locked(&self, id: &str, settings: &AppSettings) -> AppResult<()> {
        let state = self.state();
        let package = state
            .catalog
            .as_ref()
            .and_then(|c| c.packages.iter().find(|p| p.id == id))
            .cloned()
            .ok_or_else(|| AppError::validation("Select an available upstream package."))?;
        let result = async {
            for dependency in &package.dependencies {
                if !self.diagnostic(*dependency, settings).await.detected {
                    let preference = Self::preference(&state, *dependency, settings);
                    if preference.source == ToolSource::Custom || (preference.source == ToolSource::System && state.tools.contains_key(dependency)) {
                        return Err(AppError::validation(format!("The selected {} executable is unavailable. Check it or choose a managed version in MPV settings; your installation will not be replaced.", dependency.key())));
                    }
                    let p = state
                        .catalog
                        .as_ref()
                        .and_then(|c| {
                            c.packages
                                .iter()
                                .find(|p| p.tool == *dependency && p.recommended)
                        })
                        .cloned()
                        .ok_or_else(|| AppError::validation("A required tool is unavailable."))?;
                    let package_id = p.id.clone();
                    self.install_one_selected(p, false, state.tools.get(dependency).cloned())
                        .await?;
                    let current = self.state();
                    if current.tools.get(dependency).is_some_and(|p| p.source == ToolSource::Managed && p.active.as_deref() == Some(&package_id)) {
                        self.diagnostic(*dependency, settings).await;
                    }
                }
            }
            self.install_one_selected(
                package.clone(),
                false,
                state.tools.get(&package.tool).cloned(),
            )
            .await
        }
        .await;
        if result.is_ok() {
            let current = self.state();
            if !package.tool.plugin()
                && current.tools.get(&package.tool).is_some_and(|p| {
                    p.source == ToolSource::Managed && p.active.as_deref() == Some(&package.id)
                })
            {
                self.diagnostic(package.tool, settings).await;
            }
        }
        self.emit(ToolOperation {
            tool: Some(package.tool),
            package_id: Some(package.id),
            phase: if result.is_ok() { "complete" } else { "failed" }.into(),
            error: result.as_ref().err().map(|e| e.message.clone()),
            ..Default::default()
        });
        result
    }

    pub(super) async fn install_one(&self, package: Package, repair: bool) -> AppResult<()> {
        let expected = self.state().tools.get(&package.tool).cloned();
        self.install_one_selected(package, repair, expected).await
    }

    async fn install_one_selected(
        &self,
        package: Package,
        repair: bool,
        expected: Option<ToolPreference>,
    ) -> AppResult<()> {
        catalog::validate_package(&package)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(AppError::validation("Thelxinoe is closing."));
        }
        if !repair
            && self
                .state()
                .installed
                .iter()
                .any(|i| i.package.id == package.id)
        {
            {
                let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
                let activated = self.edit(|state| {
                    if state.tools.get(&package.tool) != expected.as_ref() {
                        return Ok(false);
                    }
                    select_package(state, package.tool, &package.id, false)?;
                    Ok(true)
                })?;
                if activated {
                    self.invalidate_tool_diagnostic(package.tool);
                }
            }
            self.enforce_version_limit();
            return Ok(());
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let staging = self.root.join("staging").join(&operation);
        fs::create_dir(&staging)?;
        let result = self
            .install_staged(package, repair, &operation, &staging, expected.as_ref())
            .await;
        if staging.exists() {
            let _ = self.remove_owned(&staging);
        }
        if result.is_ok() {
            self.enforce_version_limit();
        }
        result
    }

    async fn install_staged(
        &self,
        mut package: Package,
        repair: bool,
        operation: &str,
        staging: &Path,
        expected: Option<&ToolPreference>,
    ) -> AppResult<()> {
        let archive = staging.join("download");
        let destination = staging.join("extracted");
        let available = fs2::available_space(&self.root)?;
        if available < package.size.saturating_mul(6) + 128 * 1024 * 1024 {
            return Err(AppError::validation(
                "Not enough disk space to download and stage this tool.",
            ));
        }
        let mut response = self
            .client
            .get(&package.url)
            .send()
            .await
            .map_err(|e| AppError::validation(e.to_string()))?
            .error_for_status()
            .map_err(|e| AppError::validation(e.to_string()))?;
        let mut output = File::create(&archive)?;
        let mut digest = Sha256::new();
        let mut downloaded = 0;
        self.emit(ToolOperation {
            tool: Some(package.tool),
            package_id: Some(package.id.clone()),
            phase: "downloading".into(),
            total: package.size,
            ..Default::default()
        });
        let mut last = std::time::Instant::now();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| AppError::validation(e.to_string()))?
        {
            if self.closing.load(Ordering::SeqCst) {
                return Err(AppError::validation(
                    "Tool download stopped because Thelxinoe is closing.",
                ));
            }
            downloaded += chunk.len() as u64;
            if downloaded > package.size {
                return Err(AppError::validation(
                    "Tool download exceeded its expected size.",
                ));
            }
            digest.update(&chunk);
            output.write_all(&chunk)?;
            if last.elapsed() > Duration::from_millis(150) {
                self.emit(ToolOperation {
                    tool: Some(package.tool),
                    package_id: Some(package.id.clone()),
                    phase: "downloading".into(),
                    downloaded,
                    total: package.size,
                    error: None,
                });
                last = std::time::Instant::now();
            }
        }
        output.sync_all()?;
        drop(output);
        let hash = digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if downloaded != package.size || hash != package.sha256.to_ascii_lowercase() {
            return Err(AppError::validation(
                "Tool download failed its SHA-256 or size check.",
            ));
        }
        self.emit(ToolOperation {
            tool: Some(package.tool),
            package_id: Some(package.id.clone()),
            phase: "verifying".into(),
            downloaded,
            total: package.size,
            error: None,
        });
        let archive_path = archive.clone();
        let extraction_path = destination.clone();
        let format = package.format.clone();
        let entry = package.entry.clone();
        tokio::task::spawn_blocking(move || {
            archive::extract(&archive_path, &extraction_path, &format, &entry)
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        let entry = archive::find_entry(&destination, &package.entry)?;
        let version = if package.tool.plugin() {
            "Script package verified".into()
        } else {
            let check = detect_named_executable(package.tool.key(), entry.to_str()).await;
            if !check.detected {
                return Err(check.error.unwrap_or_else(|| {
                    AppError::validation("Tool did not pass its startup check.")
                }));
            }
            if package.tool == ToolId::Ffmpeg {
                let probe = entry.with_file_name("ffprobe.exe");
                if !self.probe("ffprobe", probe.to_str()).await.detected {
                    return Err(AppError::validation(
                        "FFmpeg package does not include a working FFprobe.",
                    ));
                }
            }
            check.version.unwrap_or_default()
        };
        package.entry = entry
            .strip_prefix(&destination)
            .map_err(|e| AppError::internal(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let directory = format!(
            "packages/{}/{}-{}",
            package.tool.key(),
            package.id,
            operation
        );
        let target = self.root.join(&directory);
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| AppError::internal("Missing package parent"))?,
        )?;
        fs::rename(&destination, &target)?;
        let installed = InstalledPackage {
            package: package.clone(),
            installed_at: crate::utils::utc_now(),
            directory,
            version_output: version,
        };
        let _configuration = self.configuration.lock().await;
        let old = {
            let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
            if repair
                && self
                    .leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&package.id)
            {
                return Err(AppError::validation(
                    "A new player or download started using this version during repair. Close it and retry.",
                ));
            }
            self.prepare_plugin_config(&package)?;
            let old = self.edit(|state| {
                let preserve = repair || state.tools.get(&package.tool) != expected;
                record_installation(state, installed, preserve)
            })?;
            let current = self.state();
            if current.tools.get(&package.tool).is_some_and(|p| {
                p.source == ToolSource::Managed && p.active.as_deref() == Some(&package.id)
            }) {
                self.invalidate_tool_diagnostic(package.tool);
            }
            old
        };
        if let Some(old) = old {
            let path = self.root.join(old);
            if path.exists() {
                let _ = self.remove_owned(&path);
            }
        }
        Ok(())
    }

    pub async fn activate(&self, tool: ToolId, id: &str) -> AppResult<()> {
        self.activate_locked(tool, id, false)
    }

    pub(super) fn activate_locked(&self, tool: ToolId, id: &str, rollback: bool) -> AppResult<()> {
        {
            let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
            self.edit(|state| select_package(state, tool, id, rollback))?;
            self.invalidate_tool_diagnostic(tool);
        }
        self.enforce_version_limit();
        Ok(())
    }

    pub async fn rollback(&self, tool: ToolId) -> AppResult<()> {
        {
            let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
            self.edit(|state| {
                let id = state
                    .tools
                    .get(&tool)
                    .and_then(|p| p.previous.clone())
                    .ok_or_else(|| {
                        AppError::validation("There is no previous installed version.")
                    })?;
                select_package(state, tool, &id, true)
            })?;
            self.invalidate_tool_diagnostic(tool);
        }
        self.enforce_version_limit();
        Ok(())
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let _guard = self.operations.lock().await;
        let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.state();
        if state
            .tools
            .values()
            .any(|p| p.active.as_deref() == Some(id) || p.previous.as_deref() == Some(id))
        {
            return Err(AppError::validation(
                "The active and rollback versions are retained.",
            ));
        }
        let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        if leases.contains_key(id) {
            return Err(AppError::validation(
                "A player or download still uses this version.",
            ));
        }
        let installed = state
            .installed
            .iter()
            .find(|i| i.package.id == id)
            .ok_or_else(|| AppError::validation("Package is not installed."))?;
        let path = self.root.join(&installed.directory);
        if path.exists() {
            self.remove_owned(&path)?;
        }
        self.edit(|s| {
            s.installed.retain(|i| i.package.id != id);
            Ok(())
        })
    }

    pub async fn repair(&self, id: &str) -> AppResult<()> {
        let _guard = self.operations.lock().await;
        if self
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
        {
            return Err(AppError::validation(
                "Close players and finish downloads using this tool before repairing it.",
            ));
        }
        let state = self.state();
        // Installed records retain the original URL/hash even after a version falls
        // out of the upstream release window. Never repair with replacement bytes.
        let package = state
            .installed
            .iter()
            .find(|i| i.package.id == id)
            .map(|i| i.package.clone())
            .ok_or_else(|| AppError::validation("Package is not installed."))?;
        let result = self.install_one(package.clone(), true).await;
        self.emit(ToolOperation {
            tool: Some(package.tool),
            package_id: Some(package.id),
            phase: if result.is_ok() { "complete" } else { "failed" }.into(),
            error: result.as_ref().err().map(|e| e.message.clone()),
            ..Default::default()
        });
        result
    }

    pub(super) fn remove_owned(&self, path: &Path) -> AppResult<()> {
        let root = self.root.canonicalize()?;
        let target = path.canonicalize()?;
        if target == root || !target.starts_with(&root) {
            return Err(AppError::validation(
                "Refusing to remove a directory outside managed tool storage.",
            ));
        }
        fs::remove_dir_all(path)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::fake_install;

    #[tokio::test]
    async fn a_download_overtaken_by_a_preference_change_does_not_activate() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        fake_install(&manager, ToolId::Mpv, "old");
        fake_install(&manager, ToolId::Mpv, "new");
        manager.activate(ToolId::Mpv, "old").await.unwrap();
        let expected = manager.state().tools[&ToolId::Mpv].clone();
        let mut chosen = expected.clone();
        chosen.source = ToolSource::System;
        manager.set_preference(ToolId::Mpv, chosen).await.unwrap();
        let package = manager
            .state()
            .installed
            .iter()
            .find(|item| item.package.id == "new")
            .unwrap()
            .package
            .clone();
        manager
            .install_one_selected(package, false, Some(expected))
            .await
            .unwrap();
        let current = manager.state();
        assert_eq!(current.tools[&ToolId::Mpv].source, ToolSource::System);
        assert_eq!(current.tools[&ToolId::Mpv].active.as_deref(), Some("old"));
        assert_eq!(current.installed.len(), 2);
    }
    #[tokio::test]
    async fn repair_registration_preserves_all_selection_and_retention_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for tool in [ToolId::Mpv, ToolId::Uosc] {
            let active = format!("{}-active", tool.key());
            let inactive = format!("{}-inactive", tool.key());
            fake_install(&manager, tool, &active);
            fake_install(&manager, tool, &inactive);
            manager.activate(tool, &active).await.unwrap();
            for source in [ToolSource::Managed, ToolSource::System, ToolSource::Custom] {
                for id in [&active, &inactive] {
                    manager
                        .edit(|state| {
                            let p = state.tools.get_mut(&tool).unwrap();
                            p.source = source;
                            p.previous = Some(inactive.clone());
                            p.pinned = true;
                            p.enabled = false;
                            p.channel = "preview".into();
                            p.update_policy = UpdatePolicy::Manual;
                            p.held_versions = vec![inactive.clone()];
                            Ok(())
                        })
                        .unwrap();
                    let before = manager.state();
                    let original = before
                        .installed
                        .iter()
                        .find(|i| &i.package.id == id)
                        .unwrap();
                    let mut repaired = original.clone();
                    repaired.directory = format!("{}/replacement", original.directory);
                    repaired.version_output = "repaired".into();
                    let old = manager
                        .edit(|state| record_installation(state, repaired.clone(), true))
                        .unwrap();
                    let after = manager.state();
                    assert_eq!(old.as_deref(), Some(original.directory.as_str()));
                    assert_eq!(
                        serde_json::to_value(&after.tools).unwrap(),
                        serde_json::to_value(&before.tools).unwrap()
                    );
                    assert_eq!(after.installed.len(), before.installed.len());
                    let installed = after
                        .installed
                        .iter()
                        .find(|i| &i.package.id == id)
                        .unwrap();
                    assert_eq!(installed.directory, repaired.directory);
                    assert_eq!(installed.version_output, "repaired");
                }
            }
            // A normal install still activates the selected version explicitly.
            let installed = manager
                .state()
                .installed
                .into_iter()
                .find(|i| i.package.id == inactive)
                .unwrap();
            manager
                .edit(|state| record_installation(state, installed, false))
                .unwrap();
            assert_eq!(
                manager.state().tools[&tool].active.as_deref(),
                Some(inactive.as_str())
            );
            assert_eq!(manager.state().tools[&tool].source, ToolSource::Managed);
        }
    }

    #[tokio::test]
    async fn rollback_holds_failed_update_and_retains_in_use_versions() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for id in ["first", "second", "third"] {
            fake_install(&manager, ToolId::Mpv, id);
        }
        manager.activate(ToolId::Mpv, "first").await.unwrap();
        manager.activate(ToolId::Mpv, "second").await.unwrap();
        manager.rollback(ToolId::Mpv).await.unwrap();
        let preference = manager.state().tools[&ToolId::Mpv].clone();
        assert_eq!(preference.active.as_deref(), Some("first"));
        assert!(preference.held_versions.contains(&"second".into()));
        assert!(manager.remove("first").await.is_err());
        assert!(manager.remove("second").await.is_err());
        manager.leases.lock().unwrap().insert("third".into(), 1);
        assert!(manager.remove("third").await.is_err());
        manager.leases.lock().unwrap().clear();
        manager.remove("third").await.unwrap();
        drop(manager);
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        assert_eq!(
            manager.state().tools[&ToolId::Mpv].active.as_deref(),
            Some("first")
        );
    }

    #[tokio::test]
    async fn plugin_updates_preserve_disabled_state() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        fake_install(&manager, ToolId::Uosc, "first");
        fake_install(&manager, ToolId::Uosc, "second");
        manager.activate(ToolId::Uosc, "first").await.unwrap();
        let mut p = manager.state().tools[&ToolId::Uosc].clone();
        p.enabled = false;
        manager.set_preference(ToolId::Uosc, p).await.unwrap();
        manager.activate(ToolId::Uosc, "second").await.unwrap();
        assert!(!manager.state().tools[&ToolId::Uosc].enabled);
    }

    #[tokio::test]
    #[ignore = "Downloads real Windows packages into an isolated temporary directory; run explicitly when qualifying upstream discovery"]
    async fn qualify_upstream_packages() {
        let directory = tempfile::tempdir().unwrap();
        println!("Qualification directory: {}", directory.path().display());
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let settings = AppSettings::default();
        assert!(manager.state().catalog.is_none());
        assert!(
            manager
                .snapshot(&settings)
                .unwrap()
                .tools
                .iter()
                .all(|tool| tool.versions.is_empty())
        );
        manager.refresh_catalog().await.unwrap();
        assert_eq!(
            manager.state().upstream_checked.len(),
            ToolId::DESKTOP.len()
        );
        let first = manager.state();
        assert_eq!(first.upstream_etags.len(), ToolId::DESKTOP.len());
        manager
            .edit(|s| {
                s.last_checked = None;
                Ok(())
            })
            .unwrap();
        manager.refresh_catalog().await.unwrap();
        assert_eq!(
            serde_json::to_value(&first.catalog).unwrap(),
            serde_json::to_value(&manager.state().catalog).unwrap()
        );
        let catalog = manager.state().catalog.unwrap();
        for id in ToolId::DESKTOP {
            let package = catalog
                .packages
                .iter()
                .find(|p| p.tool == id && p.recommended)
                .unwrap();
            println!("Installing {} {}", id.key(), package.version);
            manager
                .install(&package.id, &settings)
                .await
                .unwrap_or_else(|e| panic!("{}: {} {:?}", id.key(), e.message, e.technical));
        }
        let schema = manager.mpv_schema(&settings).await.unwrap();
        assert!(schema.options.len() > 300);
        assert!(schema.options.iter().any(|v| v["name"] == "volume"));
        println!("MPV exposes {} options", schema.options.len());
        manager
            .set_mpv_preferences(MpvPreferences {
                source: MpvConfigSource::Managed,
                directory: None,
            })
            .await
            .unwrap();
        let doc = manager.config_document("mpv.conf").unwrap();
        let saved = manager
            .save_config_document(
                "mpv.conf",
                "# qualification\nvolume=42\n",
                &doc.revision,
                &settings,
            )
            .await
            .unwrap();
        assert_eq!(saved.text, "# qualification\nvolume=42\n");
        assert!(
            manager
                .save_config_document(
                    "mpv.conf",
                    "volume=invalid-number\n",
                    &saved.revision,
                    &settings
                )
                .await
                .is_err()
        );
        assert_eq!(
            manager.config_document("mpv.conf").unwrap().text,
            saved.text
        );
        assert!(
            manager
                .save_config_document(
                    "mpv.conf",
                    "nonexistent-youtwitch-option=yes\n",
                    &saved.revision,
                    &settings
                )
                .await
                .is_err()
        );
        let (_, runtime) = manager
            .resolve_launch(&settings, &[ToolId::Mpv], true)
            .await
            .unwrap();
        assert!(runtime.mpv_args.iter().any(|a| a.contains("config-dir")));
        assert_eq!(
            runtime
                .mpv_args
                .iter()
                .filter(|a| a.contains("scripts-append") && !a.contains("youtwitch-bridge-"))
                .count(),
            3
        );
        let active = manager.state().tools[&ToolId::Mpv].active.clone().unwrap();
        assert!(manager.repair(&active).await.is_err());
        drop(runtime);
        manager
            .edit(|s| {
                s.catalog
                    .as_mut()
                    .unwrap()
                    .packages
                    .retain(|p| p.id != active);
                Ok(())
            })
            .unwrap();
        manager.repair(&active).await.unwrap();
        println!(
            "{}",
            manager
                .test_mpv_configuration(&settings, false)
                .await
                .unwrap()
        );
        println!(
            "{}",
            manager
                .test_mpv_configuration(&settings, true)
                .await
                .unwrap()
        );
        println!("All upstream packages and MPV configuration checks passed");
    }
}
