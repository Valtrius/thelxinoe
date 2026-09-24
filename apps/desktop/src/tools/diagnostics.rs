use super::*;
use crate::tools::discovery::{detect_candidates, executable_candidates};
use std::time::{Instant, SystemTime};

type ProbeSlot = Arc<AsyncMutex<Option<CachedProbe>>>;

#[derive(Default)]
pub(super) struct ProbeCache(Mutex<BTreeMap<String, ProbeSlot>>);

#[derive(PartialEq, Eq)]
struct Candidate {
    path: PathBuf,
    source: String,
    metadata: Option<(u64, SystemTime)>,
}

struct CachedProbe {
    candidates: Vec<Candidate>,
    checked: Instant,
    diagnostic: ExecutableDiagnostic,
}

#[derive(Clone)]
pub(super) struct ToolCheck {
    source: ToolSource,
    path: Option<String>,
    checked_at: String,
    diagnostic: ExecutableDiagnostic,
}

#[derive(Default)]
pub(super) struct ToolChecks {
    results: BTreeMap<ToolId, ToolCheck>,
    revisions: BTreeMap<ToolId, u64>,
}

impl ToolChecks {
    fn invalidate(&mut self, id: ToolId) {
        *self.revisions.entry(id).or_default() += 1;
        self.results.remove(&id);
    }

    fn revision(&self, id: ToolId) -> u64 {
        self.revisions.get(&id).copied().unwrap_or_default()
    }
}

impl ProbeCache {
    #[cfg(test)]
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn invalidate(&self, kind: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(kind);
    }

    async fn get(
        &self,
        kind: &str,
        candidates: &[(PathBuf, String)],
        probe: impl Future<Output = ExecutableDiagnostic>,
    ) -> ExecutableDiagnostic {
        let slot = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(kind.to_string())
            .or_default()
            .clone();
        let mut cached = slot.lock().await;
        let candidates = candidates
            .iter()
            .map(|(path, source)| Candidate {
                path: path.clone(),
                source: source.clone(),
                metadata: fs::metadata(path)
                    .ok()
                    .and_then(|m| Some((m.len(), m.modified().ok()?))),
            })
            .collect::<Vec<_>>();
        if let Some(entry) = cached.as_ref()
            && entry.candidates == candidates
            && entry.checked.elapsed()
                < Duration::from_secs(if entry.diagnostic.detected { 60 } else { 5 })
        {
            return entry.diagnostic.clone();
        }
        let diagnostic = probe.await;
        *cached = Some(CachedProbe {
            candidates,
            checked: Instant::now(),
            diagnostic: diagnostic.clone(),
        });
        diagnostic
    }
}

impl ToolManager {
    pub fn invalidate_tool_diagnostic(&self, id: ToolId) {
        self.probes.invalidate(id.key());
        if id == ToolId::Ffmpeg {
            self.probes.invalidate("ffprobe");
        }
        self.checks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate(id);
    }

    pub(crate) async fn probe(&self, kind: &str, path: Option<&str>) -> ExecutableDiagnostic {
        let candidates = executable_candidates(kind, path);
        self.probes
            .get(
                kind,
                &candidates,
                detect_candidates(kind, path, &candidates),
            )
            .await
    }
}

impl ToolManager {
    pub async fn diagnostic(&self, id: ToolId, settings: &AppSettings) -> ExecutableDiagnostic {
        self.diagnostic_from(&self.state(), id, settings).await
    }

    pub(super) async fn diagnostic_from(
        &self,
        state: &Registry,
        id: ToolId,
        settings: &AppSettings,
    ) -> ExecutableDiagnostic {
        let preference = Self::preference(state, id, settings);
        let path = self.selected_path(state, id, settings);
        let revision = self
            .checks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revision(id);
        let mut diagnostic = self.probe(id.key(), path.as_deref()).await;
        if preference.source == ToolSource::Managed {
            diagnostic.source = Some("Managed by Thelxinoe".into());
            diagnostic.update_available = state.catalog.as_ref().is_some_and(|c| {
                c.packages.iter().any(|p| {
                    p.tool == id && upstream::update_available(p, &preference, &state.installed)
                })
            });
        }
        if id == ToolId::Ffmpeg
            && diagnostic.detected
            && let Some(path) = &diagnostic.path
        {
            let probe = Path::new(path).with_file_name("ffprobe.exe");
            let check = self.probe("ffprobe", probe.to_str()).await;
            if !check.detected {
                diagnostic.warning =
                    Some("FFprobe is missing or cannot start; downloads are unavailable.".into());
            }
        }
        // A slow probe for a previous selection must not replace a newer result.
        let current = self.state();
        if Self::preference(&current, id, settings).source == preference.source
            && self.selected_path(&current, id, settings) == path
        {
            let mut checks = self.checks.lock().unwrap_or_else(|e| e.into_inner());
            if checks.revision(id) == revision {
                checks.results.insert(
                    id,
                    ToolCheck {
                        source: preference.source,
                        path,
                        checked_at: crate::utils::utc_now(),
                        diagnostic: diagnostic.clone(),
                    },
                );
            }
        }
        diagnostic
    }

    fn cached_check(
        &self,
        state: &Registry,
        id: ToolId,
        settings: &AppSettings,
    ) -> Option<ToolCheck> {
        let preference = Self::preference(state, id, settings);
        let source = preference.source;
        let path = self.selected_path(state, id, settings);
        self.checks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .results
            .get(&id)
            .filter(|check| check.source == source && check.path == path)
            .cloned()
            .map(|mut check| {
                check.diagnostic.update_available = source == ToolSource::Managed
                    && state.catalog.as_ref().is_some_and(|catalog| {
                        catalog.packages.iter().any(|package| {
                            package.tool == id
                                && upstream::update_available(
                                    package,
                                    &preference,
                                    &state.installed,
                                )
                        })
                    });
                check
            })
    }

    pub fn snapshot(&self, settings: &AppSettings) -> AppResult<ToolsSnapshot> {
        let state = self.state();
        let leases = self
            .leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut tools = Vec::new();
        for id in ToolId::ALL {
            let check = self.cached_check(&state, id, settings);
            let imported_paths = if id.plugin() {
                self.imported_plugin_paths(id)
            } else {
                Vec::new()
            };
            let mut preference = Self::preference(&state, id, settings);
            if id.plugin() && !state.tools.contains_key(&id) && imported_paths.is_empty() {
                preference.enabled = false;
            }
            tools.push(ToolView {
                id,
                preference,
                diagnostic: check.as_ref().map(|check| check.diagnostic.clone()),
                checked_at: check.map(|check| check.checked_at),
                selected_path: self.selected_path(&state, id, settings),
                imported_paths,
                versions: state
                    .catalog
                    .as_ref()
                    .map(|c| {
                        c.packages
                            .iter()
                            .filter(|p| p.tool == id)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                installed: state
                    .installed
                    .iter()
                    .filter(|i| i.package.tool == id)
                    .cloned()
                    .collect(),
                in_use: state
                    .installed
                    .iter()
                    .filter(|i| i.package.tool == id && leases.contains_key(&i.package.id))
                    .map(|i| i.package.id.clone())
                    .collect(),
            });
        }
        Ok(ToolsSnapshot {
            tools,
            mpv: state.mpv,
            operation: self
                .progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            last_checked: state.last_checked,
            catalog_error: state.catalog_error,
            directory: self.root.display().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn snapshots_keep_checks_across_metadata_edits_and_invalidate_only_changed_selections() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let settings = AppSettings::default();
        assert!(
            manager
                .snapshot(&settings)
                .unwrap()
                .tools
                .iter()
                .all(|tool| tool.diagnostic.is_none())
        );
        for id in [ToolId::Mpv, ToolId::Ffmpeg] {
            manager.checks.lock().unwrap().results.insert(
                id,
                ToolCheck {
                    source: ToolSource::System,
                    path: None,
                    checked_at: "2026-01-01T00:00:00Z".into(),
                    diagnostic: ExecutableDiagnostic {
                        kind: id.key().into(),
                        detected: true,
                        path: Some("cached.exe".into()),
                        version: Some("cached version".into()),
                        source: None,
                        warning: None,
                        update_available: false,
                        error: None,
                    },
                },
            );
        }
        let mut preference = ToolPreference {
            pinned: true,
            ..Default::default()
        };
        manager
            .set_preference(ToolId::Mpv, preference.clone())
            .await
            .unwrap();
        manager
            .edit(|state| {
                state.last_checked = Some(crate::utils::utc_now());
                Ok(())
            })
            .unwrap();
        let snapshot = manager.snapshot(&settings).unwrap();
        assert!(snapshot.tools[0].diagnostic.as_ref().unwrap().detected);
        assert_eq!(
            snapshot.tools[0].checked_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        preference.source = ToolSource::Custom;
        preference.custom_path = Some(directory.path().join("other.exe").display().to_string());
        manager
            .set_preference(ToolId::Mpv, preference.clone())
            .await
            .unwrap();
        let snapshot = manager.snapshot(&settings).unwrap();
        assert!(snapshot.tools[0].diagnostic.is_none());
        assert!(snapshot.tools[3].diagnostic.as_ref().unwrap().detected);
        preference.source = ToolSource::System;
        manager
            .set_preference(ToolId::Mpv, preference)
            .await
            .unwrap();
        assert!(
            manager.snapshot(&settings).unwrap().tools[0]
                .diagnostic
                .is_none()
        );
    }

    #[tokio::test]
    async fn concurrent_probes_share_work_but_file_changes_and_retests_invalidate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tool.exe");
        fs::write(&path, b"first").unwrap();
        let candidates = vec![(path.clone(), "configured".into())];
        let cache = ProbeCache::default();
        let calls = AtomicUsize::new(0);
        let probe = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            ExecutableDiagnostic {
                kind: "mpv".into(),
                detected: true,
                path: Some(path.display().to_string()),
                version: Some("test".into()),
                source: None,
                warning: None,
                update_available: false,
                error: None,
            }
        };
        tokio::join!(
            cache.get("mpv", &candidates, probe()),
            cache.get("mpv", &candidates, probe())
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        fs::write(&path, b"replacement").unwrap();
        cache.get("mpv", &candidates, probe()).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        cache.clear();
        cache.get("mpv", &candidates, probe()).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
