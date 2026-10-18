use super::{
    config::{ConfigDocument, MpvSchema},
    models::{MpvPreferences, ToolId, ToolPreference, ToolUpdate, ToolsSnapshot},
};
use crate::{error::AppResult, tools::DesktopTools};
use tauri::{AppHandle, Emitter, State};

async fn changed(app: &AppHandle, _state: &DesktopTools) {
    let _ = app.emit("tools-changed", ());
}
#[tauri::command]
pub async fn tools_get(state: State<'_, DesktopTools>) -> AppResult<ToolsSnapshot> {
    let mut snapshot = state.manager()?.snapshot(&Default::default())?;
    snapshot
        .tools
        .retain(|tool| ToolId::DESKTOP.contains(&tool.id));
    Ok(snapshot)
}

#[tauri::command]
pub async fn tools_check(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    tool: ToolId,
) -> AppResult<()> {
    if tool.plugin() {
        return Err(crate::error::AppError::validation(
            "Use the MPV configuration test to check plugins.",
        ));
    }
    let settings = crate::models::AppSettings::default();
    state.manager()?.invalidate_tool_diagnostic(tool);
    state.manager()?.diagnostic(tool, &settings).await;
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn tools_updates(state: State<'_, DesktopTools>) -> AppResult<Vec<ToolUpdate>> {
    Ok(state.manager()?.available_updates(&Default::default()))
}
#[tauri::command]
pub async fn tools_update(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    package_id: String,
) -> AppResult<()> {
    let result = state
        .manager()?
        .update(&package_id, &Default::default())
        .await;
    changed(&app, &state).await;
    result
}
#[tauri::command]
pub async fn tools_set_preference(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    tool: ToolId,
    preference: ToolPreference,
) -> AppResult<()> {
    state.manager()?.set_preference(tool, preference).await?;
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn tools_refresh(app: AppHandle, state: State<'_, DesktopTools>) -> AppResult<()> {
    let result = state.manager()?.refresh_catalog().await;
    changed(&app, &state).await;
    result
}
#[tauri::command]
pub async fn tools_install(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    package_id: String,
) -> AppResult<()> {
    let result = state
        .manager()?
        .install(&package_id, &Default::default())
        .await;
    changed(&app, &state).await;
    result
}
#[tauri::command]
pub async fn tools_activate(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    tool: ToolId,
    package_id: String,
) -> AppResult<()> {
    state.manager()?.activate(tool, &package_id).await?;
    if !tool.plugin() {
        state.manager()?.diagnostic(tool, &Default::default()).await;
    }
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn tools_rollback(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    tool: ToolId,
) -> AppResult<()> {
    state.manager()?.rollback(tool).await?;
    if !tool.plugin() {
        state.manager()?.diagnostic(tool, &Default::default()).await;
    }
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn tools_remove(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    package_id: String,
) -> AppResult<()> {
    state.manager()?.remove(&package_id).await?;
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn tools_repair(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    package_id: String,
) -> AppResult<()> {
    let result = state.manager()?.repair(&package_id).await;
    if result.is_ok() {
        let snapshot = state.manager()?.snapshot(&Default::default())?;
        if let Some(tool) = snapshot.tools.iter().find(|tool| {
            !tool.id.plugin()
                && tool.preference.source == super::models::ToolSource::Managed
                && tool.preference.active.as_deref() == Some(&package_id)
        }) {
            state
                .manager()?
                .diagnostic(tool.id, &Default::default())
                .await;
        }
    }
    changed(&app, &state).await;
    result
}
#[tauri::command]
pub async fn mpv_set_preferences(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    preferences: MpvPreferences,
) -> AppResult<()> {
    state.manager()?.set_mpv_preferences(preferences).await?;
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn mpv_config_files(state: State<'_, DesktopTools>) -> AppResult<Vec<String>> {
    state.manager()?.config_files()
}
#[tauri::command]
pub async fn mpv_config_read(
    state: State<'_, DesktopTools>,
    name: String,
) -> AppResult<ConfigDocument> {
    state.manager()?.config_document(&name)
}
#[tauri::command]
pub async fn mpv_config_save(
    state: State<'_, DesktopTools>,
    name: String,
    text: String,
    revision: String,
) -> AppResult<ConfigDocument> {
    state
        .manager()?
        .save_config_document(&name, &text, &revision, &Default::default())
        .await
}
#[tauri::command]
pub async fn mpv_config_restore(
    state: State<'_, DesktopTools>,
    name: String,
    revision: String,
) -> AppResult<ConfigDocument> {
    state
        .manager()?
        .restore_config_document(&name, &revision, &Default::default())
        .await
}
#[tauri::command]
pub async fn mpv_config_import(
    app: AppHandle,
    state: State<'_, DesktopTools>,
    directory: String,
) -> AppResult<()> {
    state.manager()?.import_config(&directory).await?;
    changed(&app, &state).await;
    Ok(())
}
#[tauri::command]
pub async fn mpv_plugin_import(state: State<'_, DesktopTools>, path: String) -> AppResult<()> {
    state.manager()?.import_plugin(&path).await
}

#[tauri::command]
pub async fn mpv_open_configuration_directory(state: State<'_, DesktopTools>) -> AppResult<()> {
    let directory = state.manager()?.mpv_configuration_directory()?;
    tokio::task::spawn_blocking(move || open::that_detached(directory))
        .await
        .map_err(|error| crate::error::AppError::internal(error.to_string()))?
        .map_err(|error| crate::error::AppError::validation(error.to_string()))
}
#[tauri::command]
pub async fn mpv_options(state: State<'_, DesktopTools>) -> AppResult<MpvSchema> {
    state.manager()?.mpv_schema(&Default::default()).await
}
#[tauri::command]
pub async fn mpv_test_configuration(
    state: State<'_, DesktopTools>,
    clean: bool,
) -> AppResult<String> {
    state
        .manager()?
        .test_mpv_configuration(&Default::default(), clean)
        .await
}
