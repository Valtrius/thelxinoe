use super::models::{Registry, ToolId};
use crate::error::{AppError, AppResult};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::{fs, path::Path};

pub fn load(root: &Path) -> AppResult<Registry> {
    let connection = connect(root)?;
    let value: Option<String> = connection
        .query_row("SELECT value FROM manager_state WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|s| parse(&s))
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn save(root: &Path, state: &Registry) -> AppResult<()> {
    let value =
        serde_json::to_string(state).map_err(|error| AppError::internal(error.to_string()))?;
    write(root, &value)
}
fn write(root: &Path, value: &str) -> AppResult<()> {
    let connection = connect(root)?;
    connection.execute(
        "INSERT INTO manager_state(id, value) VALUES(1, ?1) \
         ON CONFLICT(id) DO UPDATE SET value = excluded.value",
        [value],
    )?;
    Ok(())
}
/// Earlier builds also managed yt-dlp, Streamlink, FFmpeg and Deno, and offered a
/// preview channel. Drop what this build no longer knows, so those installs still start;
/// startup then removes the packages that are no longer registered.
fn parse(text: &str) -> AppResult<Registry> {
    let invalid = |error: serde_json::Error| AppError::internal(error.to_string());
    let mut state: Value = serde_json::from_str(text).map_err(invalid)?;
    let known = |id: &str| serde_json::from_value::<ToolId>(id.into()).is_ok();
    for map in ["/tools", "/upstreamEtags", "/upstreamChecked"] {
        if let Some(map) = state.pointer_mut(map).and_then(Value::as_object_mut) {
            map.retain(|id, _| known(id));
        }
    }
    for (list, tool) in [
        ("/installed", "/package/tool"),
        ("/catalog/packages", "/tool"),
    ] {
        if let Some(list) = state.pointer_mut(list).and_then(Value::as_array_mut) {
            list.retain(|entry| {
                entry
                    .pointer(tool)
                    .and_then(Value::as_str)
                    .is_some_and(known)
            });
        }
    }
    // Packages only ever came from the recommended channel; preview found no updates.
    if let Some(tools) = state.get_mut("tools").and_then(Value::as_object_mut) {
        for preference in tools.values_mut() {
            if preference.get("channel").and_then(Value::as_str) == Some("preview") {
                preference["channel"] = "recommended".into();
            }
        }
    }
    serde_json::from_value(state).map_err(invalid)
}
fn connect(root: &Path) -> AppResult<Connection> {
    fs::create_dir_all(root)?;
    let conn = Connection::open(root.join("tools.db"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA busy_timeout=5000;
         CREATE TABLE IF NOT EXISTS manager_state(
             id INTEGER PRIMARY KEY CHECK(id=1),
             value TEXT NOT NULL
         );",
    )?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        ToolManager,
        models::{ToolPreference, ToolSource},
        providers::test_package,
        test_support::fake_install,
    };
    use serde_json::json;

    #[test]
    fn registries_of_builds_with_removed_tools_and_the_preview_channel_still_load() {
        let directory = tempfile::tempdir().unwrap();
        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        fake_install(&manager, ToolId::Mpv, "mpv-kept");
        manager
            .activate_locked(ToolId::Mpv, "mpv-kept", false)
            .unwrap();
        let mut legacy = serde_json::to_value(manager.state()).unwrap();
        drop(manager);
        let checked = crate::utils::utc_now();
        legacy["tools"]["mpv"]["channel"] = "preview".into();
        legacy["catalog"] = json!({"packages": [test_package(ToolId::Mpv)]});
        legacy["upstreamEtags"]["mpv"] = "mpv-etag".into();
        legacy["upstreamChecked"]["mpv"] = checked.clone().into();
        for id in ["ytdlp", "streamlink", "ffmpeg", "deno"] {
            legacy["tools"][id] = serde_json::to_value(ToolPreference {
                source: ToolSource::Custom,
                custom_path: Some(format!(r"C:\Tools\{id}.exe")),
                channel: "preview".into(),
                ..Default::default()
            })
            .unwrap();
            legacy["upstreamEtags"][id] = "etag".into();
            legacy["upstreamChecked"][id] = checked.clone().into();
            let mut package = serde_json::to_value(test_package(ToolId::Mpv)).unwrap();
            package["id"] = format!("{id}-old").into();
            package["tool"] = id.into();
            package["dependencies"] = json!(["deno"]);
            legacy["catalog"]["packages"]
                .as_array_mut()
                .unwrap()
                .push(package.clone());
            let directory_name = format!("packages/{id}/{id}-old");
            fs::create_dir_all(directory.path().join(&directory_name)).unwrap();
            legacy["installed"].as_array_mut().unwrap().push(json!({
                "package": package,
                "installedAt": checked,
                "directory": directory_name,
                "versionOutput": "old",
            }));
        }
        write(directory.path(), &legacy.to_string()).unwrap();

        let manager = ToolManager::new(directory.path().to_path_buf(), None).unwrap();
        let state = manager.state();
        assert_eq!(state.tools.keys().collect::<Vec<_>>(), [&ToolId::Mpv]);
        let mpv = &state.tools[&ToolId::Mpv];
        assert_eq!(mpv.active.as_deref(), Some("mpv-kept"));
        assert_eq!(mpv.channel, "recommended");
        assert_eq!(
            state
                .installed
                .iter()
                .map(|i| i.package.id.as_str())
                .collect::<Vec<_>>(),
            ["mpv-kept"]
        );
        assert_eq!(state.catalog.as_ref().unwrap().packages.len(), 1);
        assert_eq!(state.upstream_etags.len(), 1);
        assert_eq!(state.upstream_checked.len(), 1);
        for id in ["ytdlp", "streamlink", "ffmpeg", "deno"] {
            assert!(
                !directory
                    .path()
                    .join("packages")
                    .join(id)
                    .join(format!("{id}-old"))
                    .exists()
            );
        }
        // The cleaned registry is what the next start reads.
        assert_eq!(
            serde_json::to_value(load(directory.path()).unwrap()).unwrap(),
            serde_json::to_value(state).unwrap()
        );
    }
}
