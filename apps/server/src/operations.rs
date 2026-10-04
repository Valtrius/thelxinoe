//! Administrative observations never include credentials or arbitrary upstream payloads.

#[path = "storage/attention.rs"]
mod attention_storage;
#[path = "storage/operations.rs"]
mod storage;

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AttentionTarget {
    Services,
    Server,
    Jobs,
    Online,
    Requests,
    Movies,
    Shows,
    Music,
}
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct AttentionItem {
    id: String,
    severity: Severity,
    message: String,
    target: AttentionTarget,
    resource: Option<String>,
    revision: String,
    dismissible: bool,
    media_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct RequestAttention {
    pub(crate) id: String,
    pub(crate) revision: String,
}
pub(crate) fn native_request_revision(generation: &str, state: &str, updated: i64) -> String {
    format!("{generation}:{state}:{updated}")
}
pub(crate) fn seerr_request_state(row: &Value) -> &'static str {
    if row["media"]["status"] == 5 || row["media"]["status4k"] == 5 || row["status"] == 5 {
        "available"
    } else {
        match row["status"].as_i64() {
            Some(2) => "requested",
            Some(3) => "denied",
            Some(4) => "failed",
            _ => "pending",
        }
    }
}
pub(crate) fn seerr_request_attention(service: &str, row: &Value) -> Option<RequestAttention> {
    let id = row["id"].as_i64().filter(|id| *id > 0)?;
    Some(RequestAttention {
        id: format!("seerr:{service}:{id}"),
        revision: format!(
            "{}:{}:{}",
            seerr_request_state(row),
            row["updatedAt"]
                .as_str()
                .or_else(|| row["createdAt"].as_str())
                .unwrap_or(""),
            row["seasons"]
        ),
    })
}

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
use thelxinoe_core::{Capability, now};
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/me/attention", get(attention))
        .route("/api/v1/me/attention/{id}", put(acknowledge))
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
    Json(input): Json<UserChange>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    crate::authentication::require_fresh(&state, &p).await?;
    if input.password.is_some() {
        return Err(ApiError::bad(
            "Use an account recovery link to reset sign-in methods",
        ));
    }
    let status = storage::change_user(&state.db, key, input, p, None).await?;
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
    crate::authentication::require_fresh(&state, &p).await?;
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
async fn attention(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items = attention_storage::summary(&state.db, p).await?;
    Ok(Json(json!({"items":items})))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgment {
    revision: String,
}
async fn acknowledge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Acknowledgment>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    if !attention_storage::acknowledge(&state.db, p, key, input.revision).await? {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(user), "attention.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        let _ = crate::product::observe(&state).await;
        crate::managers::operational_health(&state).await?;
        let _ = crate::managers::observe_requests(&state).await;
        observe(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
pub(crate) async fn observe(state: &AppState) -> anyhow::Result<()> {
    for user in attention_storage::observe(&state.db).await? {
        state
            .emit(Some(user), "attention.changed", json!({}))
            .await?;
    }
    Ok(())
}
pub(crate) async fn cache_seerr_requests(
    state: &AppState,
    service: String,
    rows: Vec<Value>,
) -> anyhow::Result<()> {
    attention_storage::cache_seerr(&state.db, service, rows).await
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
