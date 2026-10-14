//! Guided ownership transfer of a Connected service into the Owned stage. The controller
//! journals Docker and file transitions; this record holds the reviewed plan, approval,
//! setup journal, checks and recovery, and gates every normal worker through its hold.
#[path = "transfer_checks.rs"]
mod checks;
#[path = "../storage/managers/transfers.rs"]
mod storage;

use super::*;
use sha2::{Digest, Sha256};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/services", get(services))
        .route("/api/v1/admin/services/{id}/test", post(test))
        .route("/api/v1/admin/services/{id}/forget", post(forget))
        .route("/api/v1/admin/services/{id}/transfer", post(review))
        .route(
            "/api/v1/admin/recyclarr/connection",
            post(connect_recyclarr),
        )
        .route("/api/v1/admin/transfers", get(list))
        .route("/api/v1/admin/transfers/{id}", get(view))
        .route(
            "/api/v1/admin/transfers/{id}/choices",
            axum::routing::put(choices),
        )
        .route("/api/v1/admin/transfers/{id}/approve", post(approve))
        .route("/api/v1/admin/transfers/{id}/cancel", post(cancel))
        .route("/api/v1/admin/transfers/{id}/retry", post(retry))
        .route("/api/v1/admin/transfers/{id}/restore", post(restore))
        .route(
            "/api/v1/admin/transfers/{id}/recovery",
            get(recovery_plan).post(recover),
        )
        .route(
            "/api/v1/admin/transfers/{id}/original/remove",
            post(remove_original),
        )
}

fn digest(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn valid_id(value: &str) -> Result<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ApiError::bad("Invalid identity"))
}

/// The guided workflow, in order. Each step says who acts.
const STEPS: [(&str, &str, &str); 12] = [
    ("prerequisites", "Check prerequisites", "backend"),
    ("review", "Review changes", "user"),
    ("automation", "Confirm automation is retired", "user"),
    ("confirm", "Confirm the transfer", "user"),
    ("stop", "Stop the original", "backend"),
    ("copy", "Copy appdata", "backend"),
    ("validate", "Validate a disposable copy", "backend"),
    ("activate", "Start the managed replacement", "backend"),
    ("commit", "Commit ownership", "backend"),
    ("setup", "Run reviewed setup", "backend"),
    ("checks", "Verify required checks", "backend"),
    ("complete", "Complete the transfer", "backend"),
];
/// Map the controller journal onto the guided steps.
fn controller_step(stage: &str) -> &'static str {
    match stage {
        "approved" | "stopping" => "stop",
        "copying" => "copy",
        "validating" => "validate",
        "building" | "creating" | "activating" => "activate",
        _ => "commit",
    }
}
fn steps(t: &storage::Row) -> Vec<Value> {
    let blocked = t.review["blockers"]
        .as_array()
        .is_some_and(|b| !b.is_empty());
    let current = STEPS.iter().position(|(k, _, _)| *k == t.step).unwrap_or(0);
    let failed = matches!(t.state.as_str(), "failed" | "suspended")
        || (t.state == "returned" && t.error.is_some());
    // Services without reviewed setup actions skip that step.
    let no_setup = t.review["setup_actions"]
        .as_array()
        .is_none_or(Vec::is_empty);
    STEPS
        .iter()
        .enumerate()
        .map(|(index, (key, label, actor))| {
            let label = if *key == "activate" && t.kind == "recyclarr" {
                "Create the managed job definition"
            } else {
                label
            };
            let status = match t.state.as_str() {
                "complete" => "complete",
                "review" => match *key {
                    "prerequisites" if blocked => "blocked",
                    "prerequisites" => "complete",
                    "review" | "automation" | "confirm" => "awaiting_input",
                    _ => "pending",
                },
                "cancelled" if index < 4 => "skipped",
                "cancelled" => "pending",
                _ if index < current => "complete",
                _ if index == current && failed => "failed",
                _ if index == current
                    && t.state == "queued"
                    && t.progress["waiting"].is_string() =>
                {
                    "blocked"
                }
                _ if index == current => "running",
                _ => "pending",
            };
            let status = if *key == "setup" && no_setup && status != "failed" {
                "skipped"
            } else {
                status
            };
            json!({"key":key,"label":label,"actor":actor,"status":status})
        })
        .collect()
}

/// Actions the backend permits for this operation right now.
fn actions(t: &storage::Row, controller: &Value) -> Value {
    let stage = controller["stage"].as_str().unwrap_or("");
    let boundary = controller["boundary_at"].is_number();
    json!({
        "approve": t.state == "review" && t.review["blockers"].as_array().is_none_or(Vec::is_empty),
        "cancel": t.state == "review" || (t.state == "queued" && matches!(stage, "" | "approved") && controller["source"]["restart_disabled"] != true),
        "retry": t.state == "failed" && t.replacement_id.is_some(),
        "restore": t.state == "failed" && !boundary && stage == "failed",
        "recovery": matches!(t.state.as_str(), "failed" | "suspended" | "recovery") && boundary && controller["completed"] != true,
        "remove_original": t.state == "complete" && controller["source"]["removed"] != true,
        "continue_in_background": matches!(t.state.as_str(), "queued" | "running" | "committed" | "recovery"),
    })
}

async fn controller_status(state: &AppState, id: &str) -> Value {
    controller_request(state, &format!("/transfers/{id}"), None)
        .await
        .unwrap_or(Value::Null)
}

/// The modal's durable view of one operation.
async fn present(state: &AppState, t: &storage::Row) -> Value {
    let controller = if matches!(t.state.as_str(), "review" | "cancelled") {
        Value::Null
    } else {
        controller_status(state, &t.id).await
    };
    let progress = if controller["progress"].is_object() {
        controller["progress"].clone()
    } else {
        t.progress.clone()
    };
    json!({"id":t.id,"kind":t.kind,"service_id":t.service_id,"state":t.state,"step":t.step,"steps":steps(t),
        "review":t.review,"review_revision":t.review_revision,"choices":t.choices,"declarations":t.declarations,
        "checks":t.checks,"setup":t.setup,"recovery":t.recovery,"progress":progress,"error":t.error,
        "replacement_id":t.replacement_id,"approved_at":t.approved_at,"created_at":t.created_at,"updated_at":t.updated_at,
        "completed_at":t.completed_at,"actions":actions(t,&controller),
        "controller":if controller.is_null() {Value::Null} else {json!({"stage":controller["stage"],"boundary_at":controller["boundary_at"],
            "activated":controller["activated"],"committed":controller["committed"],"completed":controller["completed"],
            "operation_active":controller["operation_active"],"snapshot":controller["snapshot"],"qualification":controller["qualification"],
            "source":controller["source"],"generations":controller["generations"],"error":controller["error"]})}})
}

async fn load(state: &AppState, id: &str) -> Result<storage::Row> {
    valid_id(id)?;
    storage::load(&state.db, id.to_owned())
        .await?
        .ok_or_else(ApiError::not_found)
}

// ---------- Stage list and Connected actions ----------

/// Each service kind's stage, its available actions and any operation in progress.
async fn services(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let records = storage::stages(&state.db).await?;
    let controller = controller_request(&state, "/transfers", None).await.ok();
    let mut items = Vec::new();
    for kind in [
        "seerr",
        "recyclarr",
        "radarr",
        "sonarr",
        "lidarr",
        "bazarr",
        "prowlarr",
        "nzbget",
    ] {
        let of = |field: &str| {
            records[field]
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| r["kind"] == kind)
                .cloned()
        };
        let connection = of("connections");
        let ownership = of("ownership");
        let transfer = records["transfers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["kind"] == kind && r["state"] != "review")
            .max_by_key(|r| r["updated_at"].as_i64())
            .cloned();
        // Owned is published only when the controller agrees on the committed transfer.
        let agreed =
            match ownership.as_ref().and_then(|o| o["transfer_id"].as_str()) {
                None => true,
                Some(id) => controller.as_ref().is_some_and(|c| {
                    c["items"].as_array().into_iter().flatten().any(|t| {
                        t["id"] == id && (t["committed"] == true || t["completed"] == true)
                    })
                }),
            };
        let stage = match (&ownership, &connection) {
            (Some(_), _) if agreed => "owned",
            (_, Some(_)) => "connected",
            (Some(_), None) => "owned",
            _ => "none",
        };
        let hold = ownership
            .as_ref()
            .and_then(|o| o["hold"].as_str().map(str::to_owned));
        let operation = if let Some(t) = &transfer {
            Some(
                json!({"type":"transfer","id":t["id"],"state":t["state"],"step":t["step"],"error":t["error"]}),
            )
        } else if hold.as_deref() == Some("setup") {
            Some(
                json!({"type":"install","id":ownership.as_ref().map(|o| o["id"].clone()),"state":ownership.as_ref().map(|o| o["state"].clone()),"error":ownership.as_ref().map(|o| o["error"].clone())}),
            )
        } else {
            None
        };
        let actions: Vec<&str> = match stage {
            "connected" if transfer.is_some() => vec!["open_transfer"],
            "connected" => vec!["test", "edit", "forget", "take_ownership"],
            "owned" if hold.is_some() => vec!["open_transfer"],
            "owned" => vec!["manage"],
            _ => vec!["connect", "install"],
        };
        let draft = records["transfers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["kind"] == kind && r["state"] == "review")
            .map(|r| r["id"].clone());
        items.push(json!({"kind":kind,"stage":stage,"connection":connection,"ownership":ownership,"hold":hold,
            "operation":operation,"draft_review":draft,"actions":actions,"agreed":agreed}));
    }
    Ok(Json(json!({"items":items})))
}

/// Read-only health and identity validation of a saved connection.
async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let record = storage::connected(&state.db, id.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    match record.kind.as_str() {
        "radarr" | "sonarr" | "lidarr" => Ok(Json(super::test_manager(&state, &id).await?)),
        "recyclarr" => {
            let actor = security::require(&state, &headers, Capability::ManageServer).await?;
            validate_recyclarr(&state, &record.container, Some(actor.user.id)).await
        }
        _ => Ok(Json(support::test(&state, &id).await?)),
    }
}

async fn forget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    if !storage::forget(&state.db, id.clone(), actor.user.id).await? {
        return Err(ApiError::conflict(
            "Only a Connected service without an ownership transfer can be forgotten",
        ));
    }
    state.managers.connection_wake.notify_one();
    state
        .emit(None, "stack.changed", json!({"service_id":id}))
        .await?;
    Ok(Json(json!({"forgotten":true})))
}

#[derive(Deserialize)]
struct RecyclarrConnection {
    container_id: String,
}
/// Recyclarr has no HTTP API: its Connected stage validates the job container read-only.
async fn connect_recyclarr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RecyclarrConnection>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    validate_recyclarr(&state, &input.container_id, Some(actor.user.id)).await
}
async fn validate_recyclarr(
    state: &AppState,
    container: &str,
    actor: Option<String>,
) -> Result<Json<Value>> {
    let _guard = state.managers.guard.service("recyclarr").await;
    let observed = docker(state, &format!("containers/{container}")).await?;
    let reviewed = controller_request(
        state,
        "/transfers/validate",
        Some(json!({"kind":"recyclarr","container_id":observed["id"],"service_id":thelxinoe_core::id()})),
    )
    .await?;
    let error = reviewed["blockers"]
        .as_array()
        .and_then(|b| b.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    let record = storage::JobConnection {
        container: observed["id"].as_str().ok_or_else(unavailable)?.to_owned(),
        container_name: observed["name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('/')
            .to_owned(),
        engine: observed["engine"].as_str().unwrap_or("").to_owned(),
        version: reviewed["source"]["version"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        evidence: connection_evidence(&observed, ""),
        error: error.clone(),
    };
    let id = storage::connect_recyclarr(&state.db, record, actor)
        .await?
        .ok_or_else(|| ApiError::conflict("Thelxinoe already owns or is transferring Recyclarr"))?;
    state
        .emit(None, "stack.changed", json!({"service_id":id}))
        .await?;
    Ok(Json(
        json!({"id":id,"healthy":error.is_none(),"version":reviewed["source"]["version"],"error":error,
        "configuration":reviewed["inspection"]["format"]}),
    ))
}

// ---------- Review and approval ----------

/// Review a Connected service's takeover. Reopening returns the unfinished operation.
async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(service): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let record = storage::connected(&state.db, service.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    if let Some(active) = storage::active_for_kind(&state.db, record.kind.clone()).await? {
        return Ok(Json(present(&state, &load(&state, &active).await?).await));
    }
    if record.owned {
        return Err(ApiError::conflict("Thelxinoe already owns this service"));
    }
    let _guard = state.managers.guard.service(&record.kind).await;
    let mut blockers: Vec<String> = Vec::new();
    // Read-only API evidence: identity, version and activity, before anything stops.
    let api = match record.kind.as_str() {
        "recyclarr" => json!({}),
        "radarr" | "sonarr" | "lidarr" => match super::test_manager(&state, &service).await {
            Ok(value) => {
                if let Err(error) = operations::ensure_idle(&state, &service).await {
                    blockers.push(format!("{} is busy: {}. Wait for active imports, downloads or commands to finish, then review again", failures::label(&record.kind), error.2));
                }
                value
            }
            Err(error) => {
                blockers.push(format!(
                    "{}. Start the original and test its connection, then review again",
                    error.2
                ));
                json!({})
            }
        },
        _ => match support::test(&state, &service).await {
            Ok(value) => {
                if let Err(error) = support::ensure_idle(&state, &service).await {
                    blockers.push(format!("{} is busy: {}. Wait until it is idle, with no queued downloads or running jobs, then review again", failures::label(&record.kind), error.2));
                }
                value
            }
            Err(error) => {
                blockers.push(format!(
                    "{}. Start the original and test its connection, then review again",
                    error.2
                ));
                json!({})
            }
        },
    };
    let plan = controller_request(
        &state,
        "/transfers/review",
        Some(json!({"kind":record.kind,"container_id":record.container,"service_id":service})),
    )
    .await?;
    let id = plan["review_id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    blockers.extend(
        plan["blockers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    let recyclarr_targets = if record.kind == "recyclarr" {
        recyclarr_targets(&state, &plan, &mut blockers).await?
    } else {
        json!([])
    };
    let links = super::connections::transfer_links(&state, &service, &record.kind).await?;
    let dependencies = dependencies(&state, &plan).await?;
    let checks = checks::plan(&record.kind);
    let setup = checks::setup_actions(&record.kind);
    let review = json!({"plan":plan,"api":api,"blockers":blockers,"links":links,"dependencies":dependencies,
        "checks":checks,"setup_actions":setup,"recyclarr_targets":recyclarr_targets,
        "source_connection":{"container_id":record.container,"container_name":record.container_name,"port":record.port,"url_base":record.url_base,"engine":record.engine,
            "media_source":record.media_source,"evidence":record.evidence},
        "original":{"stops":true,"appdata_preserved":true,"restart_disabled":true}});
    let revision = digest(&json!([
        plan["revision"],
        blockers,
        links,
        dependencies,
        recyclarr_targets,
        Value::Null
    ]));
    storage::save_review(
        &state.db,
        storage::Draft {
            id: id.clone(),
            connected: record,
            actor: actor.user.id,
            review_revision: revision,
            review,
            steps: json!({}),
        },
    )
    .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Recyclarr's configured targets must be the reviewed, operationally ready Owned managers.
async fn recyclarr_targets(
    state: &AppState,
    plan: &Value,
    blockers: &mut Vec<String>,
) -> Result<Value> {
    let configured = plan["inspection"]["recyclarr"].clone();
    let managers = super::storage::list(&state.db).await?;
    let mut targets = Vec::new();
    for imported in plan["recyclarr"]["targets"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let kind = imported["kind"].as_str().unwrap_or_default();
        let Some(manager) = managers
            .iter()
            .find(|m| m["kind"] == kind && m["operational"] == true)
        else {
            blockers.push(format!(
                "Recyclarr synchronizes {}, which isn't an operational Owned service yet. Take ownership of it or install it first",
                failures::label(kind)
            ));
            continue;
        };
        let s = connected(state, manager["id"].as_str().ok_or_else(unavailable)?).await?;
        let c = Connection::open(state, &s).await?;
        // The same instance: Recyclarr's saved key authenticates against the Owned manager.
        if imported["secret"] != digest(&json!(c.key)) {
            blockers.push(format!(
                "Recyclarr's {} API key doesn't match the Owned {}; it targets a different instance",
                failures::label(kind),
                failures::label(kind)
            ));
            continue;
        }
        targets.push(json!({"service_id":s.id,"generation":s.generation,"kind":kind,"name":s.name,"trash_id":imported["trash_id"],
            "quality_sizes":imported["quality_sizes"],"groups":imported["groups"],"overrides":imported["overrides"],"guide_name":imported["guide_name"],
            "rewrite":{"before":imported["url"],"after":format!("Thelxinoe's managed {} connection",failures::label(kind))}}));
    }
    if targets.is_empty() && blockers.is_empty() {
        blockers
            .push("Recyclarr needs at least one configured Owned Radarr or Sonarr target".into());
    }
    let _ = configured;
    Ok(json!(targets))
}

/// Peers referenced by the copied configuration that Thelxinoe doesn't own stay external.
async fn dependencies(state: &AppState, plan: &Value) -> Result<Value> {
    let records = storage::stages(&state.db).await?;
    let mut found = Vec::new();
    for peer in plan["inspection"]["peers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let connected = records["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| {
                c["container_name"] == peer
                    || c["container_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(peer))
            });
        let (status, kind) = match connected {
            Some(c) if c["operational"] == true => ("owned", c["kind"].clone()),
            Some(c) => ("connected", c["kind"].clone()),
            None => ("external", Value::Null),
        };
        if status != "owned" {
            found.push(json!({"host":peer,"kind":kind,"status":status,
                "detail":"Remains externally managed; Thelxinoe keeps the reference and never changes that service"}));
        }
    }
    Ok(json!(found))
}

#[derive(Deserialize)]
struct Choices {
    #[serde(default)]
    compose_mode: Option<String>,
}
/// Changing a reviewed choice invalidates any pending confirmation.
async fn choices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Choices>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if !matches!(
        input.compose_mode.as_deref(),
        None | Some("scoped" | "project")
    ) {
        return Err(ApiError::bad(
            "Choose scoped detachment or project-wide retirement",
        ));
    }
    let choices = json!({"compose_mode":input.compose_mode});
    let revision = digest(&json!([
        t.review["plan"]["revision"],
        t.review["blockers"],
        t.review["links"],
        t.review["dependencies"],
        t.review["recyclarr_targets"],
        choices
    ]));
    if !storage::choices(&state.db, id.clone(), choices, revision).await? {
        return Err(ApiError::conflict(
            "The transfer is no longer being reviewed",
        ));
    }
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

#[derive(Deserialize)]
struct Approve {
    review_revision: String,
    confirm: bool,
    declarations: Declarations,
}
/// Administrator declarations about automation Thelxinoe cannot inspect.
#[derive(Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Declarations {
    cron: bool,
    systemd: bool,
    ci_cd: bool,
    configuration_management: bool,
    manual_compose: bool,
    unobservable_automation: bool,
    scoped_removed_from_files: bool,
    scoped_no_orphan_removal: bool,
    scoped_no_restart: bool,
    scoped_no_appdata_writes: bool,
    project_retired: bool,
    unrelated_services_acknowledged: bool,
}
async fn approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Approve>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if t.state != "review" {
        // Repeated submissions find the operation they already queued.
        return Ok(Json(present(&state, &t).await));
    }
    if !input.confirm {
        return Err(ApiError::bad("Confirm the reviewed transfer"));
    }
    if t.actor_id != actor.user.id {
        return Err(ApiError::conflict(
            "This review belongs to another administrator; review the transfer yourself",
        ));
    }
    if input.review_revision != t.review_revision {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "revision_changed",
            "The review changed; check it again before confirming".into(),
        ));
    }
    if let Some(blocker) = t.review["blockers"].as_array().and_then(|b| b.first()) {
        return Err(ApiError::conflict(
            blocker
                .as_str()
                .unwrap_or("Resolve the review's blocking problems first")
                .to_owned(),
        ));
    }
    let d = &input.declarations;
    if !(d.cron && d.systemd && d.ci_cd && d.configuration_management && d.manual_compose) {
        return Err(ApiError::bad(
            "Declare that off-host cron, systemd timers, CI/CD, configuration management and manual Compose commands can't change the original or its copy",
        ));
    }
    let plan = &t.review["plan"];
    if plan["automation"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|a| a["kind"] == "docker_access")
        && !d.unobservable_automation
    {
        return Err(ApiError::bad(
            "Declare that the containers with Docker access don't automate the original; Thelxinoe can't inspect them",
        ));
    }
    if plan["compose"].is_object() {
        match t.choices["compose_mode"].as_str() {
            Some("scoped")
                if d.scoped_removed_from_files
                    && d.scoped_no_orphan_removal
                    && d.scoped_no_restart
                    && d.scoped_no_appdata_writes => {}
            Some("scoped") => {
                return Err(ApiError::bad(
                    "Scoped detachment needs restart, recreation, deletion and appdata writes all excluded",
                ));
            }
            Some("project")
                if d.project_retired
                    && (d.unrelated_services_acknowledged
                        || plan["compose"]["unrelated"]
                            .as_array()
                            .is_none_or(Vec::is_empty)) => {}
            Some("project") => {
                return Err(ApiError::bad(
                    "Confirm the Compose project is retired and acknowledge its unrelated services",
                ));
            }
            _ => {
                return Err(ApiError::bad(
                    "Choose scoped detachment or project-wide retirement for the Compose project",
                ));
            }
        }
    }
    // Observed Docker evidence and administrator declarations are recorded separately.
    let declarations = json!({"declared":serde_json::to_value(d).map_err(|_| unavailable())?,
        "compose_mode":t.choices["compose_mode"],"observed":{"automation":plan["automation"],"compose":plan["compose"]},
        "verified_by_thelxinoe":false,"at":now()});
    match storage::approve(&state.db, id.clone(), actor.user.id, declarations).await? {
        storage::Approval::Queued => {}
        storage::Approval::Stale => {
            return Err(ApiError(
                axum::http::StatusCode::CONFLICT,
                "revision_changed",
                "The connection changed after review; review the transfer again".into(),
            ));
        }
        storage::Approval::Busy => {
            return Err(ApiError::conflict(
                "Another transfer of this service is in progress",
            ));
        }
    }
    state
        .emit(None, "stack.changed", json!({"transfer":id}))
        .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    let items = rows
        .iter()
        .map(|t| json!({"id":t.id,"kind":t.kind,"service_id":t.service_id,"state":t.state,"step":t.step,"error":t.error,"updated_at":t.updated_at}))
        .collect::<Vec<_>>();
    Ok(Json(json!({"items":items})))
}

async fn view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Cancel only before the original changes. Closing the modal never cancels.
async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    match t.state.as_str() {
        "cancelled" => {}
        "review" => {
            let _ = controller_request(&state, &format!("/transfers/{id}/cancel"), Some(json!({})))
                .await;
            storage::outcome(
                &state.db,
                id.clone(),
                "cancelled".into(),
                None,
                Some(actor.user.id),
            )
            .await?;
        }
        "queued" | "running" => {
            let _guard = state.managers.guard.service(&t.kind).await;
            match controller_request(&state, &format!("/transfers/{id}/cancel"), Some(json!({})))
                .await
            {
                Ok(_) => {}
                Err(error) if error.1 == "not_found" => {}
                Err(error) => return Err(error),
            }
            storage::outcome(
                &state.db,
                id.clone(),
                "cancelled".into(),
                None,
                Some(actor.user.id),
            )
            .await?;
        }
        _ => {
            return Err(ApiError::conflict(
                "The original already changed; use the recovery options instead",
            ));
        }
    }
    state
        .emit(None, "stack.changed", json!({"transfer":id}))
        .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Repeat setup and required checks of a committed transfer that failed them.
async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if t.state != "failed" || t.replacement_id.is_none() {
        return Err(ApiError::conflict(
            "Only a committed transfer that failed its checks can be retried; review again otherwise",
        ));
    }
    storage::update(
        &state.db,
        id.clone(),
        Some("committed".into()),
        Some("setup".into()),
        None,
        Some(None),
    )
    .await?;
    storage::requeue(&state.db, id.clone(), 0).await?;
    storage::recovery(
        &state.db,
        id.clone(),
        t.recovery.clone(),
        None,
        Some(actor.user.id),
        "transfer.retry",
    )
    .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Restore the original after a failure that never reached production.
async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    let _guard = state.managers.guard.service(&t.kind).await;
    controller_request(&state, &format!("/transfers/{id}/restore"), Some(json!({}))).await?;
    storage::outcome(
        &state.db,
        id.clone(),
        "returned".into(),
        t.error.clone(),
        Some(actor.user.id),
    )
    .await?;
    state
        .emit(None, "stack.changed", json!({"transfer":id}))
        .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

// ---------- The transfer job ----------

/// Drive one approved transfer from its durable state. Returns false when it waits.
pub(crate) async fn run_job(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<bool> {
    let id = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing transfer identity"))?
        .to_owned();
    let t = storage::load(&state.db, id.clone())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Transfer not found"))?;
    let outcome = match t.state.as_str() {
        "queued" | "running" => drive(state, &t).await,
        "committed" => finish(state, &t).await.map(|()| true),
        _ => return Ok(true),
    };
    match outcome {
        Ok(true) => {
            state
                .emit(None, "stack.changed", json!({"transfer":id}))
                .await?;
            Ok(true)
        }
        Ok(false) => {
            storage::requeue(&state.db, id, 5).await?;
            Ok(false)
        }
        Err(error) => {
            // A failed post-commit transfer stays Owned under its hold with recovery available.
            storage::checks(
                &state.db,
                id.clone(),
                storage::load(&state.db, id.clone())
                    .await?
                    .map(|t| t.checks)
                    .unwrap_or_default(),
                t.step.clone(),
                Some(error.2.clone()),
            )
            .await?;
            state
                .emit(None, "stack.changed", json!({"transfer":id}))
                .await?;
            Ok(true)
        }
    }
}

async fn wait(state: &AppState, t: &storage::Row, reason: String) -> Result<bool> {
    storage::update(
        &state.db,
        t.id.clone(),
        None,
        None,
        Some(json!({"waiting":reason})),
        None,
    )
    .await?;
    Ok(false)
}

/// Start or follow the controller's journal; commit once the replacement is activated.
async fn drive(state: &AppState, t: &storage::Row) -> Result<bool> {
    // A restart after the controller accepted the start must follow its journal, not
    // recheck prerequisites against an original that may already be stopped.
    let started = t.state == "queued"
        && controller_status(state, &t.id).await["stage"]
            .as_str()
            .is_some_and(|stage| !matches!(stage, "" | "review" | "approved"));
    if started {
        storage::update(
            &state.db,
            t.id.clone(),
            Some("running".into()),
            None,
            None,
            None,
        )
        .await?;
    }
    if t.state == "queued" && !started {
        // Required peers must be operationally ready; unrelated services keep working meanwhile.
        for target in t.review["recyclarr_targets"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let id = target["service_id"].as_str().unwrap_or_default();
            if !ownership::operational(state, id).await? {
                return wait(
                    state,
                    t,
                    format!(
                        "Waiting for {} to become an operational Owned service",
                        failures::label(target["kind"].as_str().unwrap_or("its target"))
                    ),
                )
                .await;
            }
        }
        // Recheck activity immediately before the original stops.
        let busy = match t.kind.as_str() {
            "recyclarr" => Ok(()),
            "radarr" | "sonarr" | "lidarr" => operations::ensure_idle(state, &t.service_id).await,
            _ => support::ensure_idle(state, &t.service_id).await,
        };
        if let Err(error) = busy {
            return wait(
                state,
                t,
                format!(
                    "Waiting for {} to become idle: {}",
                    failures::label(&t.kind),
                    error.2
                ),
            )
            .await;
        }
        let started = controller_request(
            state,
            &format!("/transfers/{}/start", t.id),
            Some(json!({"revision":t.review["plan"]["revision"],"declarations":t.declarations,"compose_mode":t.choices["compose_mode"]})),
        )
        .await;
        match started {
            Ok(_) => {}
            Err(error) if error.1 == "revision_changed" => {
                storage::outcome(
                    &state.db,
                    t.id.clone(),
                    "returned".into(),
                    Some(format!("{}; nothing changed", error.2)),
                    None,
                )
                .await?;
                return Ok(true);
            }
            Err(error) => return Err(error),
        }
        storage::update(
            &state.db,
            t.id.clone(),
            Some("running".into()),
            Some("stop".into()),
            Some(json!({})),
            Some(None),
        )
        .await?;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        let status = controller_request(state, &format!("/transfers/{}", t.id), None).await?;
        let stage = status["stage"].as_str().unwrap_or("").to_owned();
        match stage.as_str() {
            "activated" | "committed" | "complete" => break,
            "returned" | "stale" => {
                storage::outcome(
                    &state.db,
                    t.id.clone(),
                    "returned".into(),
                    status["error"].as_str().map(str::to_owned).or(Some(
                        "The transfer stopped before production and restored the original".into(),
                    )),
                    None,
                )
                .await?;
                return Ok(true);
            }
            "cancelled" => {
                storage::outcome(&state.db, t.id.clone(), "cancelled".into(), None, None).await?;
                return Ok(true);
            }
            "failed" | "recovery-required" => {
                storage::update(
                    &state.db,
                    t.id.clone(),
                    Some("failed".into()),
                    Some(controller_step(&stage).into()),
                    None,
                    Some(status["error"].as_str().map(str::to_owned)),
                )
                .await?;
                return Ok(true);
            }
            _ => {}
        }
        storage::update(
            &state.db,
            t.id.clone(),
            None,
            Some(controller_step(&stage).into()),
            Some(status["progress"].clone()),
            None,
        )
        .await?;
        if status["operation_active"] == false {
            // An interrupted controller resumes from its own journal.
            controller_request(
                state,
                &format!("/transfers/{}/resume", t.id),
                Some(json!({})),
            )
            .await?;
        }
        if tokio::time::Instant::now() > deadline {
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    commit(state, t).await?;
    let committed = storage::load(&state.db, t.id.clone())
        .await?
        .ok_or_else(unavailable)?;
    finish(state, &committed).await?;
    Ok(true)
}

/// Commit the activated replacement under its transfer hold, then record controller agreement.
async fn commit(state: &AppState, t: &storage::Row) -> Result<()> {
    let status = controller_request(state, &format!("/transfers/{}", t.id), None).await?;
    let replacement = status["replacement"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let observed = docker(state, &format!("containers/{replacement}")).await?;
    let template = controller_request(state, "/templates", None).await?["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| item["kind"] == t.kind.as_str())
        .cloned()
        .ok_or_else(unavailable)?;
    let port = template["port"].as_u64().unwrap_or(0) as u16;
    let url_base = thelxinoe_core::service_url_base(&t.kind).to_owned();
    // Transfer-scoped, read-only identity check of the exact replacement before commit.
    // The reviewed replacement's own storage mapping becomes the recorded layout.
    let mut media_source = String::new();
    let credential = match t.kind.as_str() {
        "recyclarr" => None,
        "radarr" | "sonarr" | "lidarr" => {
            let s = connected(state, &t.service_id).await?;
            let key = String::from_utf8(
                state
                    .secrets
                    .decrypt(&format!("manager:{}", s.id), &s.credential)?,
            )
            .map_err(|_| unavailable())?;
            let (base, media) = evidence(state, &replacement, port).await?;
            media_source = media;
            let c =
                Connection::validation(state, base, url_base.clone(), key.clone(), t.kind.clone());
            let status = c.get("system/status").await?;
            if !status["appName"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(&t.kind))
            {
                return Err(failures::identity(&t.kind, &status["appName"]));
            }
            Some(
                state
                    .secrets
                    .encrypt(&format!("provision:{}", t.id), key.as_bytes())?,
            )
        }
        _ => {
            let s = support::load_connected(state, &t.service_id).await?;
            let (base, media) = support::api_evidence(state, &replacement, port, &t.kind).await?;
            media_source = media;
            let c = Connection::validation(
                state,
                base,
                url_base.clone(),
                s.credentials.secret.clone(),
                t.kind.clone(),
            );
            support::version(&c, &s.credentials).await?;
            let stored = super::stack::ProvisionCredentials {
                credentials: s.credentials.clone(),
            };
            Some(state.secrets.encrypt(
                &format!("provision:{}", t.id),
                &serde_json::to_vec(&stored).map_err(|_| unavailable())?,
            )?)
        }
    };
    storage::commit(
        &state.db,
        storage::Commit {
            id: t.id.clone(),
            kind: t.kind.clone(),
            service: t.service_id.clone(),
            actor: t.approved_by.clone().unwrap_or_else(|| t.actor_id.clone()),
            replacement: replacement.clone(),
            container_name: observed["name"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches('/')
                .to_owned(),
            port,
            url_base,
            media_source,
            evidence: connection_evidence(&observed, ""),
            host_port: t.review["plan"]["target"]["published_port"]
                .as_u64()
                .map(|p| p as u16),
            credential,
        },
    )
    .await?;
    controller_request(
        state,
        &format!("/transfers/{}/commit", t.id),
        Some(json!({"container_id":replacement})),
    )
    .await?;
    Ok(())
}

/// Reviewed setup, required checks, then completion with hold release in one transaction.
async fn finish(state: &AppState, t: &storage::Row) -> Result<()> {
    let status = controller_request(state, &format!("/transfers/{}", t.id), None).await?;
    if status["committed"] != true {
        // Reconcile a commit the controller didn't record before continuing.
        let replacement = t.replacement_id.clone().ok_or_else(unavailable)?;
        controller_request(
            state,
            &format!("/transfers/{}/commit", t.id),
            Some(json!({"container_id":replacement})),
        )
        .await?;
    }
    storage::update(
        &state.db,
        t.id.clone(),
        None,
        Some("setup".into()),
        None,
        None,
    )
    .await?;
    checks::setup(state, t).await?;
    let t = storage::load(&state.db, t.id.clone())
        .await?
        .ok_or_else(unavailable)?;
    storage::update(
        &state.db,
        t.id.clone(),
        None,
        Some("checks".into()),
        None,
        None,
    )
    .await?;
    let mut row = t.clone();
    row.review["source_state"] = json!({"records":status["source_records"]});
    let results = checks::evaluate(state, &row, &status).await?;
    let failed: Vec<&Value> = results
        .iter()
        .filter(|c| c["required"] == true && c["status"] != "passed")
        .collect();
    if let Some(first) = failed.first() {
        let message = format!(
            "Required check failed: {}{}",
            first["label"].as_str().unwrap_or("check"),
            first["detail"]
                .as_str()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default()
        );
        storage::checks(
            &state.db,
            t.id.clone(),
            json!(results),
            "checks".into(),
            Some(message.clone()),
        )
        .await?;
        return Err(ApiError::conflict(message));
    }
    storage::checks(
        &state.db,
        t.id.clone(),
        json!(results),
        "complete".into(),
        None,
    )
    .await?;
    storage::complete(&state.db, t.id.clone()).await?;
    // The server released its hold durably; the controller hold follows and is reconciled on retry.
    controller_request(
        state,
        &format!("/transfers/{}/complete", t.id),
        Some(json!({})),
    )
    .await?;
    state.managers.connection_wake.notify_one();
    Ok(())
}

// ---------- Recovery after the production boundary ----------

async fn recovery_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if !matches!(t.state.as_str(), "failed" | "suspended" | "recovery") {
        return Err(ApiError::conflict(
            "Recovery is available for a failed or suspended transfer",
        ));
    }
    let plan = controller_request(&state, &format!("/transfers/{id}/recovery"), None).await?;
    // Transfer setup changed only the replacement itself; peer services were never written.
    let written = t.setup.as_array().cloned().unwrap_or_default();
    let mut recovery = plan.clone();
    recovery["transfer_written"] = json!(written);
    recovery["peer_compensation"] = json!([]);
    recovery["peer_note"] = json!(
        "Takeover setup doesn't write peer services; links are created only after completion"
    );
    storage::recovery(
        &state.db,
        id.clone(),
        recovery.clone(),
        None,
        Some(actor.user.id),
        "transfer.recovery.plan",
    )
    .await?;
    Ok(Json(recovery))
}

#[derive(Deserialize)]
struct Recover {
    action: String,
    revision: String,
    #[serde(default)]
    approval: Value,
}
async fn recover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Recover>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if !matches!(t.state.as_str(), "failed" | "suspended" | "recovery") {
        return Err(ApiError::conflict(
            "Recovery is available for a failed or suspended transfer",
        ));
    }
    let _lease = state.managers.maintenance(&state).await;
    let _guard = state.managers.guard.service(&t.kind).await;
    storage::update(
        &state.db,
        id.clone(),
        Some("recovery".into()),
        Some("recovery".into()),
        None,
        None,
    )
    .await?;
    let result = controller_request(
        &state,
        &format!("/transfers/{id}/recovery"),
        Some(json!({"action":input.action,"revision":input.revision,"approval":input.approval})),
    )
    .await;
    let mut recovery = t.recovery.clone();
    recovery["executed"] = json!({"action":input.action,"approval":input.approval,"by":actor.user.id,"at":now(),
        "outcome":match &result {Ok(_) => json!("applied"), Err(error) => json!(error.2)}});
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            storage::recovery(
                &state.db,
                id.clone(),
                recovery,
                Some(t.state.clone()),
                Some(actor.user.id),
                "transfer.recovery.failed",
            )
            .await?;
            return Err(error);
        }
    };
    match input.action.as_str() {
        "return_original" => {
            let source = &t.review["source_connection"];
            storage::return_original(
                &state.db,
                storage::Original {
                    id: id.clone(),
                    kind: t.kind.clone(),
                    service: t.service_id.clone(),
                    container: source["container_id"]
                        .as_str()
                        .unwrap_or(&t.container_id)
                        .to_owned(),
                    container_name: source["container_name"].as_str().unwrap_or("").to_owned(),
                    port: source["port"].as_u64().unwrap_or(0) as u16,
                    url_base: source["url_base"].as_str().unwrap_or("").to_owned(),
                    media_source: source["media_source"].as_str().unwrap_or("").to_owned(),
                    evidence: source["evidence"].clone(),
                    engine: t.engine.clone(),
                    actor: actor.user.id.clone(),
                },
            )
            .await?;
            storage::recovery(
                &state.db,
                id.clone(),
                recovery,
                None,
                Some(actor.user.id),
                "transfer.recovery.return",
            )
            .await?;
        }
        "suspend" => {
            storage::recovery(
                &state.db,
                id.clone(),
                recovery,
                Some("suspended".into()),
                Some(actor.user.id),
                "transfer.recovery.suspend",
            )
            .await?;
        }
        _ => {
            // Repair or a restored candidate repeats commit agreement, setup and required checks.
            let next = if t.replacement_id.is_some() {
                "committed"
            } else {
                "running"
            };
            if next == "committed"
                && status["replacement"]
                    != t.replacement_id
                        .as_deref()
                        .map(Value::from)
                        .unwrap_or(Value::Null)
            {
                storage::rebind(
                    &state.db,
                    id.clone(),
                    status["replacement"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
                .await?;
            }
            storage::recovery(
                &state.db,
                id.clone(),
                recovery,
                Some(next.into()),
                Some(actor.user.id),
                "transfer.recovery.continue",
            )
            .await?;
            storage::requeue(&state.db, id.clone(), 0).await?;
        }
    }
    state.managers.connection_wake.notify_one();
    state
        .emit(None, "stack.changed", json!({"transfer":id}))
        .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Remove the stopped original container after success; its appdata is never deleted.
async fn remove_original(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let t = load(&state, &id).await?;
    if t.state != "complete" {
        return Err(ApiError::conflict(
            "Remove the original only after the transfer completes",
        ));
    }
    controller_request(
        &state,
        &format!("/transfers/{id}/original/remove"),
        Some(json!({})),
    )
    .await?;
    storage::recovery(
        &state.db,
        id.clone(),
        t.recovery.clone(),
        None,
        Some(actor.user.id),
        "transfer.original.remove",
    )
    .await?;
    Ok(Json(present(&state, &load(&state, &id).await?).await))
}

/// Background checks of Connected services: read-only health and identity only, at most
/// every ten minutes per service. They never change upstream settings.
pub(crate) async fn connected_health(state: &AppState) -> anyhow::Result<()> {
    let records = storage::stages(&state.db).await?;
    let ownership = records["ownership"].as_array().cloned().unwrap_or_default();
    for connection in records["connections"].as_array().into_iter().flatten() {
        let kind = connection["kind"].as_str().unwrap_or_default();
        let id = connection["id"].as_str().unwrap_or_default();
        let due = connection["checked_at"].as_i64().unwrap_or(0)
            < now() - if kind == "recyclarr" { 1800 } else { 600 };
        if connection["operational"] == true || !due || ownership.iter().any(|o| o["kind"] == kind)
        {
            continue;
        }
        let _ = match kind {
            "radarr" | "sonarr" | "lidarr" => super::test_manager(state, id).await.map(|_| ()),
            "recyclarr" => {
                let container = connection["container_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                validate_recyclarr(state, &container, None)
                    .await
                    .map(|_| ())
            }
            _ => support::test(state, id).await.map(|_| ()),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, state: &str, step: &str, setup_actions: Value) -> storage::Row {
        storage::Row {
            id: "transfer".into(),
            kind: kind.into(),
            service_id: "service".into(),
            actor_id: "alice".into(),
            container_id: "container".into(),
            engine: "engine".into(),
            connection_revision: "revision".into(),
            review_revision: "review".into(),
            review: json!({"setup_actions":setup_actions,"blockers":[]}),
            choices: json!({}),
            declarations: json!({}),
            state: state.into(),
            step: step.into(),
            steps: json!({}),
            checks: json!([]),
            setup: json!([]),
            recovery: json!({}),
            progress: json!({}),
            replacement_id: None,
            error: None,
            approved_by: None,
            approved_at: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
        }
    }
    fn status(steps: &[Value], key: &str) -> String {
        steps.iter().find(|s| s["key"] == key).unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn steps_follow_the_operation_and_skip_unreviewed_setup() {
        let hosts = json!([{"action":"prowlarr.hosts"}]);
        let review = steps(&row("radarr", "review", "review", json!([])));
        assert_eq!(status(&review, "prerequisites"), "complete");
        assert_eq!(status(&review, "automation"), "awaiting_input");
        assert_eq!(status(&review, "stop"), "pending");
        assert_eq!(status(&review, "setup"), "skipped");
        let running = steps(&row("prowlarr", "running", "copy", hosts.clone()));
        assert_eq!(status(&running, "stop"), "complete");
        assert_eq!(status(&running, "copy"), "running");
        assert_eq!(status(&running, "setup"), "pending");
        let failed = steps(&row("prowlarr", "failed", "setup", hosts));
        assert_eq!(status(&failed, "setup"), "failed");
        let done = steps(&row("radarr", "complete", "complete", json!([])));
        assert_eq!(status(&done, "checks"), "complete");
        assert_eq!(status(&done, "setup"), "skipped");
    }
}
