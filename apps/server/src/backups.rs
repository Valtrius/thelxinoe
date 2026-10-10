//! Encrypted backups and their daily automatic run. The controller journal remains authoritative.

#[path = "storage/backups.rs"]
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

#[cfg(test)]
#[path = "backups_tests.rs"]
mod tests;

const PASSPHRASE: &str = "backups.passphrase";
/// A busy server releases that window's updates after this long without a backup.
const IDLE_GRACE: i64 = 30 * 60;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/backups", get(list).post(backup))
        .route("/api/v1/admin/backups/policy", post(policy))
        .route("/api/v1/admin/backups/passphrase", post(passphrase))
        .route("/api/v1/admin/backups/{id}/restore", post(restore))
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Policy {
    pub(crate) policy: String,
    pub(crate) retain: u16,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            policy: "manual".into(),
            retain: 7,
        }
    }
}
/// One maintenance window's attempt. The ID is chosen before the first request,
/// so a response lost to the backup's own server restart cannot start another.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Run {
    window: String,
    id: String,
    state: String,
    since: i64,
    updated_at: i64,
    error: Option<String>,
}
async fn configured(state: &AppState) -> Result<Policy> {
    Ok(storage::policy(&state.db).await?.unwrap_or_default())
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut value = crate::managers::controller_request(&state, "/backups", None).await?;
    let window = crate::product::configured_policy(&state).await?;
    value["policy"] = json!(configured(&state).await?);
    value["passphrase_saved"] = json!(storage::passphrase_saved(&state.db).await?);
    value["automatic"] = json!(storage::run(&state.db).await?);
    value["window"] = json!({"start":window.window_start,"end":window.window_end});
    Ok(Json(value))
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Policy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !["automatic", "manual"].contains(&input.policy.as_str())
        || !(1..=365).contains(&input.retain)
    {
        return Err(ApiError::bad(
            "Choose Automatic or Manual and keep 1 to 365 automatic backups",
        ));
    }
    storage::save_policy(&state.db, input, p.user.id).await?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Passphrase {
    passphrase: String,
}
async fn passphrase(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Passphrase>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    // Whoever holds this passphrase can read every later automatic archive.
    crate::authentication::require_fresh(&state, &p).await?;
    if !(16..=1024).contains(&input.passphrase.len()) {
        return Err(ApiError::bad("Use a backup passphrase of 16 to 1024 bytes"));
    }
    state
        .secrets
        .put(&state.db, PASSPHRASE.into(), input.passphrase.as_bytes())
        .await?;
    storage::audit(&state.db, Some(p.user.id), "backup.passphrase").await?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
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
    let action = if restore.is_some() {
        "backup.restore"
    } else {
        "backup.create"
    };
    storage::audit(&state.db, Some(p.user.id), action).await?;
    Ok(Json(
        crate::managers::controller_request(
            &state,
            &path,
            Some(json!({"passphrase":input.passphrase})),
        )
        .await?,
    ))
}

/// The local date of the server window that is open now, when automatic backups are enabled.
async fn due_window(state: &AppState) -> Result<Option<String>> {
    if configured(state).await?.policy != "automatic" {
        return Ok(None);
    }
    let window = crate::product::configured_policy(state).await?;
    Ok(crate::timezones::opened_server_window(
        state,
        window.window_start.into(),
        window.window_end.into(),
    )
    .await?
    .map(|date| date.to_string()))
}
/// Automatic updates in an open server window wait until its backup has finished or given up.
pub(crate) async fn holds_updates(state: &AppState) -> Result<bool> {
    let Some(window) = due_window(state).await? else {
        return Ok(false);
    };
    if !storage::passphrase_saved(&state.db).await? {
        return Ok(false);
    }
    Ok(storage::run(&state.db).await?.is_none_or(|run| {
        run.window != window || matches!(run.state.as_str(), "waiting" | "running")
    }))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        if let Err(e) = tick(&state).await {
            tracing::warn!(code = e.1, "Automatic backup will retry");
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
async fn tick(state: &AppState) -> Result<()> {
    let window = due_window(state).await?;
    let mut run = match storage::run(&state.db).await? {
        // Follow an accepted backup to its end, even after its window closes.
        Some(run) if matches!(run.state.as_str(), "waiting" | "running") => run,
        Some(run) if Some(&run.window) == window.as_ref() => return Ok(()),
        _ => match window.clone() {
            Some(window) if storage::passphrase_saved(&state.db).await? => Run {
                window,
                id: thelxinoe_core::id(),
                state: "waiting".into(),
                since: now(),
                updated_at: now(),
                error: None,
            },
            _ => return Ok(()),
        },
    };
    let open = window.as_ref() == Some(&run.window);
    advance(state, &mut run, open).await?;
    run.updated_at = now();
    storage::save_run(&state.db, run).await?;
    Ok(())
}
async fn advance(state: &AppState, run: &mut Run, open: bool) -> Result<()> {
    let observed = crate::managers::controller_request(state, "/backups", None).await;
    if let Some(item) = observed.as_ref().ok().and_then(|value| {
        value["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["id"] == run.id.as_str())
    }) {
        let stage = item["stage"].as_str().unwrap_or("");
        run.state = match stage {
            "queued" | "quiescing" | "snapshotting" | "encrypting" => "running",
            "failed" | "rollback-activating" => "failed",
            _ => "complete",
        }
        .into();
        run.error = (run.state == "failed").then(|| {
            item["error"]
                .as_str()
                .unwrap_or("The automatic backup failed")
                .to_owned()
        });
        return Ok(());
    }
    if run.state == "running" {
        if observed.is_ok() {
            run.state = "failed".into();
            run.error = Some("The controller no longer reports this backup".into());
        }
        return Ok(());
    }
    // Waiting: the controller has not accepted this attempt yet.
    if !open || now() - run.since >= IDLE_GRACE {
        run.state = "missed".into();
        run.error = Some(
            "The server was not idle during the first 30 minutes of the maintenance window".into(),
        );
        return Ok(());
    }
    if observed.is_err()
        || storage::busy(&state.db).await?
        || state
            .release_quiescing
            .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(());
    }
    let passphrase = match state.secrets.get(&state.db, PASSPHRASE).await {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        _ => None,
    };
    let Some(passphrase) = passphrase else {
        run.state = "failed".into();
        run.error = Some("The saved backup passphrase is unreadable; save it again".into());
        return Ok(());
    };
    let retain = configured(state).await?.retain;
    // Persist the identity first: the controller stops this server shortly after accepting.
    storage::save_run(&state.db, run.clone()).await?;
    match crate::managers::controller_request(
        state,
        "/backups",
        Some(json!({"passphrase":passphrase,"automatic":{"id":run.id,"retain":retain}})),
    )
    .await
    {
        Ok(_) => {
            run.state = "running".into();
            storage::audit(&state.db, None, "backup.create").await?;
        }
        // Contention and an unavailable controller retry with the same ID.
        Err(e)
            if e.1 == "dependency_unavailable"
                || e.2.contains("Another Docker operation is active") => {}
        Err(e) => {
            run.state = "failed".into();
            run.error = Some(e.2);
        }
    }
    Ok(())
}
