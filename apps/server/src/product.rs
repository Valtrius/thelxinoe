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
/// Controller stages that keep the server fenced. Validation works on copies while
/// the restarted server runs, and activation takes its own fresh rollback copy.
const FENCED: [&str; 9] = [
    "preparing",
    "snapshotting",
    "preparing-activation",
    "isolated-migration",
    "creating-successor",
    "handoff",
    "activating",
    "recovering",
    "restoring-release",
];
const PREPARING: &str = "A release operation is already preparing";
fn controller_requires_maintenance(observed: &Value) -> Result<bool> {
    let items = observed["items"]
        .as_array()
        .ok_or_else(|| ApiError::conflict("Controller returned an invalid update journal"))?;
    let busy = observed["busy"]
        .as_bool()
        .ok_or_else(|| ApiError::conflict("Controller update ownership is unavailable"))?;
    // The controller holds its gate through validation; otherwise a busy gate may
    // still be a command whose journal is not written yet.
    let validating = items.iter().any(|u| u["stage"] == "validating");
    Ok((busy && !validating)
        || items
            .iter()
            .any(|u| u["stage"].as_str().is_some_and(|s| FENCED.contains(&s))))
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
/// Whether a journal prepared containers that no longer run, e.g. after
/// `docker compose up` installed a release. Older controllers leave these in
/// place; activations and recoveries never become obsolete this way.
fn obsolete(operation: &Value, observed: &Value) -> bool {
    let moved = observed["generation"]
        .as_u64()
        .is_some_and(|generation| operation["source_generation"].as_u64() != Some(generation));
    match operation["stage"].as_str() {
        Some("superseded") => true,
        Some("blocked" | "ready") => moved,
        // Only a recovered activation took a rollback copy.
        Some("recovered") => moved && operation["snapshot_ready"] != true,
        _ => false,
    }
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/product-update", get(status))
        .route("/api/v1/admin/product-update/policy", post(policy))
        .route("/api/v1/admin/product-update/check", post(check))
        .route("/api/v1/admin/product-update/install", post(install))
        .route("/api/v1/admin/product-update/prepare", post(prepare))
        .route("/api/v1/admin/product-update/cancel", post(cancel))
        .route("/api/v1/admin/product-update/dismiss", post(dismiss))
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
    // The operations loop records the controller's state every few seconds, so
    // reading the status never waits on the controller.
    let mut controller = setting(&state, "product.controller").await?;
    if controller.is_none() {
        // Before that loop's first record, ask once instead of reporting it missing.
        let _ = observe(&state).await;
        controller = setting(&state, "product.controller").await?;
    }
    let mut controller = controller
        .filter(|value| value["available"] == true)
        .unwrap_or_else(|| json!({"items":[],"error":"Controller unavailable"}));
    // Journals carry the controller's own words; explain its generic outage text.
    for u in controller["items"].as_array_mut().into_iter().flatten() {
        if let Some(error) = u["error"].as_str().map(crate::managers::controller_message) {
            u["error"] = json!(error);
        }
    }
    let mut request = setting(&state, "product.install").await?;
    if let Some(request) = request.as_mut().and_then(Value::as_object_mut) {
        request.remove("envelope");
    }
    // Progress is current only while the operations loop keeps asking.
    let download = setting(&state, "product.download").await?.filter(|d| {
        d["state"] == "downloading" && d["at"].as_i64().is_some_and(|at| at > now() - 30)
    });
    Ok(Json(
        json!({"version":thelxinoe_core::VERSION,"timezone":storage::server_zone(&state.db).await?.name(),"policy":configured_policy(&state).await?,"release":eligible(&state).await?,"request":request,"observation":setting(&state,"product.observation").await?,"automatic":setting(&state,"product.automatic").await?,"dismissed":setting(&state,"product.dismissed").await?,"download":download,"configured":configured(),"controller":controller}),
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
    match controller_status(state).await {
        Ok(mut value) => {
            value["available"] = json!(true);
            save(state, "product.controller", value).await
        }
        Err(error) => {
            if let Some(mut value) = setting(state, "product.controller").await? {
                value["available"] = json!(false);
                save(state, "product.controller", value).await?;
            }
            Err(error)
        }
    }
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
            .map_err(|e| channel_failure(&e));
    };
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid release envelope"))?;
    let manifest = thelxinoe_releases::verify(&envelope, &key)
        .map_err(|_| ApiError::bad("Release signature or manifest is invalid"))?;
    Ok((envelope, manifest))
}
/// Why the release channel gave no release, without its response contents.
fn channel_failure(error: &anyhow::Error) -> ApiError {
    let message = if let Some(e) = error.downcast_ref::<reqwest::Error>() {
        let cause = std::iter::successors(Some(e as &dyn std::error::Error), |e| e.source())
            .map(|e| e.to_string().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(": ");
        let unresolved = [
            "dns error",
            "failed to lookup address",
            "name or service not known",
            "no such host",
            "name resolution",
        ];
        if let Some(status) = e.status() {
            format!("The release channel answered HTTP {}", status.as_u16())
        } else if e.is_timeout() {
            "The release channel did not answer within 60 seconds".into()
        } else if unresolved.iter().any(|text| cause.contains(text)) {
            "The release channel's host name could not be resolved; check the server's DNS".into()
        } else if e.is_builder()
            || ["certificate", "tls", "ssl", "handshake"]
                .iter()
                .any(|text| cause.contains(text))
        {
            "The secure connection to the release channel failed; check the server's clock and trusted certificates".into()
        } else if e.is_connect() {
            "The release channel could not be reached; check the server's internet connection"
                .into()
        } else {
            "The connection to the release channel failed before the release arrived".into()
        }
    } else if error.is::<serde_json::Error>() {
        "The release channel returned an invalid release envelope".into()
    } else {
        let text = error.to_string();
        if text.contains("size limit") || text.contains("too large") {
            "The release manifest exceeds the size limit".into()
        } else if text.contains("HTTPS") {
            text
        } else {
            // What remains are signature checks and the manifest's own rules.
            format!(
                "The release signature or manifest is invalid ({})",
                text.chars().take(120).collect::<String>()
            )
        }
    };
    ApiError(
        axum::http::StatusCode::CONFLICT,
        "release_unavailable",
        message,
    )
}
async fn check_release(state: &AppState) -> Result<()> {
    let _guard = state.release_check.lock().await;
    let result = async {
        let (envelope, manifest) = fetch().await?;
        // Only a newer release concerns this server, also once an older manifest expires.
        let newer = manifest
            .newer_than(thelxinoe_core::VERSION)
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        let mut incompatible = None;
        if newer {
            manifest
                .valid_at(now())
                .map_err(|e| ApiError::conflict(e.to_string()))?;
            // A release this server cannot migrate to is news, not a failed check.
            if let Err(e) = manifest.candidate(
                thelxinoe_core::VERSION,
                thelxinoe_database::SCHEMA_VERSION,
                now(),
            ) {
                incompatible = Some(json!({"version":manifest.version,"reason":e.to_string()}));
            }
        }
        let available = newer && incompatible.is_none();
        Ok::<_, ApiError>((envelope, manifest, available, incompatible))
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
    // Pin the signed envelope the administrator chose, including images and schema.
    let request = json!({"id":thelxinoe_core::id(),"version":release.version,"previous_version":thelxinoe_core::VERSION,"release":release,"envelope":setting(&state,"product.envelope").await?,"state":"pending","error":null});
    audit(&state, Some(p.user.id), "install", &input.version).await?;
    save(&state, "product.install", request.clone()).await?;
    state.emit(None, "product.changed", json!({})).await?;
    Ok(Json(request))
}
/// Withdraws a queued update until the server is fenced for the controller.
async fn cancel(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.release_install.lock().await;
    let mut intent = setting(&state, "product.install")
        .await?
        .filter(|v| v["state"] == "pending")
        .ok_or_else(|| ApiError::conflict("No server update is queued"))?;
    // A submitted command keeps the fence until the controller settles it.
    let observed = controller_status(&state).await.ok();
    if state
        .release_quiescing
        .load(std::sync::atomic::Ordering::SeqCst)
        || observed
            .as_ref()
            .and_then(|o| o["items"].as_array())
            .into_iter()
            .flatten()
            .any(|u| {
                u["id"] == intent["id"] && u["stage"].as_str().is_some_and(|s| FENCED.contains(&s))
            })
    {
        return Err(ApiError::conflict(
            "The update is already installing and can no longer be cancelled",
        ));
    }
    intent["state"] = json!("cancelled");
    let version = intent["version"].as_str().unwrap_or_default().to_owned();
    audit(&state, Some(p.user.id), "cancel", &version).await?;
    save(&state, "product.install", intent).await?;
    state.emit(None, "product.changed", json!({})).await?;
    Ok(Json(json!({"cancelled":true})))
}
#[derive(Deserialize)]
struct Dismiss {
    id: String,
}
/// Hides a failed attempt from the version card and attention; the journal stays.
async fn dismiss(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Dismiss>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&input.id).is_err() {
        return Err(ApiError::bad("Choose a failed server update"));
    }
    audit(&state, Some(p.user.id), "dismiss", &input.id).await?;
    save(&state, "product.dismissed", json!(input.id)).await?;
    state.emit(None, "product.changed", json!({})).await?;
    crate::operations::observe(&state).await?;
    Ok(Json(json!({"dismissed":true})))
}

/// Contention clears by itself and never counts as a failed attempt.
fn contended(e: &ApiError) -> bool {
    e.1 == "operation_contended"
        || e.2.starts_with("Wait for")
        || e.2.contains("Another Docker operation is active")
        || e.2 == PREPARING
}
/// Docker, the controller, downloads and the server itself may work on a later try.
fn transient(e: &ApiError) -> bool {
    contended(e)
        || e.0.is_server_error()
        || matches!(e.1, "dependency_unavailable" | "download_failed")
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
    if operation.is_some_and(|u| u["failure"] == "interrupted" || obsolete(u, &observed)) {
        // Preserve the selected release, but qualify it again: against the new
        // containers, or after a restart interrupted its preparation or activation.
        intent["id"] = json!(thelxinoe_core::id());
        save(state, "product.install", intent.clone()).await?;
        operation = None;
    }
    let release = serde_json::from_value::<Manifest>(intent["release"].clone()).ok();
    if let Some(u) = operation {
        match u["stage"].as_str().unwrap_or("") {
            "committed" if u["version"] == thelxinoe_core::VERSION => {
                intent["state"] = json!("completed");
            }
            "blocked" | "recovered" | "restored" | "runtime-failure" | "recovery-required" => {
                intent["state"] = json!("failed");
                intent["error"] =
                    json!(u["error"].as_str().map(crate::managers::controller_message));
            }
            "ready" => (),
            _ => return Ok(true),
        }
    } else if release.as_ref().is_some_and(|r| {
        r.newer_than(thelxinoe_core::VERSION)
            .is_ok_and(|newer| !newer)
    }) {
        // Installed another way, e.g. with `docker compose pull`.
        intent["state"] = json!("completed");
    }
    if intent["state"] != "pending" {
        save(state, "product.install", intent).await?;
        state.emit(None, "product.changed", json!({})).await?;
        return Ok(true);
    }
    if intent["retry_at"].as_i64().is_some_and(|at| at > now()) {
        return Ok(true);
    }
    let result = async {
        // A newer discovery leaves the pinned release; only time can end its eligibility.
        let release = release.ok_or_else(|| ApiError::conflict("The queued release is invalid"))?;
        release
            .candidate(
                thelxinoe_core::VERSION,
                thelxinoe_database::SCHEMA_VERSION,
                now(),
            )
            .map_err(|e| {
                ApiError::conflict(format!(
                    "The queued release can no longer be installed: {e}"
                ))
            })?;
        let envelope = if intent["envelope"].is_null() {
            // Requests queued by earlier versions pinned only the manifest.
            if eligible(state).await?.map(|m| json!(m)).as_ref() != Some(&intent["release"]) {
                return Err(ApiError::conflict(
                    "The queued release changed. Review the available version and try again.",
                ));
            }
            setting(state, "product.envelope")
                .await?
                .unwrap_or_default()
        } else {
            intent["envelope"].clone()
        };
        if operation.is_none() && !download(state, &release.version, &envelope).await? {
            return Ok(());
        }
        // A ready preflight can coexist with new activity after the snapshot restart.
        // Wait again, then quiesced_command checks idle under the write gate.
        if storage::idle(&state.db).await?
            || state
                .release_quiescing
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(());
        }
        let (path, body) = if let Some(u) = operation {
            (
                format!("/product/{}/activate", u["id"].as_str().unwrap_or("")),
                json!({"confirm":true}),
            )
        } else {
            (
                "/product/preflight".into(),
                json!({"envelope":envelope,"request_id":intent["id"]}),
            )
        };
        quiesced_command(state, &path, Some(body)).await.map(|_| ())
    }
    .await;
    // Contention and a lost controller response are retryable with the same ID;
    // an unavailable Docker or controller is explained while the request waits.
    let waiting = match result {
        Err(e) if !contended(&e) && transient(&e) => Some(e.2),
        Err(e) if !contended(&e) => {
            intent["state"] = json!("failed");
            intent["error"] = json!(e.2);
            save(state, "product.install", intent).await?;
            state.emit(None, "product.changed", json!({})).await?;
            return Ok(true);
        }
        _ => None,
    };
    let changed = intent["waiting"].as_str() != waiting.as_deref();
    if changed || waiting.is_some() {
        intent["retry_at"] = json!(waiting.as_ref().map(|_| now() + 60));
        intent["waiting"] = json!(waiting);
        save(state, "product.install", intent).await?;
    }
    if changed {
        state.emit(None, "product.changed", json!({})).await?;
    }
    Ok(true)
}
/// Has the controller fetch both pinned images while the server keeps working.
/// Older controllers lack this; their preflight downloads under maintenance.
async fn download(state: &AppState, version: &str, envelope: &Value) -> Result<bool> {
    let response = match crate::managers::controller_request(
        state,
        "/product/download",
        Some(json!({"envelope":envelope})),
    )
    .await
    {
        Err(e) if e.1 == "not_found" => return Ok(true),
        response => response?,
    };
    save(
        state,
        "product.download",
        json!({"version":version,"state":response["state"],"download":response["download"],"at":now()}),
    )
    .await?;
    match response["state"].as_str() {
        Some("ready") => Ok(true),
        Some("downloading") => Ok(false),
        Some("failed") => Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "download_failed",
            format!(
                "Downloading the update failed: {}",
                crate::managers::controller_message(response["error"].as_str().unwrap_or(""))
            ),
        )),
        _ => Err(ApiError::conflict(
            "Controller returned an invalid download state",
        )),
    }
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
async fn prepare_update(state: &AppState, user: Option<String>, refresh: bool) -> Result<Value> {
    // Automatic preparation reuses the scheduled check instead of fetching again.
    if refresh {
        check_release(state).await?;
    }
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
    {
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
            return Err(ApiError::conflict(PREPARING));
        }
        idle(state).await?;
        // This record travels with snapshots and is read before new work is admitted
        // after a restart. A failed response does not prove controller rejection.
        save(state, "product.maintenance", json!(true)).await?;
        state
            .release_quiescing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // Requests admitted from here on see the fence, so a slow controller holds no gate.
    crate::managers::controller_request(state, path, body).await
}

async fn prepare(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(prepare_update(&state, Some(p.user.id), true).await?))
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
    // Each start checks once, so a restarted server learns of releases promptly.
    let mut startup = true;
    loop {
        if let Err(e) = reconcile_maintenance(&state).await {
            tracing::warn!(code = e.1, error = %e.2, "Release maintenance reconciliation will retry");
        }
        if configured() {
            if let Err(e) = tick(&state, startup).await {
                tracing::warn!(code = e.1, error = %e.2, "Product update check did not complete");
            }
            startup = false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
/// Releases are checked hourly and after each start. An old result is renewed
/// when the window opens, so Automatic installs a release published that day.
fn check_due(observation: &Value, now: i64, window: bool, startup: bool) -> bool {
    startup
        || observation["next_check"].as_i64().unwrap_or(0) <= now
        || (window
            && observation["failures"].as_u64().unwrap_or(0) == 0
            && observation["last_success"].as_i64().unwrap_or(0) < now - 1800)
}
async fn tick(state: &AppState, startup: bool) -> Result<()> {
    // Discovery continues while a queued request waits.
    let pending = advance_install(state).await;
    let p = configured_policy(state).await?;
    let window = p.policy == "automatic"
        && crate::timezones::in_server_window(state, p.window_start.into(), p.window_end.into())
            .await?;
    let observation = setting(state, "product.observation")
        .await?
        .unwrap_or_default();
    if check_due(&observation, now(), window, startup) {
        check_release(state).await?;
    }
    if pending? || !window || crate::backups::holds_updates(state).await? {
        return Ok(());
    }
    let Some(release) = eligible(state).await? else {
        return Ok(());
    };
    let version = release.version;
    let mut automatic = setting(state, "product.automatic")
        .await?
        .and_then(|value| serde_json::from_value::<Automatic>(value).ok())
        .filter(|automatic| automatic.version == version)
        .unwrap_or_else(|| Automatic {
            version: version.clone(),
            ..Default::default()
        });
    let observed = crate::managers::controller_request(state, "/product", None).await?;
    let items = observed["items"].as_array().cloned().unwrap_or_default();
    let before = automatic.clone();
    let due = automatic_due(&mut automatic, &items, now());
    if automatic != before {
        save(state, "product.automatic", json!(automatic)).await?;
    }
    if !due {
        return Ok(());
    }
    // Failed attempts that automatic_due allows to repeat start a new preflight.
    let candidate = items.iter().find(|u| {
        u["version"] == version
            && !matches!(
                u["stage"].as_str(),
                Some("superseded" | "blocked" | "recovered")
            )
            && (u["stage"] != "ready" || current_preflight(u, &observed))
    });
    // Playback and background work postpone the attempt without counting against it.
    let attempt = match candidate {
        None => {
            async {
                let envelope = setting(state, "product.envelope")
                    .await?
                    .ok_or_else(|| ApiError::conflict("Check for a signed release first"))?;
                if !download(state, &version, &envelope).await? || storage::idle(&state.db).await? {
                    return Ok(());
                }
                prepare_update(state, None, false).await.map(|_| ())
            }
            .await
        }
        Some(u) if u["stage"] == "ready" && u["recovery_tested"] == true => {
            if storage::idle(&state.db).await? {
                return Ok(());
            }
            let id = u["id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Invalid controller update"))?;
            audit(state, None, "activate", id).await?;
            quiesced_command(
                state,
                &format!("/product/{id}/activate"),
                Some(json!({"confirm":true})),
            )
            .await
            .map(|_| ())
        }
        _ => return Ok(()),
    };
    if let Err(e) = &attempt
        && !contended(e)
    {
        // A refused or failed command waits before the next try instead of
        // repeating every few seconds through the whole window.
        automatic.failures += 1;
        automatic.retry_at = now() + automatic_backoff(automatic.failures);
        automatic.error = Some(e.2.clone());
        automatic.held = automatic.failures >= AUTOMATIC_ATTEMPTS;
        save(state, "product.automatic", json!(automatic)).await?;
    }
    attempt
}
/// Automatic attempts at one release. Failures back off instead of repeating
/// every tick; a new release starts over.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Automatic {
    version: String,
    failures: u32,
    retry_at: i64,
    /// The newest blocked controller attempt already counted.
    counted: Option<String>,
    error: Option<String>,
    /// Automatic mode stopped trying this release; an administrator may install it.
    #[serde(default)]
    held: bool,
}
/// Failed attempts at one release before only an administrator retries it.
const AUTOMATIC_ATTEMPTS: u32 = 5;
fn automatic_backoff(failures: u32) -> i64 {
    (15 * 60 * 2i64.pow(failures.saturating_sub(1).min(5))).min(6 * 3600)
}
/// Whether a failed attempt may be repeated: its environment (Docker, disk,
/// download, drift) or a restart stopped it, not the release itself. Older
/// controllers classify nothing; their blocked preparations count as environment.
fn retryable(operation: &Value) -> bool {
    match operation["stage"].as_str() {
        Some("blocked") => !matches!(
            operation["failure"].as_str(),
            Some("validation" | "incompatible")
        ),
        Some("recovered") => matches!(
            operation["failure"].as_str(),
            Some("interrupted" | "environment")
        ),
        _ => false,
    }
}
/// Whether automatic mode may try `automatic.version` now. A retryable failure
/// waits for a growing pause, a few times; an interruption by a restart is
/// retried soon without counting. A release that failed validation, compatibility
/// or activation stays failed, also across database rollback; an administrator
/// may still install it by hand.
fn automatic_due(automatic: &mut Automatic, items: &[Value], now: i64) -> bool {
    let attempts = || items.iter().filter(|u| u["version"] == automatic.version);
    if let Some(failed) = attempts().find(|u| {
        matches!(
            u["stage"].as_str(),
            Some("blocked" | "recovered" | "restored" | "runtime-failure" | "recovery-required")
        ) && !retryable(u)
    }) {
        automatic.held = true;
        automatic.error = failed["error"]
            .as_str()
            .map(crate::managers::controller_message);
        return false;
    }
    // The controller lists attempts newest first.
    if let Some(failed) = attempts().find(|u| retryable(u))
        && failed["id"].as_str() != automatic.counted.as_deref()
    {
        automatic.counted = failed["id"].as_str().map(str::to_owned);
        automatic.error = failed["error"]
            .as_str()
            .map(crate::managers::controller_message);
        if failed["failure"] == "interrupted" {
            automatic.retry_at = now + 60;
        } else {
            automatic.failures += 1;
            automatic.retry_at = now + automatic_backoff(automatic.failures);
        }
    }
    automatic.held = automatic.failures >= AUTOMATIC_ATTEMPTS;
    !automatic.held && automatic.retry_at <= now
}
