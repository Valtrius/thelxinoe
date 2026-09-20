use super::*;

pub(super) fn legacy_path(id: ToolId, s: &AppSettings) -> Option<&str> {
    match id {
        ToolId::Mpv => s.mpv_path.as_deref(),
        ToolId::Ytdlp => s.ytdlp_path.as_deref(),
        ToolId::Streamlink => s.streamlink_path.as_deref(),
        ToolId::Ffmpeg => s.ffmpeg_path.as_deref(),
        _ => None,
    }
    .filter(|p| !p.trim().is_empty())
}

impl LaunchTools {
    pub fn configure(&self, command: &mut tokio::process::Command) -> AppResult<()> {
        let current = std::env::var_os("PATH").unwrap_or_default();
        let dirs = self
            .directories
            .iter()
            .cloned()
            .chain(std::env::split_paths(&current));
        command.env(
            "PATH",
            std::env::join_paths(dirs).map_err(|e| AppError::validation(e.to_string()))?,
        );
        command.env("PYTHONDONTWRITEBYTECODE", "1");
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct LaunchTools {
    pub deno: Option<String>,
    pub mpv_args: Vec<String>,
    pub directories: Vec<PathBuf>,
    pub lease: ToolLease,
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        for id in &self.ids {
            if let Some(count) = leases.get_mut(id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    leases.remove(id);
                }
            }
        }
        drop(leases);
        self.cleanup_requested.notify_one();
        if let Some(path) = &self.config {
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub(super) struct LeaseInner {
    pub(super) ids: Vec<String>,
    pub(super) leases: Arc<Mutex<BTreeMap<String, usize>>>,
    pub(super) cleanup_requested: Arc<Notify>,
    pub(super) config: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub struct ToolLease {
    _inner: Option<Arc<LeaseInner>>,
}

impl ToolManager {
    pub(super) fn preference(
        state: &Registry,
        id: ToolId,
        settings: &AppSettings,
    ) -> ToolPreference {
        state.tools.get(&id).cloned().unwrap_or_else(|| {
            if id.plugin() {
                return ToolPreference {
                    source: ToolSource::Custom,
                    enabled: true,
                    ..Default::default()
                };
            }
            let custom_path = legacy_path(id, settings).map(str::to_string);
            ToolPreference {
                source: if custom_path.is_some() {
                    ToolSource::Custom
                } else {
                    ToolSource::System
                },
                custom_path,
                ..Default::default()
            }
        })
    }

    pub(super) fn selected_path(
        &self,
        state: &Registry,
        id: ToolId,
        settings: &AppSettings,
    ) -> Option<String> {
        let p = Self::preference(state, id, settings);
        match p.source {
            ToolSource::System => None,
            ToolSource::Custom => p.custom_path,
            ToolSource::Managed => p
                .active
                .and_then(|active| {
                    state
                        .installed
                        .iter()
                        .find(|i| i.package.id == active)
                        .map(|i| {
                            self.root
                                .join(&i.directory)
                                .join(&i.package.entry)
                                .display()
                                .to_string()
                        })
                })
                .or_else(|| {
                    Some(
                        self.root
                            .join("missing")
                            .join(format!("{}.exe", id.key()))
                            .display()
                            .to_string(),
                    )
                }),
        }
    }

    pub(super) fn resolve_settings_from(
        &self,
        state: &Registry,
        settings: &AppSettings,
    ) -> AppSettings {
        let mut next = settings.clone();
        next.mpv_path = self.selected_path(state, ToolId::Mpv, settings);
        next.ytdlp_path = self.selected_path(state, ToolId::Ytdlp, settings);
        next.streamlink_path = self.selected_path(state, ToolId::Streamlink, settings);
        next.ffmpeg_path = self.selected_path(state, ToolId::Ffmpeg, settings);
        next
    }

    pub async fn set_preference(&self, id: ToolId, preference: ToolPreference) -> AppResult<()> {
        if !id.plugin()
            && preference.source == ToolSource::Custom
            && let Some(path) = preference
                .custom_path
                .as_deref()
                .and_then(|p| Path::new(p).canonicalize().ok())
            && path.starts_with(self.root.canonicalize()?.join("packages"))
        {
            return Err(AppError::validation(
                "Choose Provided by Thelxinoe to use an installed managed package.",
            ));
        }
        if !matches!(preference.channel.as_str(), "recommended" | "preview") {
            return Err(AppError::validation("Unknown tool channel."));
        }
        if !id.plugin()
            && preference.source == ToolSource::Custom
            && preference.custom_path.as_deref().is_none_or(|p| {
                !Path::new(p).is_absolute() || !p.to_ascii_lowercase().ends_with(".exe")
            })
        {
            return Err(AppError::validation(
                "Select an absolute Windows executable path.",
            ));
        }
        let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
        let changed_selection = self.edit(|s| {
            let old = s.tools.get(&id).cloned().unwrap_or_default();
            let mut p = preference;
            p.active = old.active.clone();
            p.previous = old.previous.clone();
            p.held_versions = old.held_versions.clone();
            if id.plugin() && p.enabled && p.source == ToolSource::Managed && p.active.is_none() {
                return Err(AppError::validation(
                    "Install the plugin before enabling it.",
                ));
            }
            if id.plugin()
                && p.enabled
                && p.source != ToolSource::Managed
                && self.imported_plugin_paths(id).is_empty()
            {
                return Err(AppError::validation(
                    "Import a local copy of this plugin before selecting it.",
                ));
            }
            let changed = old.source != p.source || old.custom_path != p.custom_path;
            s.tools.insert(id, p);
            Ok(changed)
        })?;
        if changed_selection && !id.plugin() {
            self.invalidate_tool_diagnostic(id);
        }
        Ok(())
    }

    pub async fn resolve_launch(
        &self,
        settings: &AppSettings,
        required: &[ToolId],
        mpv: bool,
    ) -> AppResult<(AppSettings, LaunchTools)> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(AppError::validation("Thelxinoe is closing."));
        }
        let (resolved, mut runtime, deno_path, deno_source) = {
            let _selection = self.selection.lock().unwrap_or_else(|e| e.into_inner());
            let state = self.state();
            let mut runtime = LaunchTools::default();
            let mut ids = HashSet::new();
            for id in ToolId::ALL {
                if !(required.contains(&id)
                    || id.plugin() && mpv && state.mpv.source == MpvConfigSource::Managed)
                {
                    continue;
                }
                let p = Self::preference(&state, id, settings);
                if p.source == ToolSource::Managed
                    && (!id.plugin() || (mpv && p.enabled))
                    && let Some(active) = p.active
                {
                    ids.insert(active);
                }
                if !id.plugin()
                    && let Some(path) = self.selected_path(&state, id, settings)
                    && let Some(parent) = Path::new(&path).parent()
                {
                    runtime.directories.push(parent.into());
                }
            }
            let ids = ids.into_iter().collect::<Vec<_>>();
            {
                let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
                for id in &ids {
                    *leases.entry(id.clone()).or_default() += 1;
                }
            }
            runtime.lease = ToolLease {
                _inner: Some(Arc::new(LeaseInner {
                    ids,
                    leases: Arc::clone(&self.leases),
                    cleanup_requested: Arc::clone(&self.cleanup_requested),
                    config: None,
                })),
            };
            if mpv {
                let (args, config) = self.player_config_arguments(&state)?;
                runtime.mpv_args = args;
                if let Some(inner) = runtime.lease._inner.as_mut().and_then(Arc::get_mut) {
                    inner.config = config;
                }
            }
            (
                self.resolve_settings_from(&state, settings),
                runtime,
                self.selected_path(&state, ToolId::Deno, settings),
                Self::preference(&state, ToolId::Deno, settings).source,
            )
        };
        if !required.contains(&ToolId::Deno) {
            return Ok((resolved, runtime));
        }
        let explicitly_selected = deno_source != ToolSource::System;
        let unavailable = || {
            AppError::configuration(
                "selected_deno_unavailable",
                "The selected Deno runtime is missing or cannot run.",
                "Open MPV settings and repair Deno or select a working executable.",
            )
        };
        if explicitly_selected
            && deno_path
                .as_deref()
                .is_none_or(|path| path.trim().is_empty())
        {
            return Err(unavailable());
        }
        let deno = self.probe("deno", deno_path.as_deref()).await;
        if explicitly_selected && (!deno.detected || deno.path.is_none()) {
            return Err(unavailable());
        }
        if deno.detected {
            runtime.deno = deno.path;
        }
        Ok((resolved, runtime))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::fake_install;
    #[tokio::test]
    async fn launches_lease_only_the_tools_they_use() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        for (tool, id) in [(ToolId::Mpv, "mpv-test"), (ToolId::Ytdlp, "ytdlp-test")] {
            fake_install(&manager, tool, id);
            manager.activate(tool, id).await.unwrap();
        }
        let (_, runtime) = manager
            .resolve_launch(&AppSettings::default(), &[ToolId::Mpv], false)
            .await
            .unwrap();
        let leases = manager.leases.lock().unwrap().clone();
        assert_eq!(leases.get("mpv-test"), Some(&1));
        assert!(!leases.contains_key("ytdlp-test"));
        assert_eq!(runtime.directories.len(), 1);
        drop(runtime);
        assert!(manager.leases.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn explicit_deno_selection_rejects_missing_and_unusable_executables() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let invalid = directory.path().join("invalid.exe");
        fs::write(&invalid, b"not an executable").unwrap();
        for path in [
            Some(directory.path().join("missing.exe").display().to_string()),
            Some(invalid.display().to_string()),
            None,
        ] {
            manager
                .edit(|state| {
                    state.tools.insert(
                        ToolId::Deno,
                        ToolPreference {
                            source: ToolSource::Custom,
                            custom_path: path,
                            ..Default::default()
                        },
                    );
                    Ok(())
                })
                .unwrap();
            let error = manager
                .resolve_launch(&AppSettings::default(), &[ToolId::Deno], false)
                .await
                .err()
                .unwrap();
            assert_eq!(error.code, "selected_deno_unavailable");
            assert!(manager.leases.lock().unwrap().is_empty());
        }
        fake_install(&manager, ToolId::Deno, "broken-deno");
        manager.activate(ToolId::Deno, "broken-deno").await.unwrap();
        let error = manager
            .resolve_launch(&AppSettings::default(), &[ToolId::Deno], false)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "selected_deno_unavailable");
        assert!(manager.leases.lock().unwrap().is_empty());
        // MPV option discovery, cached-video playback and reconciliation do not use Deno.
        assert!(
            manager
                .resolve_launch(&AppSettings::default(), &[ToolId::Mpv], false)
                .await
                .is_ok()
        );
    }
}
