use super::*;

const DOWNLOADED_VERSIONS_PER_TOOL: usize = 5;

fn cleanup_candidates(state: &Registry, protected: &HashSet<String>) -> Vec<String> {
    let mut candidates = Vec::new();
    for tool in ToolId::ALL {
        let mut installed: Vec<_> = state
            .installed
            .iter()
            .filter(|i| i.package.tool == tool)
            .collect();
        let excess = installed.len().saturating_sub(DOWNLOADED_VERSIONS_PER_TOOL);
        installed.sort_by(|a, b| {
            upstream::timestamp(&a.installed_at)
                .cmp(&upstream::timestamp(&b.installed_at))
                .then_with(|| a.package.id.cmp(&b.package.id))
        });
        candidates.extend(
            installed
                .into_iter()
                .filter(|i| !protected.contains(&i.package.id))
                .take(excess)
                .map(|i| i.package.id.clone()),
        );
    }
    candidates
}

impl ToolManager {
    pub async fn wait_for_version_cleanup(&self) {
        self.cleanup_requested.notified().await;
    }

    pub async fn cleanup_versions(&self) -> AppResult<bool> {
        let _operation = self.operations.lock().await;
        self.cleanup_versions_locked()
    }

    // Called after successful installs/selections while the operation lock is held,
    // or during construction before the manager is shared.
    pub(super) fn enforce_version_limit(&self) {
        if let Err(error) = self.cleanup_versions_locked() {
            tracing::warn!(code=%error.code, message=%error.message, "old tool version cleanup will be retried");
        }
    }

    fn cleanup_versions_locked(&self) -> AppResult<bool> {
        // Keep selection and lease acquisition out until deletion and the registry
        // update finish. Lease releases may proceed; they can only make more space.
        let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.state();
        let mut protected: HashSet<String> = state
            .tools
            .values()
            .flat_map(|p| [p.active.clone(), p.previous.clone()])
            .flatten()
            .collect();
        protected.extend(
            self.leases
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keys()
                .cloned(),
        );

        // A custom executable can point into a downloaded package too.
        let custom_paths: Vec<_> = state
            .tools
            .values()
            .filter(|p| p.source == ToolSource::Custom)
            .filter_map(|p| p.custom_path.as_ref())
            .filter_map(|p| Path::new(p).canonicalize().ok())
            .collect();
        for installed in &state.installed {
            if !custom_paths.is_empty()
                && let Ok(directory) = self.root.join(&installed.directory).canonicalize()
                && custom_paths.iter().any(|path| path.starts_with(&directory))
            {
                protected.insert(installed.package.id.clone());
            }
        }

        let candidates = cleanup_candidates(&state, &protected);
        let changed = !candidates.is_empty();
        for id in candidates {
            let installed = state
                .installed
                .iter()
                .find(|i| i.package.id == id)
                .ok_or_else(|| AppError::internal("Cleanup package is no longer registered."))?;
            let path = self.root.join(&installed.directory);
            if path.exists() {
                let packages = self.root.join("packages").canonicalize()?;
                let target = path.canonicalize()?;
                if target == packages || !target.starts_with(&packages) {
                    return Err(AppError::validation(
                        "Refusing to clean a version outside managed packages.",
                    ));
                }
                self.remove_owned(&path)?;
            }
            self.edit(|s| {
                s.installed.retain(|i| i.package.id != id);
                Ok(())
            })?;
            tracing::info!(package_id=%id, "removed old downloaded tool version");
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::retention_install;
    #[tokio::test]
    async fn downloaded_versions_are_capped_per_tool_after_activation() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for tool in [ToolId::Mpv, ToolId::Deno] {
            for number in 1..=12 {
                let id = retention_install(&manager, tool, number);
                manager.activate(tool, &id).await.unwrap();
                assert!(
                    manager
                        .state()
                        .installed
                        .iter()
                        .filter(|i| i.package.tool == tool)
                        .count()
                        <= 5
                );
            }
            for number in 1..=12 {
                let path = directory
                    .path()
                    .join(format!("packages/{}-{number:02}", tool.key()));
                assert_eq!(path.exists(), number >= 8);
            }
        }
        assert_eq!(manager.state().installed.len(), 10);
    }

    #[tokio::test]
    async fn cleanup_keeps_old_pinned_and_rollback_versions_without_trimming_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for number in 1..=9 {
            let id = retention_install(&manager, ToolId::Mpv, number);
            if number <= 2 {
                manager.activate(ToolId::Mpv, &id).await.unwrap();
            }
        }
        manager
            .edit(|state| {
                state.tools.get_mut(&ToolId::Mpv).unwrap().pinned = true;
                state.tools.get_mut(&ToolId::Mpv).unwrap().held_versions = vec!["mpv-03".into()];
                state.catalog = Some(Catalog {
                    packages: state.installed.iter().map(|i| i.package.clone()).collect(),
                });
                Ok(())
            })
            .unwrap();
        let before = manager.state();
        assert!(manager.cleanup_versions().await.unwrap());
        let after = manager.state();
        assert_eq!(
            after
                .installed
                .iter()
                .map(|i| i.package.id.as_str())
                .collect::<Vec<_>>(),
            ["mpv-01", "mpv-02", "mpv-07", "mpv-08", "mpv-09"]
        );
        assert_eq!(
            serde_json::to_value(&before.tools).unwrap(),
            serde_json::to_value(&after.tools).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&before.catalog).unwrap(),
            serde_json::to_value(&after.catalog).unwrap()
        );
        manager.rollback(ToolId::Mpv).await.unwrap();
        assert_eq!(
            manager.state().tools[&ToolId::Mpv].active.as_deref(),
            Some("mpv-01")
        );
        assert!(directory.path().join("packages/mpv-01/tool.exe").is_file());
    }

    #[test]
    fn startup_cleans_existing_downloads_and_persists_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        {
            let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
            for number in 1..=12 {
                retention_install(&manager, ToolId::Mpv, number);
            }
        }
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        assert_eq!(manager.state().installed.len(), 5);
        assert_eq!(store::load(directory.path()).unwrap().installed.len(), 5);
        for number in 1..=12 {
            assert_eq!(
                directory
                    .path()
                    .join(format!("packages/mpv-{number:02}"))
                    .exists(),
                number >= 8
            );
        }
    }

    #[tokio::test]
    async fn final_lease_release_requests_cleanup_and_restores_the_cap() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let mut launches = Vec::new();
        for number in 1..=7 {
            let id = retention_install(&manager, ToolId::Mpv, number);
            manager.activate(ToolId::Mpv, &id).await.unwrap();
            launches.push(
                manager
                    .resolve_launch(&AppSettings::default(), &[ToolId::Mpv], false)
                    .await
                    .unwrap()
                    .1,
            );
        }
        assert_eq!(manager.state().installed.len(), 7);
        let first_copy = launches[0].clone();
        drop(launches.remove(0));
        assert!(!manager.cleanup_versions().await.unwrap());
        drop(first_copy);
        tokio::time::timeout(Duration::from_secs(1), manager.wait_for_version_cleanup())
            .await
            .unwrap();
        assert!(manager.cleanup_versions().await.unwrap());
        assert_eq!(manager.state().installed.len(), 6);
        assert!(!directory.path().join("packages/mpv-01").exists());
        assert!(directory.path().join("packages/mpv-02/tool.exe").exists());
        drop(launches.remove(0));
        tokio::time::timeout(Duration::from_secs(1), manager.wait_for_version_cleanup())
            .await
            .unwrap();
        assert!(manager.cleanup_versions().await.unwrap());
        assert_eq!(manager.state().installed.len(), 5);
        for number in 3..=7 {
            assert!(
                directory
                    .path()
                    .join(format!("packages/mpv-{number:02}/tool.exe"))
                    .exists()
            );
        }
    }

    #[tokio::test]
    async fn cleanup_rejects_external_directories_and_keeps_custom_executables() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), b"not managed").unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for number in 1..=6 {
            retention_install(&manager, ToolId::Mpv, number);
        }
        manager
            .edit(|state| {
                state.installed[0].directory = outside.path().display().to_string();
                Ok(())
            })
            .unwrap();
        assert!(manager.cleanup_versions().await.is_err());
        assert_eq!(
            fs::read(outside.path().join("keep.txt")).unwrap(),
            b"not managed"
        );
        assert_eq!(manager.state().installed.len(), 6);
        manager
            .edit(|state| {
                state.installed[0].directory = "packages/mpv-01".into();
                state.tools.insert(
                    ToolId::Mpv,
                    ToolPreference {
                        source: ToolSource::Custom,
                        custom_path: Some(
                            directory
                                .path()
                                .join("packages/mpv-01/tool.exe")
                                .display()
                                .to_string(),
                        ),
                        ..Default::default()
                    },
                );
                Ok(())
            })
            .unwrap();
        assert!(manager.cleanup_versions().await.unwrap());
        assert_eq!(manager.state().installed.len(), 5);
        assert!(directory.path().join("packages/mpv-01/tool.exe").exists());
        assert!(!directory.path().join("packages/mpv-02").exists());
    }
}
