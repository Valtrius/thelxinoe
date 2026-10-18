#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod runtime;

use tauri::Manager;
#[derive(Default)]
pub struct DesktopPlayback {
    #[cfg(windows)]
    pub player: runtime::Player,
}
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_play(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopPlayback>,
    choice: runtime::Choice,
    queue: Option<Vec<runtime::Choice>>,
    music: bool,
    run: Option<String>,
) -> Result<runtime::View, String> {
    let mut choices = queue
        .filter(|q| music && !q.is_empty())
        .unwrap_or_else(|| vec![choice.clone()]);
    let index = choices
        .iter()
        .position(|c| {
            if let Some(queue) = &choice.queue_context {
                c.queue_context
                    .as_ref()
                    .is_some_and(|q| q["index"] == queue["index"])
            } else {
                c.id == choice.id
            }
        })
        .unwrap_or(0);
    choices = choices.into_iter().skip(index).collect();
    choices[0] = choice;
    let tools = app
        .state::<crate::tools::DesktopTools>()
        .manager()
        .map_err(|e| e.to_string())?
        .clone();
    state
        .player
        .play(app.clone(), tools, run, choices, music)
        .await
        .map_err(|e| e.to_string())
}
/// One run, or the most recently started one.
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_state(
    state: tauri::State<'_, DesktopPlayback>,
    run: Option<String>,
) -> Result<runtime::View, String> {
    Ok(state.player.view(run.as_deref()).await)
}
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_sessions(
    state: tauri::State<'_, DesktopPlayback>,
) -> Result<Vec<runtime::View>, String> {
    Ok(state.player.views().await)
}
/// Controls one run, or the most recently started one. Stop without a run stops all.
#[cfg(windows)]
#[tauri::command]
pub async fn mpv_command(
    state: tauri::State<'_, DesktopPlayback>,
    command: String,
    value: Option<f64>,
    run: Option<String>,
) -> Result<(), String> {
    let control = match command.as_str() {
        "pause" => runtime::Control::Pause,
        "next" => runtime::Control::Next,
        "stop" => {
            state.player.stop(run.as_deref()).await;
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
        .control(run.as_deref(), control)
        .await
        .map_err(|e| e.to_string())
}
