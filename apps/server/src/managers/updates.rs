//! Administrator policy and durable orchestration; Docker journal remains authoritative.

#[path = "../storage/managers/updates.rs"]
mod storage;

use super::*;
use stack::controller;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/service-updates", get(list))
        .route("/api/v1/admin/service-updates/policy/{id}", post(policy))
        .route("/api/v1/admin/service-updates/check", post(discover))
        .route(
            "/api/v1/admin/service-updates/preflight/{id}",
            post(preflight),
        )
        .route("/api/v1/admin/service-updates/{id}/{action}", post(action))
}
#[derive(Deserialize)]
struct Policy {
    policy: String,
    window_start: u8,
    window_end: u8,
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Policy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&key).is_err()
        || !["automatic", "notify", "manual", "inherit"].contains(&input.policy.as_str())
        || input.window_start > 23
        || input.window_end > 23
    {
        return Err(ApiError::bad("Invalid update policy or maintenance window"));
    }
    storage::policy(&state.db, key, input, p).await?;
    Ok(Json(json!({"saved":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let server_policy = crate::product::configured_policy(&state).await?;
    let mut value = storage::list(&state.db).await?;
    value["server_policy"] = json!(server_policy);
    if let Ok(observed) = controller(&state, "/updates", None).await {
        for item in value["items"].as_array_mut().ok_or_else(unavailable)? {
            if let Some(current) = observed["items"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|v| v["id"] == item["id"])
            {
                item["classification"] = current["classification"].clone();
            }
        }
    }
    Ok(Json(value))
}
async fn discover(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    check_releases(&state, true).await?;
    Ok(Json(json!({"checked":true})))
}
async fn preflight(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let key = enqueue(&state, &service, Some(p.user.id)).await?;
    Ok(Json(json!({"id":key,"state":"queued"})))
}
async fn enqueue(state: &AppState, service: &str, actor: Option<String>) -> Result<String> {
    if uuid::Uuid::parse_str(service).is_err() {
        return Err(ApiError::bad("Invalid managed service"));
    }
    let service = service.to_owned();
    let key = id();
    let returned = key.clone();
    let inserted = storage::enqueue(service, key, &state.db, actor).await?;
    if !inserted {
        return Err(ApiError::conflict(
            "Service is unavailable or already has an active update",
        ));
    }
    Ok(returned)
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((key, action)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !["activate", "recover"].contains(&action.as_str()) {
        return Err(ApiError::bad("Unsupported update action"));
    }
    queue_action(&state, key, action, Some(p.user.id)).await?;
    Ok(Json(json!({"queued":true})))
}
async fn queue_action(
    state: &AppState,
    key: String,
    action: String,
    actor: Option<String>,
) -> Result<()> {
    let changed = storage::queue_action(&state.db, key, action, actor).await?;
    if !changed {
        return Err(ApiError::conflict("Update is not ready for that action"));
    }
    Ok(())
}
async fn progress(
    state: &AppState,
    key: &str,
    stage: &str,
    candidate: Option<String>,
    error: Option<String>,
) -> Result<()> {
    let key = key.to_owned();
    let stage = stage.to_owned();
    storage::progress(key, stage, &state.db, candidate, error).await?;
    Ok(())
}
async fn idle(state: &AppState, provision: &str) -> Result<()> {
    let provision = provision.to_owned();
    let (kind, service) = storage::idle_read_stack_provisions(provision, &state.db).await?;
    let active = storage::idle_read_playback_sessions(&state.db).await?;
    if active {
        return Err(ApiError::conflict(
            "Updates wait for active playback to finish",
        ));
    }
    if ["radarr", "sonarr", "lidarr"].contains(&kind.as_str()) {
        operations::ensure_idle(state, &service).await
    } else {
        support::ensure_idle(state, &service).await
    }
}
pub(crate) async fn run_job(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<bool> {
    let key = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing update identity"))?;
    let action = job.payload["action"].as_str().unwrap_or("");
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    let lookup = key.to_owned();
    let server_policy = crate::product::configured_policy(state)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.2))?;
    let automatic = storage::run_job_read_service_updates(lookup, &state.db).await?;
    let inherited = automatic
        .2
        .as_deref()
        .is_none_or(|policy| policy == "inherit");
    let mode = if inherited {
        server_policy.policy.clone()
    } else {
        automatic
            .2
            .clone()
            .unwrap_or_else(|| server_policy.policy.clone())
    };
    let start = if inherited {
        server_policy.window_start as u32
    } else {
        automatic.3.unwrap_or(server_policy.window_start as u32)
    };
    let end = if inherited {
        server_policy.window_end as u32
    } else {
        automatic.4.unwrap_or(server_policy.window_end as u32)
    };
    if automatic.0
        && ["queued", "queued-activate"].contains(&automatic.5.as_str())
        && action != "recover"
    {
        let wait = mode != "automatic"
            || !crate::timezones::in_server_window(state, start, end).await?
            || idle(state, &automatic.1).await.is_err();
        if wait {
            let job_id = job.id.clone();
            storage::run_job_write_jobs(job_id, &state.db).await?;
            progress(
                state,
                key,
                if action == "activate" {
                    "queued-activate"
                } else {
                    "queued"
                },
                None,
                Some("Waiting for Automatic policy, maintenance window and an idle service".into()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
            return Ok(false);
        }
    }
    let result = run_locked(state, key, action).await;
    if let Err(error) = result {
        progress(state, key, "blocked", None, Some(error.2.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        state
            .emit(None, "service.update.blocked", json!({"id":key}))
            .await?;
        anyhow::bail!("{}", error.2);
    }
    Ok(true)
}
async fn run_locked(state: &AppState, key: &str, action: &str) -> Result<()> {
    let lookup = key.to_owned();
    let (service, stage, authorized) = storage::run_locked(lookup, &state.db).await?;
    if !authorized {
        return Err(ApiError::forbidden());
    }
    if ["committed", "rolled-back"].contains(&stage.as_str()) {
        return Ok(());
    }
    let observed = controller(state, "/updates", None).await?;
    let existing = observed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| u["id"] == key)
        .cloned();
    let submit = match action {
        "preflight" => existing.is_none(),
        "activate" => existing.as_ref().is_some_and(|v| v["stage"] == "ready"),
        "recover" => true,
        _ => return Err(ApiError::bad("Unsupported update job")),
    };
    if submit {
        if action != "recover" {
            idle(state, &service).await?;
        }
        progress(state, key, "submitting", None, None).await?;
        let path = if action == "preflight" {
            format!("/{service}/preflight")
        } else {
            format!("/updates/{key}/{action}")
        };
        controller(state, &path, Some(json!({"operation_id":key}))).await?;
    }
    for _ in 0..1200 {
        let items = controller(state, "/updates", None).await?;
        let update = items["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| v["id"] == key)
            .ok_or_else(unavailable)?;
        let stage = update["stage"].as_str().ok_or_else(unavailable)?;
        progress(
            state,
            key,
            stage,
            update["candidate"].as_str().map(str::to_owned),
            update["error"].as_str().map(str::to_owned),
        )
        .await?;
        if ["committed", "runtime-failure"].contains(&stage) && update["container_id"].is_string() {
            reconnect(
                state,
                &service,
                update["container_id"].as_str().ok_or_else(unavailable)?,
            )
            .await?;
        }
        if [
            "ready",
            "committed",
            "rolled-back",
            "blocked",
            "recovery-required",
            "runtime-failure",
        ]
        .contains(&stage)
        {
            state
                .emit(
                    None,
                    "service.update.changed",
                    json!({"id":key,"state":stage}),
                )
                .await?;
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(ApiError::conflict(
        "Update remains unfinished; inspect controller recovery state",
    ))
}
async fn reconnect(state: &AppState, provision: &str, container: &str) -> Result<()> {
    let provision = provision.to_owned();
    let container = container.to_owned();
    storage::reconnect(provision, container, &state.db).await?;
    Ok(())
}
async fn check_releases(state: &AppState, force: bool) -> Result<()> {
    let server_policy = crate::product::configured_policy(state).await?;
    let raw = storage::check_releases_read_stack_provisions(&state.db).await?;
    let rows = raw
        .into_iter()
        .map(|(id, kind, policy, start, end, checked)| {
            let inherited = policy.as_deref().is_none_or(|value| value == "inherit");
            let mode = if inherited {
                server_policy.policy.clone()
            } else {
                policy.unwrap_or_else(|| server_policy.policy.clone())
            };
            let start = if inherited {
                server_policy.window_start as u32
            } else {
                start.unwrap_or(server_policy.window_start as u32)
            };
            let end = if inherited {
                server_policy.window_end as u32
            } else {
                end.unwrap_or(server_policy.window_end as u32)
            };
            (id, kind, mode, start, end, checked)
        })
        .collect::<Vec<_>>();
    let pending: Vec<_> = rows
        .into_iter()
        .filter(|r| force || (r.2 != "manual" && now() - r.5 >= 21600))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let releases = controller(state, "/releases", None).await?;
    let stack = controller(state, "", None).await?;
    for (service, kind, policy, start, end, _) in pending {
        let candidate = releases["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["kind"] == kind)
            .and_then(|r| r["image"].as_str())
            .map(str::to_owned);
        let current = stack["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["id"] == service)
            .and_then(|r| r["image"].as_str());
        let changed = candidate.as_deref().is_some_and(|c| Some(c) != current);
        let key = service.clone();
        let found = candidate.clone();
        let mode = "inherit".to_owned();
        storage::check_releases_write_service_update_policy(
            start, end, key, found, mode, &state.db,
        )
        .await?;
        if changed {
            state
                .emit(
                    None,
                    "service.update.available",
                    json!({"service_id":service,"candidate":candidate}),
                )
                .await?;
            if policy == "automatic"
                && crate::timezones::in_server_window(state, start, end).await?
            {
                let _ = enqueue(state, &service, None).await;
            }
        }
    }
    Ok(())
}
pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        // Failures remain visible in policy/update state; they never terminate the server.
        let _ = check_releases(&state, false).await;
        let server_policy = crate::product::configured_policy(&state)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        let raw = storage::run(&state.db).await?;
        for (key, policy, start, end) in raw {
            let inherited = policy.as_deref().is_none_or(|value| value == "inherit");
            let mode = if inherited {
                server_policy.policy.as_str()
            } else {
                policy.as_deref().unwrap_or(server_policy.policy.as_str())
            };
            if mode != "automatic" {
                continue;
            }
            let start = if inherited {
                server_policy.window_start as u32
            } else {
                start.unwrap_or(server_policy.window_start as u32)
            };
            let end = if inherited {
                server_policy.window_end as u32
            } else {
                end.unwrap_or(server_policy.window_end as u32)
            };
            if crate::timezones::in_server_window(&state, start, end).await? {
                let _ = queue_action(&state, key, "activate".into(), None).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
