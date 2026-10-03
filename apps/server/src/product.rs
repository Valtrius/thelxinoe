//! First-party release policy; privileged state transitions remain controller-owned.

#[path = "storage/product.rs"]
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
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, now};
use thelxinoe_releases::{Envelope, Manifest};

#[cfg(test)]
mod tests;

pub(crate) async fn maintenance_pending(db: &thelxinoe_database::Database) -> anyhow::Result<bool> {
    Ok(storage::setting("product.maintenance".into(), db).await? == Some(json!(true)))
}
fn controller_requires_maintenance(observed: &Value) -> Result<bool> {
    let items = observed["items"]
        .as_array()
        .ok_or_else(|| ApiError::conflict("Controller returned an invalid update journal"))?;
    let busy = observed["busy"]
        .as_bool()
        .ok_or_else(|| ApiError::conflict("Controller update ownership is unavailable"))?;
    Ok(busy
        || items.iter().any(|u| {
            matches!(
                u["stage"].as_str(),
                Some(
                    "preparing"
                        | "snapshotting"
                        | "validating"
                        | "preparing-activation"
                        | "isolated-migration"
                        | "creating-successor"
                        | "handoff"
                        | "activating"
                        | "recovering"
                        | "restoring-release"
                )
            )
        }))
}
async fn reconcile_maintenance(state: &AppState) -> Result<()> {
    // Only a submitted release command fences the server. Service operations
    // alone must not put an otherwise available server into maintenance.
    if !state
        .release_quiescing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(());
    }
    let _gate = state.release_gate.write().await;
    let observed = controller_status(state).await?;
    if !controller_requires_maintenance(&observed)? {
        save(state, "product.maintenance", json!(false)).await?;
        state
            .release_quiescing
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}
fn current_preflight(operation: &Value, observed: &Value) -> bool {
    operation["source_generation"]
        .as_u64()
        .is_some_and(|generation| observed["generation"].as_u64() == Some(generation))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/product-update", get(status))
        .route("/api/v1/admin/product-update/policy", post(policy))
        .route("/api/v1/admin/product-update/check", post(check))
        .route("/api/v1/admin/product-update/install", post(install))
        .route("/api/v1/admin/product-update/prepare", post(prepare))
        .route("/api/v1/admin/product-update/{id}/{action}", post(action))
        .route("/api/v1/release", get(release))
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Policy {
    pub(crate) policy: String,
    pub(crate) window_start: u8,
    pub(crate) window_end: u8,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            policy: "automatic".into(),
            window_start: 3,
            window_end: 5,
        }
    }
}
async fn setting(state: &AppState, key: &str) -> Result<Option<Value>> {
    let key = key.to_owned();
    Ok(storage::setting(key, &state.db).await?)
}
async fn save(state: &AppState, key: &str, value: Value) -> Result<()> {
    let key = key.to_owned();
    storage::save(key, &state.db, value).await?;
    Ok(())
}
pub(crate) async fn configured_policy(state: &AppState) -> Result<Policy> {
    Ok(setting(state, "product.policy")
        .await?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}
fn channel() -> String {
    std::env::var("THELXINOE_RELEASE_URL")
        .unwrap_or_else(|_| thelxinoe_releases::DEFAULT_CHANNEL.into())
}
fn key_file() -> String {
    std::env::var("THELXINOE_RELEASE_KEY_FILE")
        .unwrap_or_else(|_| "/etc/thelxinoe/release.pub".into())
}
fn configured() -> bool {
    (!channel().is_empty() || std::env::var("THELXINOE_RELEASE_MANIFEST_FILE").is_ok())
        && std::path::Path::new(&key_file()).is_file()
}
async fn eligible(state: &AppState) -> Result<Option<Manifest>> {
    Ok(setting(state, "product.release")
        .await?
        .and_then(|value| serde_json::from_value::<Manifest>(value).ok())
        .filter(|manifest| {
            manifest
                .candidate(
                    thelxinoe_core::VERSION,
                    thelxinoe_database::SCHEMA_VERSION,
                    now(),
                )
                .is_ok()
        }))
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let controller = controller_status(&state)
        .await
        .unwrap_or_else(|_| json!({"items":[],"error":"Controller unavailable"}));
    Ok(Json(
        json!({"version":thelxinoe_core::VERSION,"timezone":storage::server_zone(&state.db).await?.name(),"policy":configured_policy(&state).await?,"release":eligible(&state).await?,"request":setting(&state,"product.install").await?,"observation":setting(&state,"product.observation").await?,"configured":configured(),"controller":controller}),
    ))
}
async fn controller_status(state: &AppState) -> Result<Value> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::managers::controller_request(state, "/product", None),
    )
    .await
    .map_err(|_| {
        ApiError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "controller_unavailable",
            "Controller did not respond".into(),
        )
    })?
}
pub(crate) async fn observe(state: &AppState) -> Result<()> {
    let value = crate::managers::controller_request(state, "/product", None).await?;
    save(state, "product.controller", value).await
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Policy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !["automatic", "notify"].contains(&input.policy.as_str())
        || input.window_start > 23
        || input.window_end > 23
    {
        return Err(ApiError::bad("Invalid update policy or maintenance window"));
    }
    save(&state, "product.policy", json!(input)).await?;
    audit(&state, Some(p.user.id), "policy", "server").await?;
    Ok(Json(json!({"saved":true})))
}
async fn audit(state: &AppState, user: Option<String>, action: &str, target: &str) -> Result<()> {
    let action = format!("product.update.{action}");
    let target = target.to_owned();
    storage::audit(action, target, &state.db, user).await?;
    Ok(())
}
async fn fetch() -> Result<(Envelope, Manifest)> {
    let key = std::fs::read_to_string(key_file())
        .map_err(|_| ApiError::conflict("Release signing public key is not configured"))?;
    let bytes = if let Ok(path) = std::env::var("THELXINOE_RELEASE_MANIFEST_FILE") {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|_| ApiError::conflict("Signed release file is unavailable"))?;
        let mut bytes = vec![];
        file.take(thelxinoe_releases::MAX_ENVELOPE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ApiError::conflict("Signed release file cannot be read"))?;
        if bytes.len() > thelxinoe_releases::MAX_ENVELOPE {
            return Err(ApiError::bad("Release manifest exceeds the size limit"));
        }
        bytes
    } else {
        let ca = std::env::var("THELXINOE_RELEASE_CA_FILE")
            .ok()
            .map(std::fs::read_to_string)
            .transpose()
            .map_err(|_| ApiError::conflict("Publisher TLS certificate is unavailable"))?;
        return thelxinoe_releases::fetch(&channel(), &key, ca.as_deref())
            .await
            .map_err(|_| ApiError::conflict("Signed release channel is unavailable or invalid"));
    };
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid release envelope"))?;
    let manifest = thelxinoe_releases::verify(&envelope, &key)
        .map_err(|_| ApiError::bad("Release signature or manifest is invalid"))?;
    Ok((envelope, manifest))
}
async fn check_release(state: &AppState) -> Result<()> {
    let _guard = state.release_check.lock().await;
    let result = async {
        let (envelope, manifest) = fetch().await?;
        manifest
            .valid_at(now())
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        let candidate = manifest
            .newer_than(thelxinoe_core::VERSION)
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        if candidate {
            manifest
                .candidate(
                    thelxinoe_core::VERSION,
                    thelxinoe_database::SCHEMA_VERSION,
                    now(),
                )
                .map_err(|e| ApiError::conflict(e.to_string()))?;
        }
        Ok::<_, ApiError>((envelope, manifest, candidate))
    }
    .await;
    storage::discovered(
        &state.db,
        result.as_ref().ok().cloned(),
        result.as_ref().err().map(|e| e.2.clone()),
    )
    .await?;
    state.emit(None, "product.changed", json!({})).await?;
    crate::operations::observe(state).await?;
    result.map(|_| ())
}
async fn check(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    check_release(&state).await?;
    Ok(Json(json!({"checked":true})))
}

#[derive(Deserialize)]
struct Install {
    version: String,
}
async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Install>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.release_install.lock().await;
    if let Some(request) = setting(&state, "product.install").await?
        && request["state"] == "pending"
    {
        let observed = controller_status(&state).await?;
        let finished = observed["items"].as_array().into_iter().flatten().any(|u| {
            u["id"] == request["id"]
                && matches!(
                    u["stage"].as_str(),
                    Some("blocked" | "recovered" | "restored" | "committed")
                )
        });
        if !finished {
            return Ok(Json(request));
        }
    }
    check_release(&state).await?;
    let release = eligible(&state)
        .await?
        .ok_or_else(|| ApiError::conflict("No eligible server release is available"))?;
    if release.version != input.version {
        return Err(ApiError::conflict(
            "The available release changed. Check the version and try again.",
        ));
    }
    let request = json!({"id":thelxinoe_core::id(),"version":release.version,"previous_version":thelxinoe_core::VERSION,"release":release,"state":"pending","error":null});
    audit(&state, Some(p.user.id), "install", &input.version).await?;
    save(&state, "product.install", request.clone()).await?;
    state.emit(None, "product.changed", json!({})).await?;
    Ok(Json(request))
}

// The intent lives in server state before the first snapshot. The controller's
// durable operation ID makes resuming it safe after response loss or rollback.
async fn advance_install(state: &AppState) -> Result<bool> {
    let _guard = state.release_install.lock().await;
    let Some(mut intent) = setting(state, "product.install")
        .await?
        .filter(|v| v["state"] == "pending")
    else {
        return Ok(false);
    };
    let observed = controller_status(state).await?;
    let mut operation = observed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| u["id"] == intent["id"]);
    if operation.is_some_and(|u| {
        u["stage"] == "superseded" || (u["stage"] == "ready" && !current_preflight(u, &observed))
    }) {
        // Preserve the selected release, but qualify it against the new containers.
        intent["id"] = json!(thelxinoe_core::id());
        save(state, "product.install", intent.clone()).await?;
        operation = None;
    }
    if let Some(u) = operation {
        match u["stage"].as_str().unwrap_or("") {
            "committed" if u["version"] == thelxinoe_core::VERSION => {
                intent["state"] = json!("completed");
            }
            "blocked" | "recovered" | "restored" | "runtime-failure" | "recovery-required" => {
                intent["state"] = json!("failed");
                intent["error"] = u["error"].clone();
            }
            "ready" => (),
            _ => return Ok(true),
        }
        if intent["state"] != "pending" {
            save(state, "product.install", intent).await?;
            state.emit(None, "product.changed", json!({})).await?;
            return Ok(true);
        }
    }
    // A ready preflight can coexist with new activity after the snapshot restart.
    // Wait again, then quiesced_command checks idle under the write gate.
    if storage::idle(&state.db).await?
        || state
            .release_quiescing
            .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(true);
    }
    let result = async {
        check_release(state).await?;
        let release = eligible(state).await?.ok_or_else(|| ApiError::conflict("The queued release is no longer available"))?;
        // Pin the signed identity the user selected, including images and schema.
        if json!(release) != intent["release"] {
            return Err(ApiError::conflict("The queued release changed. Review the available version and try again."));
        }
        let (path, body) = if let Some(u) = operation {
            (format!("/product/{}/activate", u["id"].as_str().unwrap_or("")), json!({"confirm":true}))
        } else {
            ("/product/preflight".into(), json!({"envelope":setting(state,"product.envelope").await?,"request_id":intent["id"]}))
        };
        quiesced_command(state, &path, Some(body)).await?;
        Ok::<_, ApiError>(())
    }.await;
    if let Err(e) = result {
        // Contention and a lost controller response are retryable with the same ID.
        // Discovery errors are definitive and require another explicit click.
        if e.2.starts_with("Wait for")
            || e.2.contains("Another Docker operation is active")
            || e.0 == axum::http::StatusCode::SERVICE_UNAVAILABLE
        {
            return Ok(true);
        }
        intent["state"] = json!("failed");
        intent["error"] = json!(e.2);
        save(state, "product.install", intent).await?;
        state.emit(None, "product.changed", json!({})).await?;
    }
    Ok(true)
}
async fn idle(state: &AppState) -> Result<()> {
    let busy = storage::idle(&state.db).await?;
    if busy {
        return Err(ApiError::conflict(
            "Wait for playback, downloads and background work to finish",
        ));
    }
    Ok(())
}
async fn prepare_update(state: &AppState, user: Option<String>) -> Result<Value> {
    check_release(state).await?;
    if eligible(state).await?.is_none() {
        return Err(ApiError::conflict(
            "No eligible server release is available",
        ));
    }
    let envelope = setting(state, "product.envelope")
        .await?
        .ok_or_else(|| ApiError::conflict("Check for a signed release first"))?;
    audit(state, user, "preflight", "server").await?;
    quiesced_command(
        state,
        "/product/preflight",
        Some(json!({"envelope":envelope})),
    )
    .await
}
async fn quiesced_command(state: &AppState, path: &str, body: Option<Value>) -> Result<Value> {
    let _gate = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.release_gate.write(),
    )
    .await
    .map_err(|_| ApiError::conflict("Wait for current requests to finish and try again"))?;
    if state
        .release_quiescing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(ApiError::conflict(
            "A release operation is already preparing",
        ));
    }
    idle(state).await?;
    // This record travels with snapshots and is read before new work is admitted
    // after a restart. A failed response does not prove controller rejection.
    save(state, "product.maintenance", json!(true)).await?;
    state
        .release_quiescing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    crate::managers::controller_request(state, path, body).await
}

async fn prepare(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(prepare_update(&state, Some(p.user.id)).await?))
}
#[derive(Deserialize)]
struct Confirm {
    confirm: bool,
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((key, action)): Path<(String, String)>,
    Json(input): Json<Confirm>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&key).is_err()
        || !["activate", "recover"].contains(&action.as_str())
        || !input.confirm
    {
        return Err(ApiError::bad("Confirm the selected release operation"));
    }
    if action == "activate" {
        check_release(&state).await?;
        let release = eligible(&state).await?.ok_or_else(|| {
            ApiError::conflict("Check for an eligible server release before installing")
        })?;
        let observed = crate::managers::controller_request(&state, "/product", None).await?;
        if !observed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|u| u["id"] == key && u["version"] == release.version)
        {
            return Err(ApiError::conflict(
                "The prepared release has been replaced; prepare the current release",
            ));
        }
    }
    idle(&state).await?;
    audit(&state, Some(p.user.id), &action, &key).await?;
    Ok(Json(
        quiesced_command(
            &state,
            &format!("/product/{key}/{action}"),
            Some(json!({"confirm":true})),
        )
        .await?,
    ))
}
async fn release(State(state): State<AppState>) -> Result<Json<Value>> {
    // Expose the last verified publisher envelope for diagnostics and clients.
    Ok(Json(
        json!({"envelope":setting(&state,"product.envelope").await?}),
    ))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        if let Err(e) = reconcile_maintenance(&state).await {
            tracing::warn!(code = e.1, "Release maintenance reconciliation will retry");
        }
        if configured()
            && let Err(e) = tick(&state).await
        {
            tracing::warn!(code = e.1, "Product update check did not complete");
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
async fn tick(state: &AppState) -> Result<()> {
    if advance_install(state).await? {
        return Ok(());
    }
    let p = configured_policy(state).await?;
    let next_check = setting(state, "product.observation")
        .await?
        .and_then(|v| v["next_check"].as_i64())
        .unwrap_or(0);
    if next_check <= now() {
        check_release(state).await?;
    }
    if p.policy != "automatic"
        || !crate::timezones::in_server_window(state, p.window_start.into(), p.window_end.into())
            .await?
    {
        return Ok(());
    }
    let Some(release) = eligible(state).await?.map(|m| json!(m)) else {
        return Ok(());
    };
    let observed = crate::managers::controller_request(state, "/product", None).await?;
    // A restored or failed release stays blocked across database rollback. A
    // human may prepare it again, but automatic mode never retries that version.
    if observed["items"].as_array().into_iter().flatten().any(|u| {
        u["version"] == release["version"]
            && matches!(
                u["stage"].as_str(),
                Some(
                    "blocked" | "recovered" | "restored" | "runtime-failure" | "recovery-required"
                )
            )
    }) {
        return Ok(());
    }
    let candidate = observed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| {
            u["version"] == release["version"]
                && u["stage"] != "superseded"
                && (u["stage"] != "ready" || current_preflight(u, &observed))
        });
    idle(state).await?;
    match candidate {
        None => {
            prepare_update(state, None).await?;
        }
        Some(u) if u["stage"] == "ready" && u["recovery_tested"] == true => {
            let id = u["id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Invalid controller update"))?;
            audit(state, None, "activate", id).await?;
            quiesced_command(
                state,
                &format!("/product/{id}/activate"),
                Some(json!({"confirm":true})),
            )
            .await?;
        }
        _ => (),
    }
    Ok(())
}
