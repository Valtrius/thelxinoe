use super::*;
use tauri::{Emitter, Manager};

pub struct DesktopTools {
    tools: AppResult<Arc<ToolManager>>,
}

impl DesktopTools {
    pub fn manager(&self) -> AppResult<&Arc<ToolManager>> {
        self.tools.as_ref().map_err(Clone::clone)
    }
    /// Unavailable tools have no writes to drain.
    pub async fn restart_guard(
        &self,
    ) -> Option<(
        tokio::sync::MutexGuard<'_, ()>,
        tokio::sync::MutexGuard<'_, ()>,
    )> {
        match &self.tools {
            Ok(tools) => Some(tools.restart_guard().await),
            Err(_) => None,
        }
    }
}

fn open(app: &tauri::App) -> AppResult<Arc<ToolManager>> {
    let data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let handle = app.handle().clone();
    let manager = Arc::new(ToolManager::new(
        data.join("tools"),
        Some(Box::new(move |operation| {
            let _ = handle.emit("tools-progress", operation);
        })),
    )?);
    tauri::async_runtime::block_on(manager.migrate_legacy(&data.join("mpv")))?;
    Ok(manager)
}

/// Only another running instance is fatal; it keeps the tools and desktop updates.
/// Any other failure leaves playback and tools unavailable, while the rest of the
/// app and desktop updates keep running so a fixed release can be installed.
pub fn initialize(app: &tauri::App) -> AppResult<()> {
    let manager = match open(app) {
        Ok(manager) => manager,
        Err(error) if error.code == "tools_in_use" => return Err(error),
        Err(error) => {
            tracing::error!(%error, technical = %error.technical, "Desktop tools could not start");
            app.manage(DesktopTools {
                tools: Err(AppError::configuration(
                    "tools_unavailable",
                    format!(
                        "Player and tools could not start: {}. Restart Thelxinoe or install a desktop update.",
                        error.technical.trim_end_matches('.')
                    ),
                    "Restart Thelxinoe or install a desktop update.",
                )),
            });
            return Ok(());
        }
    };
    app.manage(DesktopTools {
        tools: Ok(manager.clone()),
    });
    let handle = app.handle().clone();
    let cleanup = manager.clone();
    tauri::async_runtime::spawn(async move {
        let mut retry = tokio::time::interval(Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = cleanup.wait_for_version_cleanup() => {},
                _ = retry.tick() => {},
            }
            if cleanup.closing.load(Ordering::SeqCst) {
                break;
            }
            match cleanup.cleanup_versions().await {
                Ok(true) => {
                    let _ = handle.emit("tools-changed", ());
                }
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "MPV version cleanup will be retried"),
            }
        }
    });
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        manager.diagnostic(ToolId::Mpv).await;
        let _ = handle.emit("tools-changed", ());
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if manager.closing.load(Ordering::SeqCst) {
                break;
            }
            manager.auto_update().await;
            let _ = handle.emit("tools-changed", ());
        }
    });
    Ok(())
}

impl ToolManager {
    // Keep the original installation and configuration intact. Existing binaries
    // remain selected as local executables until the user chooses a managed build.
    async fn migrate_legacy(&self, old: &Path) -> AppResult<()> {
        let marker = self.root.join("legacy-migrated");
        if marker.exists() {
            return Ok(());
        }
        if old.join("config").is_dir() {
            self.import_config(&old.join("config").to_string_lossy())
                .await?;
        }
        if old.join("selection.json").is_file() {
            let selection: serde_json::Value =
                serde_json::from_slice(&fs::read(old.join("selection.json"))?)
                    .map_err(|e| AppError::internal(e.to_string()))?;
            if let Some(path) = selection["path"].as_str().filter(|p| !p.is_empty()) {
                self.set_preference(
                    ToolId::Mpv,
                    ToolPreference {
                        source: ToolSource::Custom,
                        custom_path: Some(path.into()),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }
        fs::write(marker, b"1")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migration_preserves_legacy_config_plugins_and_selected_executable() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("mpv");
        fs::create_dir_all(old.join("config/scripts")).unwrap();
        fs::write(old.join("config/mpv.conf"), "volume=37\n").unwrap();
        fs::write(old.join("config/scripts/custom.lua"), "-- user plugin\n").unwrap();
        let executable = old.join("mpv.exe").display().to_string();
        fs::write(
            old.join("selection.json"),
            serde_json::json!({"path":executable}).to_string(),
        )
        .unwrap();
        let manager = ToolManager::new(temp.path().join("tools"), None).unwrap();
        manager.migrate_legacy(&old).await.unwrap();
        assert_eq!(
            manager.state().tools[&ToolId::Mpv].custom_path.as_deref(),
            Some(executable.as_str())
        );
        assert_eq!(
            manager.config_document("mpv.conf").unwrap().text,
            "volume=37\n"
        );
        assert!(manager.root.join("config/mpv/scripts/custom.lua").exists());
        assert!(old.join("config/scripts/custom.lua").exists());
        fs::write(old.join("config/mpv.conf"), "volume=90\n").unwrap();
        manager.migrate_legacy(&old).await.unwrap();
        assert_eq!(
            manager.config_document("mpv.conf").unwrap().text,
            "volume=37\n"
        );
    }
}
