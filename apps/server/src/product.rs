//! First-party release policy; privileged state transitions remain controller-owned.
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
use chrono::Timelike;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, now};
use thelxinoe_releases::{Envelope, Manifest};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/product-update", get(status))
        .route("/api/v1/admin/product-update/policy", post(policy))
        .route("/api/v1/admin/product-update/check", post(check))
        .route("/api/v1/admin/product-update/prepare", post(prepare))
        .route("/api/v1/admin/product-update/{id}/{action}", post(action))
        .route("/api/v1/release", get(release))
}
#[derive(Clone, Serialize, Deserialize)]
struct Policy {
    policy: String,
    window_start: u8,
    window_end: u8,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            policy: "notify".into(),
            window_start: 3,
            window_end: 5,
        }
    }
}
async fn setting(state: &AppState, key: &str) -> Result<Option<Value>> {
    let key = key.to_owned();
    Ok(state
        .db
        .call(move |db| {
            Ok(db
                .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .map(|s| serde_json::from_str(&s))
                .transpose()?)
        })
        .await?)
}
async fn save(state: &AppState, key: &str, value: Value) -> Result<()> {
    let key = key.to_owned();
    state.db.call(move|db|{db.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()])?;Ok(())}).await?;
    Ok(())
}
async fn configured_policy(state: &AppState) -> Result<Policy> {
    Ok(setting(state, "product.policy")
        .await?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}
fn configured() -> bool {
    (std::env::var("THELXINOE_RELEASE_URL").is_ok_and(|v| !v.is_empty())
        || std::env::var("THELXINOE_RELEASE_MANIFEST_FILE")
            .is_ok_and(|v| std::path::Path::new(&v).is_file()))
        && std::env::var("THELXINOE_RELEASE_KEY_FILE")
            .is_ok_and(|v| std::path::Path::new(&v).is_file())
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let controller = crate::managers::controller_request(&state, "/product", None)
        .await
        .unwrap_or_else(|_| json!({"items":[],"error":"Controller unavailable"}));
    Ok(Json(
        json!({"version":thelxinoe_core::VERSION,"policy":configured_policy(&state).await?,"release":setting(&state,"product.release").await?,"observation":setting(&state,"product.observation").await?,"configured":configured(),"controller":controller}),
    ))
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
    if !["automatic", "notify", "manual"].contains(&input.policy.as_str())
        || input.window_start > 23
        || input.window_end > 23
    {
        return Err(ApiError::bad(
            "Invalid update policy or UTC maintenance window",
        ));
    }
    save(&state, "product.policy", json!(input)).await?;
    audit(&state, Some(p.user.id), "policy", "server").await?;
    Ok(Json(json!({"saved":true})))
}
async fn audit(state: &AppState, user: Option<String>, action: &str, target: &str) -> Result<()> {
    let action = format!("product.update.{action}");
    let target = target.to_owned();
    state
        .db
        .call(move |db| {
            db.execute(
                "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
                params![user, action, target, now()],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}
async fn fetch() -> Result<(Envelope, Manifest)> {
    let key = std::env::var("THELXINOE_RELEASE_KEY_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .ok_or_else(|| ApiError::conflict("Release signing public key is not configured"))?;
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
        fetch_bytes().await?
    };
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid release envelope"))?;
    let manifest = thelxinoe_releases::verify(&envelope, &key)
        .map_err(|_| ApiError::bad("Release signature or manifest is invalid"))?;
    Ok((envelope, manifest))
}
async fn fetch_bytes() -> Result<Vec<u8>> {
    let source = std::env::var("THELXINOE_RELEASE_URL")
        .map_err(|_| ApiError::conflict("Release channel is not configured"))?;
    let url = thelxinoe_releases::https(&source)
        .map_err(|_| ApiError::bad("Release channel must use HTTPS"))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| ApiError::conflict("Release channel is unavailable"))?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| ApiError::conflict("Release channel is unavailable"))?
        .error_for_status()
        .map_err(|_| ApiError::conflict("Release channel is unavailable"))?;
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::conflict("Release response was interrupted"))?
    {
        if bytes.len() + chunk.len() > thelxinoe_releases::MAX_ENVELOPE {
            return Err(ApiError::bad("Release manifest exceeds the size limit"));
        }
        bytes.extend(chunk);
    }
    Ok(bytes)
}
async fn check_release(state: &AppState) -> Result<()> {
    let result = async {
        let (envelope, manifest) = fetch().await?;
        if manifest.version == thelxinoe_core::VERSION {
            save(state, "product.release", Value::Null).await?;
            return Ok(());
        }
        manifest
            .candidate(
                thelxinoe_core::VERSION,
                thelxinoe_database::SCHEMA_VERSION,
                now(),
            )
            .map_err(|_| {
                ApiError::conflict("Release version, validity or recovery metadata is incompatible")
            })?;
        save(state, "product.envelope", json!(envelope)).await?;
        save(state, "product.release", json!(manifest)).await?;
        Ok::<_, ApiError>(())
    }
    .await;
    save(
        state,
        "product.observation",
        json!({"checked_at":now(),"error":result.as_ref().err().map(|e|&e.2)}),
    )
    .await?;
    state.emit(None, "product.changed", json!({})).await?;
    result
}
async fn check(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    check_release(&state).await?;
    Ok(Json(json!({"checked":true})))
}
async fn idle(state: &AppState) -> Result<()> {
    let busy=state.db.call(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running') OR EXISTS(SELECT 1 FROM media_operations WHERE state='executing')",[now()],|r|r.get::<_,bool>(0))?)).await?;
    if busy {
        return Err(ApiError::conflict(
            "Wait for playback, downloads and background work to finish",
        ));
    }
    Ok(())
}
async fn prepare_update(state: &AppState, user: Option<String>) -> Result<Value> {
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
    let _gate = state
        .release_gate
        .try_write()
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
    state
        .release_quiescing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    match crate::managers::controller_request(state, path, body).await {
        Err(e) => {
            state
                .release_quiescing
                .store(false, std::sync::atomic::Ordering::SeqCst);
            Err(e)
        }
        Ok(value) => {
            let operation = value["id"].clone();
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Ok(observed) =
                        crate::managers::controller_request(&state, "/product", None).await
                        && observed["items"].as_array().into_iter().flatten().any(|u| {
                            u["id"] == operation
                                && matches!(
                                    u["stage"].as_str(),
                                    Some(
                                        "blocked"
                                            | "ready"
                                            | "recovered"
                                            | "recovery-required"
                                            | "runtime-failure"
                                            | "committed"
                                            | "restored"
                                    )
                                )
                        })
                    {
                        state
                            .release_quiescing
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        break;
                    }
                }
            });
            Ok(value)
        }
    }
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
    // Public publisher metadata lets an incompatible desktop update before login.
    // The desktop independently verifies its embedded publisher trust key.
    Ok(Json(
        json!({"envelope":setting(&state,"product.envelope").await?}),
    ))
}
fn window(start: u8, end: u8, hour: u8) -> bool {
    if start == end {
        true
    } else if start < end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        if configured()
            && let Err(e) = tick(&state).await
        {
            tracing::warn!(code = e.1, "Product update check did not complete");
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
async fn tick(state: &AppState) -> Result<()> {
    let p = configured_policy(state).await?;
    if p.policy == "manual" {
        return Ok(());
    }
    let checked = setting(state, "product.observation")
        .await?
        .and_then(|v| v["checked_at"].as_i64())
        .unwrap_or(0);
    if checked < now() - 6 * 3600 {
        check_release(state).await?;
    }
    if p.policy != "automatic"
        || !window(
            p.window_start,
            p.window_end,
            chrono::Utc::now().hour() as u8,
        )
    {
        return Ok(());
    }
    let Some(release) = setting(state, "product.release")
        .await?
        .filter(|v| !v.is_null())
    else {
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
        .find(|u| u["version"] == release["version"]);
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
