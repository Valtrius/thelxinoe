mod archive;
#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod runtime;
pub mod tools;

use serde_json::{Value, json};
use tauri::Manager;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct DesktopPlayback {
    pub tool_lock: Mutex<()>,
    #[cfg(windows)]
    pub player: runtime::Player,
}
fn store(app: &tauri::AppHandle) -> Result<tools::ToolStore, String> {
    tools::ToolStore::new(
        app.path()
            .app_local_data_dir()
            .map_err(|e| e.to_string())?
            .join("mpv"),
    )
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn mpv_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let store = store(&app)?;
    let selection = store.selection().map_err(|e| e.to_string())?;
    let config = store.configuration();
    let text = std::fs::read_to_string(config.join("mpv.conf")).unwrap_or_default();
    let plugins = std::fs::read_dir(config.join("scripts"))
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "lua"))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    Ok(
        json!({"selection":selection,"configuration":config,"text":text,"plugins":plugins,"product_version":env!("CARGO_PKG_VERSION"),"updates":"unpublished"}),
    )
}
#[tauri::command]
pub async fn mpv_install(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopPlayback>,
) -> Result<Value, String> {
    let _guard = state.tool_lock.lock().await;
    let selection = store(&app)?.install().await.map_err(|e| e.to_string())?;
    Ok(json!(selection))
}
#[tauri::command]
pub async fn mpv_custom(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopPlayback>,
    path: String,
) -> Result<Value, String> {
    let _guard = state.tool_lock.lock().await;
    Ok(json!(
        store(&app)?
            .select_custom(path.into())
            .await
            .map_err(|e| e.to_string())?
    ))
}
#[tauri::command]
pub async fn mpv_configuration(app: tauri::AppHandle, text: String) -> Result<(), String> {
    if text.len() > 64 * 1024 {
        return Err("MPV configuration exceeds 64 KiB".into());
    }
    std::fs::write(store(&app)?.configuration().join("mpv.conf"), text).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn mpv_plugin(
    app: tauri::AppHandle,
    name: String,
    text: Option<String>,
) -> Result<(), String> {
    if name.len() > 100
        || !name.ends_with(".lua")
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("Use a simple .lua plugin filename".into());
    }
    let path = store(&app)?.configuration().join("scripts").join(name);
    match text {
        Some(text) => {
            if text.len() > 512 * 1024 {
                return Err("Plugin exceeds 512 KiB".into());
            }
            std::fs::write(path, text).map_err(|e| e.to_string())
        }
        None => std::fs::remove_file(path).map_err(|e| e.to_string()),
    }
}

#[cfg(windows)]
#[tauri::command]
pub async fn mpv_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopPlayback>,
    choice: runtime::Choice,
    queue: Option<Vec<runtime::Choice>>,
    music: bool,
) -> Result<runtime::View, String> {
    let mut choices = queue
        .filter(|q| music && !q.is_empty())
        .unwrap_or_else(|| vec![choice.clone()]);
    let index = choices.iter().position(|c| c.id == choice.id).unwrap_or(0);
    choices = choices.into_iter().skip(index).collect();
    choices[0] = choice;
    state
        .player
        .play(app.clone(), store(&app)?, choices, music)
        .await
        .map_err(|e| e.to_string())
}
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_state(state: tauri::State<'_, DesktopPlayback>) -> Result<runtime::View, String> {
    Ok(state.player.view().await)
}
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_command(
    state: tauri::State<'_, DesktopPlayback>,
    command: String,
    value: Option<f64>,
) -> Result<(), String> {
    let control = match command.as_str() {
        "pause" => runtime::Control::Pause,
        "next" => runtime::Control::Next,
        "stop" => {
            state.player.stop().await;
            return Ok(());
        }
        "seek" => runtime::Control::Seek(
            value
                .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1_209_600.0)
                .ok_or("Invalid seek position")?,
        ),
        "volume" => runtime::Control::Volume(
            value
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .ok_or("Invalid volume")?,
        ),
        _ => return Err("Unknown player control".into()),
    };
    state
        .player
        .control(control)
        .await
        .map_err(|e| e.to_string())
}
