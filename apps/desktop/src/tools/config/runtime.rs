use super::*;

impl ToolManager {
    pub(in crate::tools) fn prepare_plugin_config(
        &self,
        package: &crate::tools::models::Package,
    ) -> AppResult<()> {
        if !package.tool.plugin() {
            return Ok(());
        }
        let root = self.config_root().join("script-opts");
        fs::create_dir_all(&root)?;
        let (name, text) = match package.tool {
            ToolId::Thumbfast => (
                "thumbfast.conf",
                "# Enable thumbnails for network videos. Set no to disable.\nnetwork=yes\n",
            ),
            ToolId::SubSelect => (
                "sub-select.json",
                "[\n  { \"alang\": \"*\", \"slang\": [\"forced\", \"default\", \"no\"] }\n]\n",
            ),
            _ => return Ok(()),
        };
        let path = root.join(name);
        if !path.exists() {
            let mut file = fs::File::options()
                .create_new(true)
                .write(true)
                .open(path)?;
            std::io::Write::write_all(&mut file, text.as_bytes())?;
        }
        Ok(())
    }

    pub(in crate::tools) fn player_config_arguments(
        &self,
        state: &Registry,
    ) -> AppResult<(Vec<String>, Option<PathBuf>)> {
        let mut args = Vec::new();
        let mut snapshot = None;
        match state.mpv.source {
            MpvConfigSource::Native => {}
            MpvConfigSource::Directory => {
                let directory = state
                    .mpv
                    .directory
                    .as_ref()
                    .filter(|p| Path::new(p).is_dir())
                    .ok_or_else(|| {
                        AppError::validation("The selected MPV configuration directory is missing.")
                    })?;
                args.push(format!("--config-dir={directory}"));
            }
            MpvConfigSource::Managed => {
                let directory = self
                    .root
                    .join("runtime")
                    .join(uuid::Uuid::new_v4().to_string());
                fs::create_dir_all(&directory)?;
                if let Err(e) = copy_tree(&self.config_root(), &directory) {
                    let _ = self.remove_owned(&directory);
                    return Err(e);
                }
                args.push(format!("--config-dir={}", directory.display()));
                snapshot = Some(directory.clone());
                for id in ToolId::ALL.into_iter().filter(|id| id.plugin()) {
                    let p = Self::preference(state, id, &AppSettings::default());
                    // Off suppresses local copies too, but only in the disposable snapshot.
                    if !p.enabled || p.source == crate::tools::models::ToolSource::Managed {
                        for name in Self::plugin_script_names(id) {
                            let imported = directory.join("scripts").join(name);
                            if imported.is_dir() {
                                self.remove_owned(&imported)?;
                            } else if imported.is_file() {
                                fs::remove_file(imported)?;
                            }
                        }
                    }
                    if p.enabled
                        && p.source == crate::tools::models::ToolSource::Managed
                        && let Some(installed) = state
                            .installed
                            .iter()
                            .find(|i| Some(&i.package.id) == p.active.as_ref())
                    {
                        let entry = self
                            .root
                            .join(&installed.directory)
                            .join(&installed.package.entry);
                        let script = if id == ToolId::Uosc {
                            entry
                                .parent()
                                .ok_or_else(|| AppError::internal("Missing uosc directory"))?
                                .to_path_buf()
                        } else {
                            entry
                        };
                        args.push(format!("--scripts-append={}", script.display()));
                        if id == ToolId::Uosc {
                            self.prepare_uosc_snapshot(&directory)?;
                            args.push("--osc=no".into());
                            args.push("--osd-bar=no".into());
                            let bridge = directory
                                .join(format!("thelxinoe-bridge-{}.lua", uuid::Uuid::new_v4()));
                            let managed =
                                serde_json::to_string(&self.config_root().display().to_string())
                                    .map_err(|e| AppError::internal(e.to_string()))?;
                            fs::write(
                                &bridge,
                                include_str!("../mpv-bridge.lua")
                                    .replace("__MANAGED_CONFIG_JSON__", &managed),
                            )?;
                            args.push(format!("--scripts-append={}", bridge.display()));
                        }
                    }
                }
            }
        }
        Ok((args, snapshot))
    }

    pub(in crate::tools) fn prepare_uosc_snapshot(&self, directory: &Path) -> AppResult<()> {
        // Script option files are read in order, then MPV's script-opts are
        // applied. Prepending a default preserves every explicit user override.
        // Keep downloaded media outside both the config copy and runtime cleanup.
        let subtitles = self.root.join("data").join("uosc").join("subtitles");
        fs::create_dir_all(&subtitles)?;
        let path = directory.join("script-opts").join("uosc.conf");
        let text = read_text(&path)?;
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| AppError::internal("Missing script options directory"))?,
        )?;
        fs::write(
            path,
            format!("subtitles_directory={}\n{text}", subtitles.display()),
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::fake_install;
    use crate::tools::*;

    #[tokio::test]
    async fn managed_plugins_replace_only_their_imported_scripts_in_each_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let plugins = [ToolId::Uosc, ToolId::Thumbfast, ToolId::SubSelect];
        for id in plugins {
            fake_install(&manager, id, id.key());
            manager.activate(id, id.key()).await.unwrap();
        }
        let source = tempfile::tempdir().unwrap();
        let files = [
            ("mpv.conf", "volume=42\n"),
            ("input.conf", "SPACE cycle pause\n"),
            ("scripts/uosc/main.lua", "-- imported uosc\n"),
            ("scripts/uosc.lua", "-- legacy uosc\n"),
            ("scripts/thumbfast.lua", "-- imported thumbfast\n"),
            ("scripts/sub-select.lua", "-- imported sub-select\n"),
            ("scripts/mpvSockets.lua", "-- unrelated script\n"),
            ("scripts/uosc-helper.lua", "-- unrelated helper\n"),
            ("script-opts/uosc.conf", "scale=1.7\n"),
            ("script-opts/thumbfast.conf", "network=no\n"),
            (
                "script-opts/sub_select.conf",
                "observe_audio_switches=yes\n",
            ),
            ("script-opts/sub-select.json", "[]\n"),
        ];
        for (name, text) in files {
            let path = source.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        manager
            .import_config(source.path().to_str().unwrap())
            .await
            .unwrap();

        // Off suppresses both copies in snapshots without deleting the saved imports.
        for mask in [0, 1, 2, 4, 7, 0] {
            manager
                .edit(|state| {
                    for (index, id) in plugins.iter().enumerate() {
                        state.tools.get_mut(id).unwrap().enabled = mask & (1 << index) != 0;
                    }
                    Ok(())
                })
                .unwrap();
            let (_, runtime) = manager
                .resolve_launch(&AppSettings::default(), &[], true)
                .await
                .unwrap();
            let snapshot = PathBuf::from(
                runtime
                    .mpv_args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("--config-dir="))
                    .unwrap(),
            );
            let scripts: Vec<_> = runtime
                .mpv_args
                .iter()
                .filter_map(|arg| arg.strip_prefix("--scripts-append="))
                .filter(|path| !path.contains("thelxinoe-bridge-"))
                .collect();
            assert_eq!(scripts.len(), (mask as u32).count_ones() as usize);
            for (index, id) in plugins.iter().enumerate() {
                let enabled = mask & (1 << index) != 0;
                let imported = if *id == ToolId::Uosc {
                    "scripts/uosc/main.lua".to_owned()
                } else {
                    format!("scripts/{}.lua", id.key())
                };
                assert!(!snapshot.join(imported).exists());
                assert_eq!(
                    scripts.iter().any(|path| Path::new(path)
                        .starts_with(manager.root.join("packages").join(id.key()))),
                    enabled
                );
            }
            assert!(!snapshot.join("scripts/uosc.lua").exists());
            for (name, text) in files {
                assert_eq!(fs::read_to_string(source.path().join(name)).unwrap(), text);
                assert_eq!(
                    fs::read_to_string(manager.config_root().join(name)).unwrap(),
                    text
                );
                if !name.starts_with("scripts/")
                    || name.ends_with("mpvSockets.lua")
                    || name.ends_with("uosc-helper.lua")
                {
                    let saved = fs::read_to_string(snapshot.join(name)).unwrap();
                    // uosc prepends its existing subtitle-directory default.
                    assert_eq!(
                        if name == "script-opts/uosc.conf" && mask & 1 != 0 {
                            saved.split_once('\n').unwrap().1
                        } else {
                            &saved
                        },
                        text
                    );
                }
            }
            drop(runtime);
            assert!(!snapshot.exists());
        }
    }

    #[tokio::test]
    #[ignore = "Requires THELXINOE_MPV_TEST_EXE pointing to a Windows MPV executable"]
    async fn real_mpv_loads_one_plugin_copy_with_the_saved_settings() {
        let executable = std::env::var("THELXINOE_MPV_TEST_EXE").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let plugins = [ToolId::Uosc, ToolId::Thumbfast, ToolId::SubSelect];
        let config = manager.config_root();
        fs::create_dir_all(config.join("script-opts")).unwrap();
        for id in plugins {
            fake_install(&manager, id, id.key());
            manager.activate(id, id.key()).await.unwrap();
            let installed = manager
                .state()
                .installed
                .into_iter()
                .find(|p| p.package.tool == id)
                .unwrap();
            let imported = config.join("scripts").join(if id == ToolId::Uosc {
                "uosc/main.lua".to_owned()
            } else {
                format!("{}.lua", id.key())
            });
            let managed = manager
                .root
                .join(installed.directory)
                .join(installed.package.entry);
            for (path, source) in [(imported, "imported"), (managed, "managed")] {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                // Each loaded copy reports under its actual mpv script name,
                // so duplicate instances cannot overwrite one another's result.
                fs::write(
                    path,
                    format!(
                        r#"
local options = {{ marker = 'default', subtitles_directory = '' }}
require('mp.options').read_options(options, '{name}')
mp.set_property_native('user-data/thelxinoe-plugin-probe/' .. mp.get_script_name(), {{
    plugin = '{name}', source = '{source}', setting = options.marker
}})
"#,
                        name = id.key()
                    ),
                )
                .unwrap();
            }
            fs::write(
                config
                    .join("script-opts")
                    .join(format!("{}.conf", id.key())),
                "marker=my saved settings\n",
            )
            .unwrap();
        }
        fs::write(
            config.join("scripts/mpvSockets.lua"),
            "mp.set_property('user-data/thelxinoe-unrelated-script', 'loaded')\n",
        )
        .unwrap();
        manager
            .set_mpv_preferences(MpvPreferences {
                source: MpvConfigSource::Managed,
                directory: None,
            })
            .await
            .unwrap();
        for enabled in [true, false] {
            manager
                .edit(|state| {
                    for id in plugins {
                        let p = state.tools.get_mut(&id).unwrap();
                        p.enabled = true;
                        p.source = if enabled {
                            crate::tools::models::ToolSource::Managed
                        } else {
                            crate::tools::models::ToolSource::Custom
                        };
                    }
                    Ok(())
                })
                .unwrap();
            let (_, runtime) = manager
                .resolve_launch(&AppSettings::default(), &[], true)
                .await
                .unwrap();
            let mut probe = Probe::start_mode(&executable, &runtime.mpv_args, true)
                .await
                .unwrap();
            let loaded = probe
                .request(json!(["get_property", "user-data/thelxinoe-plugin-probe"]))
                .await;
            let unrelated = probe
                .request(json!([
                    "get_property",
                    "user-data/thelxinoe-unrelated-script"
                ]))
                .await;
            let log = probe.finish().await.unwrap();
            let loaded = loaded.unwrap_or_else(|error| panic!("{error:?}\n{log}"));
            let copies = loaded.as_object().unwrap();
            assert_eq!(copies.len(), plugins.len(), "{loaded}\n{log}");
            for id in plugins {
                let matching: Vec<_> = copies
                    .values()
                    .filter(|copy| copy["plugin"] == id.key())
                    .collect();
                assert_eq!(matching.len(), 1, "{loaded}");
                assert_eq!(
                    matching[0]["source"],
                    if enabled { "managed" } else { "imported" }
                );
                assert_eq!(matching[0]["setting"], "my saved settings");
            }
            assert_eq!(unrelated.unwrap(), "loaded");
        }
    }

    #[tokio::test]
    async fn managed_uosc_subtitles_outlive_player_snapshots_and_startup_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        fake_install(&manager, ToolId::Uosc, "uosc");
        manager.activate(ToolId::Uosc, "uosc").await.unwrap();
        manager
            .set_mpv_preferences(MpvPreferences {
                source: MpvConfigSource::Managed,
                directory: None,
            })
            .await
            .unwrap();
        let config = directory.path().join("config/mpv/script-opts/uosc.conf");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let custom = "# Keep my destination\r\nsubtitles_directory=!D:/My subtitles\r\n";
        for text in ["", custom] {
            fs::write(&config, text).unwrap();
            let (_, runtime) = manager
                .resolve_launch(&AppSettings::default(), &[ToolId::Mpv], true)
                .await
                .unwrap();
            let snapshot = PathBuf::from(
                runtime
                    .mpv_args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("--config-dir="))
                    .unwrap(),
            );
            let options = fs::read_to_string(snapshot.join("script-opts/uosc.conf")).unwrap();
            let (default, rest) = options.split_once('\n').unwrap();
            assert_eq!(rest, text);
            let destination = PathBuf::from(default.strip_prefix("subtitles_directory=").unwrap());
            assert_eq!(destination, directory.path().join("data/uosc/subtitles"));
            fs::write(destination.join("downloaded.srt"), "subtitle").unwrap();
            assert_eq!(fs::read_to_string(&config).unwrap(), text);
            drop(runtime);
            assert!(!snapshot.exists());
            assert!(destination.join("downloaded.srt").is_file());
        }
        drop(manager);
        let _manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        assert!(
            directory
                .path()
                .join("data/uosc/subtitles/downloaded.srt")
                .is_file()
        );
    }
}
