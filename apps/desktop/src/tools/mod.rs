mod desktop;
pub mod discovery;
pub use desktop::{DesktopTools, initialize};
mod selection;
pub use selection::LaunchTools;
mod archive;
mod catalog;
pub mod commands;
pub mod config;
mod diagnostics;
mod installation;
pub mod models;
pub mod process;
mod providers;
mod retention;
mod store;
#[cfg(test)]
mod test_support;
mod updates;
mod upstream;

use crate::{
    error::{AppError, AppResult},
    models::{AppSettings, ExecutableDiagnostic},
    tools::discovery::detect_named_executable,
};
use fs2::FileExt;
use models::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

pub type ProgressListener = Box<dyn Fn(&ToolOperation) + Send + Sync>;

/// Start each fresh sandbox with its own MPV configuration and writable files.
#[cfg(test)]
pub(crate) fn seed_sandbox(root: &Path) -> AppResult<()> {
    let config = root.join("config/mpv");
    fs::create_dir_all(&config)?;
    let mut contents = String::from("# Fresh Thelxinoe sandbox configuration.\n");
    for (option, directory) in [
        ("watch-later-directory", "watch-later"),
        ("gpu-shader-cache-dir", "shader-cache"),
        ("screenshot-directory", "screenshots"),
    ] {
        let path = root.join("mpv-data").join(directory);
        fs::create_dir_all(&path)?;
        contents.push_str(&format!(
            "{option}=\"{}\"\n",
            path.to_string_lossy().replace('\\', "/")
        ));
    }
    fs::write(config.join("mpv.conf"), contents)?;
    store::save(
        root,
        &Registry {
            mpv: MpvPreferences {
                source: MpvConfigSource::Managed,
                directory: None,
            },
            ..Registry::default()
        },
    )
}

pub struct ToolManager {
    root: PathBuf,
    registry: Mutex<Registry>,
    probes: diagnostics::ProbeCache,
    checks: Mutex<diagnostics::ToolChecks>,
    operations: AsyncMutex<()>,
    configuration: AsyncMutex<()>,
    refresh: AsyncMutex<()>,
    selection: Mutex<()>,
    progress: Mutex<ToolOperation>,
    leases: Arc<Mutex<BTreeMap<String, usize>>>,
    cleanup_requested: Arc<Notify>,
    closing: AtomicBool,
    client: reqwest::Client,
    on_progress: Option<ProgressListener>,
    _lock: File,
}

impl ToolManager {
    pub fn new(root: PathBuf, on_progress: Option<ProgressListener>) -> AppResult<Self> {
        fs::create_dir_all(&root)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("manager.lock"))?;
        lock.try_lock_exclusive().map_err(|_| {
            AppError::configuration(
                "tools_in_use",
                "Another Thelxinoe instance owns the tool manager.",
                "Close the other instance, then retry.",
            )
        })?;
        let mut registry = store::load(&root)?;
        if registry
            .catalog
            .as_ref()
            .is_some_and(|c| catalog::validate(c).is_err())
        {
            registry.catalog = None;
            registry.upstream_etags.clear();
            registry.upstream_checked.clear();
            registry.last_checked = None;
        }
        // Previous builds seeded the database with bundled versions. Retain only
        // provider lists obtained by an actual successful upstream request.
        if let Some(catalog) = &mut registry.catalog {
            let previous_count = catalog.packages.len();
            catalog.packages.retain(|p| {
                registry.upstream_discovery && registry.upstream_checked.contains_key(&p.tool)
            });
            if catalog.packages.len() != previous_count {
                registry.last_checked = None;
            }
            if catalog.packages.is_empty() {
                registry.catalog = None;
            }
        }
        if registry.catalog.is_none() {
            registry.upstream_etags.clear();
            registry.upstream_checked.clear();
        }
        if !registry.upstream_discovery {
            registry.upstream_discovery = true;
            registry.last_checked = None;
            registry.catalog_error = None;
        }
        store::save(&root, &registry)?;
        let client = reqwest::Client::builder()
            .user_agent("Thelxinoe tool manager")
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;
        let manager = Self {
            root,
            registry: Mutex::new(registry),
            probes: diagnostics::ProbeCache::default(),
            checks: Mutex::new(diagnostics::ToolChecks::default()),
            operations: AsyncMutex::new(()),
            configuration: AsyncMutex::new(()),
            refresh: AsyncMutex::new(()),
            selection: Mutex::new(()),
            progress: Mutex::new(ToolOperation::default()),
            leases: Arc::new(Mutex::new(BTreeMap::new())),
            cleanup_requested: Arc::new(Notify::new()),
            closing: AtomicBool::new(false),
            client,
            on_progress,
            _lock: lock,
        };
        let staging = manager.root.join("staging");
        if staging.exists() {
            manager.remove_owned(&staging)?;
        }
        fs::create_dir_all(staging)?;
        let runtime = manager.root.join("runtime");
        if runtime.exists() {
            manager.remove_owned(&runtime)?;
        }
        let packages = manager.root.join("packages");
        if packages.exists() {
            let installed = manager
                .state()
                .installed
                .into_iter()
                .map(|i| manager.root.join(i.directory))
                .collect::<HashSet<_>>();
            for tool in fs::read_dir(&packages)? {
                let tool = tool?.path();
                if !tool.is_dir() {
                    continue;
                }
                for entry in fs::read_dir(tool)? {
                    let path = entry?.path();
                    if path.is_dir() && !installed.contains(&path) {
                        manager.remove_owned(&path)?;
                    }
                }
            }
        }
        manager.enforce_version_limit();
        Ok(manager)
    }
    fn state(&self) -> Registry {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn edit<T>(&self, update: impl FnOnce(&mut Registry) -> AppResult<T>) -> AppResult<T> {
        let mut state = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.clone();
        let result = update(&mut next)?;
        store::save(&self.root, &next)?;
        *state = next;
        Ok(result)
    }
    fn emit(&self, operation: ToolOperation) {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = operation.clone();
        if let Some(listener) = &self.on_progress {
            listener(&operation);
        }
    }
    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
    }
    pub async fn wait_for_operations(&self) {
        let _guard = self.operations.lock().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::fake_install;

    #[test]
    fn sandbox_starts_with_its_own_mpv_configuration_and_no_installed_tools() {
        let directory = tempfile::tempdir().unwrap();
        seed_sandbox(directory.path()).unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let state = manager.state();
        assert_eq!(state.mpv.source, MpvConfigSource::Managed);
        assert!(state.installed.is_empty());
        let (arguments, snapshot) = manager.player_config_arguments(&state).unwrap();
        let snapshot = snapshot.unwrap();
        assert_eq!(arguments[0], format!("--config-dir={}", snapshot.display()));
        assert_eq!(
            fs::read_to_string(snapshot.join("mpv.conf")).unwrap(),
            fs::read_to_string(directory.path().join("config/mpv/mpv.conf")).unwrap()
        );
        assert!(directory.path().join("mpv-data/watch-later").is_dir());
    }

    #[test]
    fn second_writer_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        assert!(ToolManager::new(directory.path().to_path_buf(), None).is_err());
        drop(manager);
        assert!(ToolManager::new(directory.path().to_path_buf(), None).is_ok());
    }

    #[test]
    fn migration_keeps_installed_selection_and_discards_the_private_feed_error() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        fake_install(&manager, ToolId::Mpv, "existing-mpv");
        manager
            .activate_locked(ToolId::Mpv, "existing-mpv", false)
            .unwrap();
        let mut legacy = serde_json::to_value(manager.state()).unwrap();
        legacy["catalog"] = serde_json::to_value(Catalog {
            packages: vec![providers::test_package(ToolId::Mpv)],
        })
        .unwrap();
        for key in [
            "upstreamDiscovery",
            "upstreamEtags",
            "upstreamChecked",
            "upstreamRetryAt",
        ] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        legacy["catalog"]["sequence"] = 123.into();
        legacy["catalog"]["expiresAt"] = "2020-01-01T00:00:00Z".into();
        legacy["catalogError"] = "Private catalog unavailable".into();
        legacy["lastChecked"] = crate::utils::utc_now().into();
        store::save(directory.path(), &serde_json::from_value(legacy).unwrap()).unwrap();
        drop(manager);
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let mut state = manager.state();
        assert!(state.upstream_discovery);
        assert!(state.catalog_error.is_none());
        assert!(state.last_checked.is_none());
        assert!(state.catalog.is_none());
        assert_eq!(
            state.tools[&ToolId::Mpv].active.as_deref(),
            Some("existing-mpv")
        );
        assert_eq!(state.installed.len(), 1);
        upstream::merge(
            &mut state,
            ToolId::Mpv,
            upstream::Discovery {
                packages: vec![providers::test_package(ToolId::Mpv)],
                etag: None,
            },
            &crate::utils::utc_now(),
        );
        assert_eq!(state.catalog.unwrap().packages[0].id, "existing-mpv");
    }

    #[test]
    fn migration_removes_only_versions_that_never_came_from_upstream() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        manager
            .edit(|state| {
                state.catalog = Some(Catalog {
                    packages: vec![
                        providers::test_package(ToolId::Mpv),
                        providers::test_package(ToolId::Deno),
                    ],
                });
                state.last_checked = Some(crate::utils::utc_now());
                state
                    .upstream_checked
                    .insert(ToolId::Mpv, crate::utils::utc_now());
                state
                    .upstream_etags
                    .insert(ToolId::Mpv, "upstream-etag".into());
                Ok(())
            })
            .unwrap();
        drop(manager);
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let state = manager.state();
        let packages = state.catalog.unwrap().packages;
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].tool, ToolId::Mpv);
        assert_eq!(state.upstream_etags[&ToolId::Mpv], "upstream-etag");
        assert!(state.last_checked.is_none());
    }
}
