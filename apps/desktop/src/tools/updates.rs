use super::*;

impl ToolManager {
    pub async fn refresh_catalog(&self) -> AppResult<()> {
        let _refresh = self.refresh.lock().await;
        let state = self.state();
        let now = chrono::Utc::now();
        if let Some(until) = state
            .upstream_retry_at
            .as_deref()
            .and_then(upstream::timestamp)
            && until > now
        {
            return Err(AppError::validation(format!(
                "GitHub update checks are paused until {until}."
            )));
        }
        // One request per provider; shared by manual and background checks.
        if state
            .last_checked
            .as_deref()
            .and_then(upstream::timestamp)
            .is_some_and(|last| now.signed_duration_since(last).num_minutes() < 15)
        {
            return state
                .catalog_error
                .map_or(Ok(()), |e| Err(AppError::validation(e)));
        }
        let checked = now.to_rfc3339();
        let mut github = upstream::Github {
            client: &self.client,
            retry_at: None,
        };
        let mut errors = Vec::new();
        for tool in ToolId::DESKTOP {
            if self.closing.load(Ordering::SeqCst) {
                break;
            }
            let previous: Vec<_> = state
                .catalog
                .as_ref()
                .map(|c| {
                    c.packages
                        .iter()
                        .filter(|p| p.tool == tool)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            match github
                .discover(
                    tool,
                    &previous,
                    state.upstream_etags.get(&tool).map(String::as_str),
                )
                .await
            {
                Ok(discovered) => self.edit(|state| {
                    upstream::merge(state, tool, discovered, &checked);
                    Ok(())
                })?,
                Err(error) => errors.push(format!("{}: {}", tool.key(), error.message)),
            }
        }
        let error = (!errors.is_empty()).then(|| errors.join("\n"));
        self.edit(|state| {
            state.last_checked = Some(checked);
            state.catalog_error = error.clone();
            state.upstream_retry_at = github.retry_at;
            Ok(())
        })?;
        error.map_or(Ok(()), |e| Err(AppError::validation(e)))
    }

    pub fn available_updates(&self, settings: &AppSettings) -> Vec<ToolUpdate> {
        let state = self.state();
        ToolId::DESKTOP
            .into_iter()
            .filter_map(|id| {
                let p = Self::preference(&state, id, settings);
                if p.source != ToolSource::Managed
                    || id.plugin() && !p.enabled
                    || p.pinned
                    || p.update_policy != UpdatePolicy::Notify
                {
                    return None;
                }
                let active = state
                    .installed
                    .iter()
                    .find(|i| Some(&i.package.id) == p.active.as_ref())?;
                let package = state.catalog.as_ref()?.packages.iter().find(|v| {
                    v.tool == id && upstream::update_available(v, &p, &state.installed)
                })?;
                Some(ToolUpdate {
                    tool: id,
                    package_id: package.id.clone(),
                    version: package.version.clone(),
                    current_version: active.package.version.clone(),
                })
            })
            .collect()
    }

    pub async fn update(&self, id: &str, settings: &AppSettings) -> AppResult<()> {
        let _operation = self.operations.lock().await;
        if !self
            .available_updates(settings)
            .iter()
            .any(|update| update.package_id == id)
        {
            return Err(AppError::validation(
                "This update is no longer available under your current tool settings.",
            ));
        }
        self.install_locked(id, settings).await
    }

    pub async fn auto_update(&self, settings: &AppSettings) {
        let state = self.state();
        let retry_hours = if state.catalog_error.is_some() { 1 } else { 24 };
        let last = state.last_checked.as_deref().and_then(upstream::timestamp);
        if last
            .is_some_and(|d| chrono::Utc::now().signed_duration_since(d).num_hours() < retry_hours)
        {
            return;
        }
        // A failed provider keeps its cache; other freshly checked tools can update.
        let started = chrono::Utc::now();
        let _ = self.refresh_catalog().await;
        let state = self.state();
        if state
            .last_checked
            .as_deref()
            .and_then(upstream::timestamp)
            .is_none_or(|checked| checked < started)
        {
            return;
        }
        for id in ToolId::DESKTOP {
            if let Err(e) = self
                .auto_update_tool(id, settings, state.last_checked.as_deref())
                .await
            {
                tracing::warn!(tool=id.key(),code=%e.code,"automatic tool update failed");
            }
        }
    }

    pub(super) async fn auto_update_tool(
        &self,
        id: ToolId,
        settings: &AppSettings,
        checked: Option<&str>,
    ) -> AppResult<()> {
        let _operation = self.operations.lock().await;
        // A queued preference change or rollback must take effect before selecting a package.
        let state = self.state();
        let p = Self::preference(&state, id, settings);
        if checked.is_none()
            || state.last_checked.as_deref() != checked
            || state.upstream_checked.get(&id).map(String::as_str) != checked
            || p.source != ToolSource::Managed
            || id.plugin() && !p.enabled
            || p.pinned
            || p.update_policy != UpdatePolicy::Automatic
        {
            return Ok(());
        }
        if let Some(package) = state.catalog.as_ref().and_then(|c| {
            c.packages
                .iter()
                .find(|v| v.tool == id && upstream::update_available(v, &p, &state.installed))
        }) {
            self.install_locked(&package.id, settings).await?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::fake_install;
    #[tokio::test]
    async fn notifications_cover_every_managed_tool_and_respect_update_selection() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let settings = AppSettings::default();
        for tool in ToolId::DESKTOP {
            let active = format!("{}-old", tool.key());
            fake_install(&manager, tool, &active);
            manager.activate(tool, &active).await.unwrap();
            manager
                .edit(|state| {
                    state.tools.get_mut(&tool).unwrap().update_policy = UpdatePolicy::Notify;
                    let mut package = providers::test_package(tool);
                    package.id = format!("{}-new", tool.key());
                    state
                        .catalog
                        .get_or_insert_with(Catalog::default)
                        .packages
                        .push(package);
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(
            manager.available_updates(&settings).len(),
            ToolId::DESKTOP.len()
        );
        let mut preference = manager.state().tools[&ToolId::Mpv].clone();
        for policy in [UpdatePolicy::Automatic, UpdatePolicy::Manual] {
            preference.update_policy = policy;
            manager
                .set_preference(ToolId::Mpv, preference.clone())
                .await
                .unwrap();
            assert!(
                !manager
                    .available_updates(&settings)
                    .iter()
                    .any(|u| u.tool == ToolId::Mpv)
            );
            assert!(manager.update("mpv-new", &settings).await.is_err());
        }
        preference.update_policy = UpdatePolicy::Notify;
        preference.pinned = true;
        manager
            .set_preference(ToolId::Mpv, preference.clone())
            .await
            .unwrap();
        assert!(manager.update("mpv-new", &settings).await.is_err());
        preference.pinned = false;
        manager
            .edit(|state| {
                state
                    .tools
                    .get_mut(&ToolId::Mpv)
                    .unwrap()
                    .held_versions
                    .push("mpv-new".into());
                Ok(())
            })
            .unwrap();
        manager
            .set_preference(ToolId::Mpv, preference.clone())
            .await
            .unwrap();
        assert!(
            !manager
                .available_updates(&settings)
                .iter()
                .any(|u| u.tool == ToolId::Mpv)
        );
        manager
            .edit(|state| {
                state
                    .tools
                    .get_mut(&ToolId::Mpv)
                    .unwrap()
                    .held_versions
                    .clear();
                Ok(())
            })
            .unwrap();
        preference.source = ToolSource::System;
        manager
            .set_preference(ToolId::Mpv, preference.clone())
            .await
            .unwrap();
        assert!(
            !manager
                .available_updates(&settings)
                .iter()
                .any(|u| u.tool == ToolId::Mpv)
        );
        assert!(manager.update("mpv-new", &settings).await.is_err());
        preference.source = ToolSource::Managed;
        manager
            .set_preference(ToolId::Mpv, preference)
            .await
            .unwrap();
        fake_install(&manager, ToolId::Mpv, "mpv-new");
        manager.update("mpv-new", &settings).await.unwrap();
        let state = manager.state();
        assert_eq!(state.tools[&ToolId::Mpv].active.as_deref(), Some("mpv-new"));
        assert_eq!(
            state.tools[&ToolId::Mpv].previous.as_deref(),
            Some("mpv-old")
        );
        assert!(
            !manager
                .available_updates(&settings)
                .iter()
                .any(|u| u.tool == ToolId::Mpv)
        );
    }

    #[tokio::test]
    async fn automatic_updates_recheck_queued_preference_changes() {
        use std::{future::Future, task::Poll};

        for change in ["pin", "source", "policy", "channel", "rollback", "active"] {
            let directory = tempfile::tempdir().unwrap();
            let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let events = Arc::clone(&completed);
            let manager = ToolManager::new(
                directory.path().to_path_buf(),
                Some(Box::new(move |operation| {
                    if operation.phase == "complete" {
                        events.fetch_add(1, Ordering::SeqCst);
                    }
                })),
            )
            .unwrap();
            let settings = AppSettings::default();
            let checked = "2026-09-12T07:00:00Z";
            for version in ["old", "new"] {
                fake_install(&manager, ToolId::Mpv, version);
            }
            manager.activate(ToolId::Mpv, "old").await.unwrap();
            if change == "rollback" {
                manager.activate(ToolId::Mpv, "new").await.unwrap();
            }
            manager
                .edit(|state| {
                    state.catalog = Some(Catalog {
                        packages: vec![
                            state
                                .installed
                                .iter()
                                .find(|i| i.package.id == "new")
                                .unwrap()
                                .package
                                .clone(),
                        ],
                    });
                    state.last_checked = Some(checked.into());
                    state.upstream_checked.insert(ToolId::Mpv, checked.into());
                    Ok(())
                })
                .unwrap();
            let guard = manager.operations.lock().await;
            let preference_change = async {
                if change == "rollback" {
                    return manager.rollback(ToolId::Mpv).await;
                }
                if change == "active" {
                    return manager.activate(ToolId::Mpv, "new").await;
                }
                let mut p = manager.state().tools[&ToolId::Mpv].clone();
                match change {
                    "pin" => p.pinned = true,
                    "source" => p.source = ToolSource::System,
                    "policy" => p.update_policy = UpdatePolicy::Notify,
                    "channel" => p.channel = "preview".into(),
                    _ => unreachable!(),
                }
                manager.set_preference(ToolId::Mpv, p).await
            };
            // Preferences and local selections do not wait behind a package download.
            tokio::time::timeout(Duration::from_secs(1), preference_change)
                .await
                .unwrap()
                .unwrap();
            let mut automatic =
                Box::pin(manager.auto_update_tool(ToolId::Mpv, &settings, Some(checked)));
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(automatic.as_mut().poll(cx).is_pending()))
                    .await
            );
            drop(guard);
            automatic.await.unwrap();
            assert_eq!(
                manager.state().tools[&ToolId::Mpv].active.as_deref(),
                Some(if change == "active" { "new" } else { "old" }),
                "{change}"
            );
            assert_eq!(completed.load(Ordering::SeqCst), 0, "{change}");
        }
    }

    #[tokio::test]
    async fn automatic_update_installs_a_current_eligible_candidate_only() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let settings = AppSettings::default();
        let checked = "2026-09-12T07:00:00Z";
        for version in ["old", "new"] {
            fake_install(&manager, ToolId::Mpv, version);
        }
        manager.activate(ToolId::Mpv, "old").await.unwrap();
        manager
            .edit(|state| {
                state.catalog = Some(Catalog {
                    packages: vec![
                        state
                            .installed
                            .iter()
                            .find(|i| i.package.id == "new")
                            .unwrap()
                            .package
                            .clone(),
                    ],
                });
                state.last_checked = Some(checked.into());
                state.upstream_checked.insert(ToolId::Mpv, checked.into());
                Ok(())
            })
            .unwrap();
        manager
            .auto_update_tool(ToolId::Mpv, &settings, Some("outdated check"))
            .await
            .unwrap();
        assert_eq!(
            manager.state().tools[&ToolId::Mpv].active.as_deref(),
            Some("old")
        );
        manager
            .auto_update_tool(ToolId::Mpv, &settings, Some(checked))
            .await
            .unwrap();
        let state = manager.state();
        assert_eq!(state.tools[&ToolId::Mpv].active.as_deref(), Some("new"));
        assert_eq!(state.tools[&ToolId::Mpv].previous.as_deref(), Some("old"));
    }

    #[tokio::test]
    async fn rate_limit_cooldown_survives_restart_without_touching_cached_packages() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        manager
            .edit(|s| {
                upstream::merge(
                    s,
                    ToolId::Mpv,
                    upstream::Discovery {
                        packages: vec![providers::test_package(ToolId::Mpv)],
                        etag: None,
                    },
                    &crate::utils::utc_now(),
                );
                s.upstream_retry_at =
                    Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
                Ok(())
            })
            .unwrap();
        let cached = serde_json::to_value(manager.state().catalog).unwrap();
        drop(manager);
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        assert!(
            manager
                .refresh_catalog()
                .await
                .unwrap_err()
                .message
                .contains("paused")
        );
        assert_eq!(
            serde_json::to_value(manager.state().catalog).unwrap(),
            cached
        );
        assert_eq!(manager.state().upstream_checked.len(), 1);
    }
}
