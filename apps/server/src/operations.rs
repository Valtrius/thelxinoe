//! Administrative observations never include credentials or arbitrary upstream payloads.

#[path = "storage/operations.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, put},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/me/notifications", get(notifications))
        .route("/api/v1/me/notifications/{id}", put(read))
        .route("/api/v1/admin/operations", get(dashboard))
        .route("/api/v1/admin/diagnostics", get(diagnostics))
        .route("/api/v1/admin/devices", get(devices))
        .route("/api/v1/admin/backups", get(backups).post(backup))
        .route(
            "/api/v1/admin/backups/{id}/restore",
            axum::routing::post(restore),
        )
        .route("/api/v1/users/{id}", put(change_user).delete(delete_user))
}
async fn backups(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(
        crate::managers::controller_request(&state, "/backups", None).await?,
    ))
}
#[derive(serde::Deserialize)]
struct BackupInput {
    passphrase: String,
    #[serde(default)]
    confirm: bool,
}
async fn backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BackupInput>,
) -> Result<Json<Value>> {
    backup_command(state, headers, input, None).await
}
async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<BackupInput>,
) -> Result<Json<Value>> {
    if uuid::Uuid::parse_str(&key).is_err() {
        return Err(ApiError::bad("Invalid backup ID"));
    }
    backup_command(state, headers, input, Some(key)).await
}
async fn backup_command(
    state: AppState,
    headers: HeaderMap,
    input: BackupInput,
    restore: Option<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !input.confirm {
        return Err(ApiError::bad("Confirm the temporary service interruption"));
    }
    let busy = storage::backup_command_read_playback_sessions(&state.db).await?;
    if busy {
        return Err(ApiError::conflict(
            "Wait for active playback and background work to finish",
        ));
    }
    let path = restore
        .as_ref()
        .map_or("/backups".into(), |key| format!("/backups/{key}/restore"));
    storage::backup_command_write_audit(p, &state.db, restore).await?;
    Ok(Json(
        crate::managers::controller_request(
            &state,
            &path,
            Some(json!({"passphrase":input.passphrase})),
        )
        .await?,
    ))
}
#[derive(serde::Deserialize)]
struct UserChange {
    role: thelxinoe_core::Role,
    password: Option<String>,
}
async fn change_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(mut input): Json<UserChange>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = if let Some(password) = input.password.take() {
        thelxinoe_auth::validate_credentials("valid-user", &password)
            .map_err(|e| ApiError::bad(e.to_string()))?;
        Some(thelxinoe_auth::password_hash(password).await?)
    } else {
        None
    };
    let status = storage::change_user(&state.db, key, input, p, hash).await?;
    match status {
        404 => Err(ApiError::not_found()),
        409 => Err(ApiError::conflict("Keep at least one administrator")),
        _ => Ok(Json(json!({"saved":true}))),
    }
}
async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    if key == p.user.id {
        return Err(ApiError::conflict(
            "Use another administrator account to delete this account",
        ));
    }
    let _lease = state.media_operations.write().await;
    let result = storage::delete_user(&state.db, key, p)
        .await?
        .ok_or_else(ApiError::not_found)?;
    for session in result {
        state.playback.stop(&session).await;
    }
    state.emit(None, "users.changed", json!({})).await?;
    Ok(Json(json!({"deleted":true})))
}
async fn notifications(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items = storage::notifications(&state.db, p).await?;
    Ok(Json(json!({"items":items})))
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    storage::read(&state.db, key, user).await?;
    state
        .emit(Some(p.user.id), "notifications.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        let _ = crate::product::observe(&state).await;
        crate::managers::operational_health(&state).await?;
        observe(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
pub(crate) async fn observe(state: &AppState) -> anyhow::Result<()> {
    let changed = storage::observe(&state.db).await?;
    for user in changed {
        state
            .emit(Some(user), "notifications.changed", json!({}))
            .await?;
    }
    Ok(())
}
fn usage(path: &std::path::Path) -> anyhow::Result<Value> {
    let mut pending = vec![path.to_path_buf()];
    let mut bytes = 0u64;
    let mut entries = 0;
    let mut limited = false;
    while let Some(dir) = pending.pop() {
        for item in std::fs::read_dir(dir)? {
            entries += 1;
            if entries > 100000 {
                limited = true;
                break;
            }
            let item = item?;
            let meta = std::fs::symlink_metadata(item.path())?;
            if meta.is_file() {
                bytes += meta.len();
            } else if meta.is_dir() {
                pending.push(item.path());
            }
        }
        if limited {
            break;
        }
    }
    Ok(json!({"bytes":bytes,"partial":limited,"free_bytes":fs2::available_space(path)?}))
}
async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut result = storage::dashboard(&state.db).await?;
    let config = state.config.clone();
    let storage = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        Ok(json!({"state":usage(&config.state)?,"cache":usage(&config.cache)?}))
    })
    .await
    .map_err(anyhow::Error::from)??;
    result["storage"] = storage;
    result["database"] = json!(state.db.metrics());
    Ok(Json(result))
}
async fn diagnostics(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let mut summary = storage::diagnostics(&state.db, p).await?;
    // Deliberate allowlist: no settings, identifiers, paths, error strings, user history or Docker specs.
    summary["database"] = json!(state.db.metrics());
    Ok(Json(summary))
}
async fn devices(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let items = storage::devices(&state.db).await?;
    Ok(Json(json!({"items":items})))
}
