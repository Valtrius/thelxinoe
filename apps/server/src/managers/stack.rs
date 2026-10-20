//! Server-owned provisioning jobs keep credentials out of job payloads and controller responses.
//! Fresh installations enter the Owned stage directly and stay held until setup checks pass.

#[path = "../storage/managers/stack.rs"]
mod storage;

use super::*;
#[cfg(test)]
#[path = "stack_recovery_tests.rs"]
mod recovery_tests;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/stack", get(list))
        .route("/api/v1/admin/stack/templates", get(templates))
        .route("/api/v1/admin/stack/releases", get(releases))
        .route("/api/v1/admin/stack/install", post(install))
        .route("/api/v1/admin/stack/{id}/login", post(login))
        .route("/api/v1/admin/stack/wire", post(wire))
        .route("/api/v1/admin/stack/{id}/action", post(action))
        .route("/api/v1/admin/stack/{id}/retry", post(retry))
}
pub(crate) async fn controller(state: &AppState, path: &str, body: Option<Value>) -> Result<Value> {
    // Release validation holds the controller's lock; explain instead of its refusal.
    if body.is_some() && !path.starts_with("/product") && crate::product::validating(state) {
        return Err(ApiError::conflict(
            "A server update is being validated; try again in a few minutes",
        ));
    }
    #[cfg(test)]
    if let Ok((code, message)) = crate::test_support::CONTROLLER_FAILURE.try_with(|v| *v) {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            code,
            message.into(),
        ));
    }
    #[cfg(test)]
    if let Some(value) = state
        .managers
        .docker
        .lock()
        .unwrap()
        .get(&format!("stack{path}"))
    {
        // Tests script a controller refusal as {"__error":[code,message]}.
        if let Some([code, message]) = value["__error"].as_array().map(Vec::as_slice) {
            let code = match code.as_str() {
                Some("revision_changed") => "revision_changed",
                Some("not_found") => "not_found",
                Some("dependency_unavailable") => "dependency_unavailable",
                Some("operation_contended") => "operation_contended",
                _ => "conflict",
            };
            return Err(ApiError(
                axum::http::StatusCode::CONFLICT,
                code,
                message.as_str().unwrap_or_default().into(),
            ));
        }
        return Ok(value.clone());
    }
    #[cfg(unix)]
    {
        let client = reqwest::Client::builder()
            .unix_socket(state.config.controller_socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(900))
            .build()
            .map_err(|_| unavailable())?;
        let mut request = client.request(
            if body.is_some() {
                reqwest::Method::POST
            } else {
                reqwest::Method::GET
            },
            format!("http://controller/stack{path}"),
        );
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| failures::controller_unreachable())?;
        if !response.status().is_success() {
            let status = response.status();
            tracing::warn!(
                path,
                status = status.as_u16(),
                "Docker controller request failed"
            );
            let message = message(&response.text().await.unwrap_or_default());
            let code = if status.is_server_error() {
                "dependency_unavailable"
            } else if matches!(
                status,
                reqwest::StatusCode::LOCKED | reqwest::StatusCode::TOO_MANY_REQUESTS
            ) {
                "operation_contended"
            } else if status == reqwest::StatusCode::PRECONDITION_FAILED {
                "revision_changed"
            } else if status == reqwest::StatusCode::NOT_FOUND {
                "not_found"
            } else {
                "conflict"
            };
            return Err(ApiError(axum::http::StatusCode::CONFLICT, code, message));
        }
        read(response).await
    }
    #[cfg(not(unix))]
    {
        let _ = (state, path, body);
        Err(ApiError::conflict(
            "Managed Docker services require the Linux controller",
        ))
    }
}
/// The controller only returns curated explanations, including for Docker and
/// registry failures; its generic outage texts point to its log instead.
pub(crate) fn message(message: &str) -> String {
    let message = message.trim();
    if message.is_empty()
        || matches!(
            message,
            "Docker inspection unavailable" | "Docker is not reachable"
        )
    {
        "The Docker controller couldn't complete the request; check that Docker is running and see the controller log".into()
    } else {
        message.chars().take(300).collect()
    }
}
async fn templates(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(controller(&state, "/templates", None).await?))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut value = controller(&state, "", None).await?;
    value["provisions"] = storage::list(&state.db).await?;
    let provisions = value["provisions"]
        .as_array()
        .ok_or_else(unavailable)?
        .clone();
    for item in value["items"].as_array_mut().ok_or_else(unavailable)? {
        let provision = provisions
            .iter()
            .find(|provision| provision["id"] == item["id"] && provision["kind"] == item["kind"]);
        item["registered"] = json!(provision.is_some());
        // Lifecycle and removal wait for setup, transfer or recovery holds.
        let held = provision.is_none_or(|p| !p["hold"].is_null());
        if held {
            item["can_recreate"] = json!(false);
            item["can_retire"] = json!(false);
            item["can_remove"] = json!(false);
        }
    }
    Ok(Json(value))
}
/// NZBGet's control login, for Owned services whose normal operations are released.
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (kind, integration) = storage::login(key.clone(), &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if kind != "nzbget" {
        return Err(ApiError::not_found());
    }
    let service = support::load(&state, &integration).await?;
    Ok(Json(
        json!({"username":service.credentials.username,"password":service.credentials.secret}),
    ))
}
#[derive(Deserialize)]
struct Install {
    kind: String,
    host_port: Option<u16>,
    #[serde(default)]
    native_url: String,
}
async fn install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Install>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !support::native_url(&input.native_url)
        || input.native_url.len() > 2000
        || (input.kind != "recyclarr" && !input.host_port.is_some_and(|p| p >= 1024))
        || (input.kind == "recyclarr"
            && (input.host_port.is_some() || !input.native_url.is_empty()))
        || !matches!(
            input.kind.as_str(),
            "radarr"
                | "sonarr"
                | "lidarr"
                | "bazarr"
                | "prowlarr"
                | "nzbget"
                | "seerr"
                | "recyclarr"
        )
    {
        return Err(ApiError::bad("Choose a supported service and local port"));
    }
    let key = id();
    let returned = key.clone();
    let credential = if input.kind == "recyclarr" {
        None
    } else {
        Some(state.secrets.encrypt(
            &format!("provision:{key}"),
            id().replace('-', "").as_bytes(),
        )?)
    };
    let inserted = storage::install(&state.db, input, p, key, credential).await?;
    if !inserted {
        return Err(ApiError::conflict(
            "This service is already connected, being transferred or managed. Forget the connection or take ownership instead",
        ));
    }
    Ok(Json(json!({"id":returned,"state":"queued"})))
}
#[derive(Deserialize)]
struct Action {
    action: String,
}

pub(super) fn confirmed_stopped(observed: &Value) -> Result<bool> {
    if observed["existence"] != "present"
        || observed["drift"] != false
        || observed["phase"] != "active"
    {
        return Err(ApiError::conflict(
            "A present, accepted container with known Docker state is required",
        ));
    }
    observed["running"]
        .as_bool()
        .map(|running| !running)
        .ok_or_else(unavailable)
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&key).is_err()
        || !matches!(
            input.action.as_str(),
            "start" | "stop" | "restart" | "reconcile" | "recreate" | "retire" | "remove"
        )
    {
        return Err(ApiError::bad("Invalid managed service action"));
    }
    let owned = storage::owned(&state.db, key.clone())
        .await?
        .ok_or_else(|| ApiError::conflict("This saved installation has no matching server record. Restore the matching server profile before managing it."))?;
    // Transfers and their recovery use the guided ownership workflow; failed setup can be
    // retired or removed, but normal lifecycle actions wait for the hold to be released.
    match owned.hold.as_deref() {
        Some("transfer" | "recovery") => {
            return Err(ApiError::conflict(
                "This service is held by its ownership transfer. Continue it or use its recovery options in Media services",
            ));
        }
        Some(_) if !matches!(input.action.as_str(), "retire" | "remove" | "reconcile") => {
            return Err(ApiError::conflict(
                "Finish setting up this service before managing its container",
            ));
        }
        _ => {}
    }
    let kind = owned.kind;
    let _lease = state.managers.maintenance(&state).await;
    let _guard = state.managers.guard.service(&kind).await;
    if input.action == "remove" {
        let observed = controller(&state, "", None).await?;
        if observed["items"]
            .as_array()
            .ok_or_else(unavailable)?
            .iter()
            .any(|s| s["id"] == key && s["can_remove"] != true)
        {
            return Err(ApiError::conflict(
                "Stop the service before removing its configuration",
            ));
        }
        if !storage::begin_retirement(&state.db, key.clone(), input.action.clone()).await? {
            return Err(ApiError::conflict(
                "Finish the current setup or update before removal",
            ));
        }
        let result = controller(
            &state,
            &format!("/{key}/action"),
            Some(json!({"action":"remove"})),
        )
        .await?;
        if result["removed"] != true {
            return Err(ApiError::conflict("Controller did not confirm removal"));
        }
        storage::retire(&state.db, key.clone(), p.user.id, true).await?;
        state.managers.connection_wake.notify_one();
        state.emit(None, "stack.changed", json!({"id":key})).await?;
        return Ok(Json(result));
    }
    if input.action == "retire" {
        let observed = controller(&state, "", None).await?;
        if observed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["id"] == key && s["existence"] != "missing")
        {
            return Err(ApiError::conflict(
                "Only a confirmed missing container can be retired",
            ));
        }
        if !storage::begin_retirement(&state.db, key.clone(), input.action.clone()).await? {
            return Err(ApiError::conflict(
                "Finish the current setup or update before retiring this installation",
            ));
        }
        let result = controller(
            &state,
            &format!("/{key}/action"),
            Some(json!({"action":"retire"})),
        )
        .await?;
        if result["retired"] != true {
            return Err(ApiError::conflict("Controller did not confirm retirement"));
        }
        storage::retire(&state.db, key.clone(), p.user.id, false).await?;
        state.managers.connection_wake.notify_one();
        state.emit(None, "stack.changed", json!({"id":key})).await?;
        return Ok(Json(result));
    }
    // A confirmed stopped service needs no live API idle check. Inspection
    // failures and missing containers never count as stopped.
    let container = controller(&state, "", None).await?["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"] == key)
        .cloned()
        .ok_or_else(ApiError::not_found)?;
    if matches!(input.action.as_str(), "stop" | "restart")
        && !confirmed_stopped(&container)?
        && container["workload"] != "job"
    {
        let c = container["container_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let (kind, service) = storage::action_read_service(&state.db, c)
            .await?
            .ok_or_else(|| ApiError::conflict("Finish setting up the service API first"))?;
        if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            operations::ensure_idle(&state, &service).await?;
        } else {
            support::ensure_idle(&state, &service).await?;
        }
    }
    let result = controller(
        &state,
        &format!("/{key}/action"),
        Some(json!({"action":input.action})),
    )
    .await?;
    if matches!(input.action.as_str(), "reconcile" | "recreate") {
        let container = result["container_id"]
            .as_str()
            .ok_or_else(unavailable)?
            .to_owned();
        let key = key.clone();
        storage::action_write_stack_provisions(&state.db, key, container).await?;
    }
    if input.action == "start" {
        let container = container["container_id"]
            .as_str()
            .ok_or_else(unavailable)?
            .to_owned();
        storage::action_write_stack_provisions(&state.db, key.clone(), container).await?;
    }
    state.emit(None, "stack.changed", json!({"id":key})).await?;
    storage::action_write_audit(&state.db, key, input, p).await?;
    Ok(Json(result))
}
async fn progress(
    state: &AppState,
    key: &str,
    stage: &str,
    container: Option<String>,
    error: Option<String>,
) -> Result<()> {
    let key = key.to_owned();
    let stage = stage.to_owned();
    storage::progress(key, stage, &state.db, container, error).await?;
    Ok(())
}

/// Install or reverify an Owned service. It stays under its `setup` hold until the
/// replacement's API is authenticated and every setup action finished.
pub(crate) async fn provision(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let key = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing provision identity"))?
        .to_owned();
    let row = storage::provision_read_stack_provisions(key.clone(), &state.db).await?;
    if row.state == "complete" && row.hold.is_none() {
        return Ok(());
    }
    if row.state == "retiring" {
        anyhow::bail!("Installation is being retired");
    }
    if row.hold.as_deref().is_some_and(|hold| hold != "setup") {
        anyhow::bail!("This service is held by its ownership transfer");
    }
    if !storage::provision_read_users(row.actor.clone(), &state.db).await? {
        progress(
            state,
            &key,
            "blocked",
            None,
            Some("Provisioning administrator no longer has access".into()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        anyhow::bail!("Provisioning administrator no longer has access");
    }
    let result = setup(state, &key, &row).await;
    if let Err(error) = result {
        progress(state, &key, "blocked", None, Some(error.2.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        anyhow::bail!("{}", error.2);
    }
    // Setup completion and hold release are one durable event.
    storage::complete_setup(&state.db, key.clone()).await?;
    state.managers.connection_wake.notify_one();
    state.emit(None, "stack.changed", json!({"id":key})).await?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
pub(super) struct ProvisionCredentials {
    #[serde(flatten)]
    pub(super) credentials: support::Credentials,
}

async fn setup(state: &AppState, key: &str, row: &storage::Provision) -> Result<()> {
    let secret_input = if let Some(encrypted) = &row.credential {
        String::from_utf8(
            state
                .secrets
                .decrypt(&format!("provision:{key}"), encrypted)?,
        )
        .map_err(|_| unavailable())?
    } else {
        String::new()
    };
    let credentials = serde_json::from_str::<ProvisionCredentials>(&secret_input)
        .map(|stored| stored.credentials)
        .unwrap_or(support::Credentials {
            username: "thelxinoe".into(),
            secret: secret_input,
        });
    let templates = controller(state, "/templates", None).await?;
    let template = templates["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["kind"] == row.kind)
        .cloned()
        .ok_or_else(unavailable)?;
    let current = controller(state, "", None).await?["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"] == key)
        .cloned();
    let installed = if let Some(current) = current {
        if matches!(current["phase"].as_str(), Some("creating" | "recreating")) {
            controller(
                state,
                &format!("/{key}/action"),
                Some(json!({"action":"reconcile"})),
            )
            .await?
        } else {
            if current["phase"] != "active" || current["drift"] == true {
                return Err(ApiError::conflict(
                    "Interrupted installation requires controller reconciliation",
                ));
            }
            current
        }
    } else {
        if row.state != "queued" {
            return Err(ApiError::conflict(
                "Interrupted Docker submission requires review before retry",
            ));
        }
        progress(state, key, "installing", None, None).await?;
        controller(state,"/install",Some(json!({"operation_id":key,"kind":row.kind,"host_port":row.host_port,"username":"thelxinoe","secret":credentials.secret}))).await?
    };
    let container = installed["container_id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    progress(state, key, "connecting", Some(container.clone()), None).await?;
    if row.kind == "recyclarr" {
        // A fresh job definition has no API; its schedule runs only after completion.
        super::recyclarr::installed(state, key, &row.actor).await?;
        return Ok(());
    }
    let service_name = match &row.service {
        Some(integration) => {
            storage::provision_read_manager_services(integration.clone(), &state.db).await?
        }
        None => format!("Managed {}", row.kind),
    };
    let port = template["port"].as_u64().ok_or_else(unavailable)? as u16;
    let mut last = None;
    let mut registered = None;
    for _ in 0..60 {
        // Thelxinoe generated or preserved these credentials and started the container itself.
        let registration = if matches!(row.kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            super::register_with_actor(
                state.clone(),
                super::Register {
                    name: service_name.clone(),
                    kind: row.kind.clone(),
                    container_id: container.clone(),
                    port,
                    api_key: credentials.secret.clone(),
                    url_base: thelxinoe_core::service_url_base(&row.kind).into(),
                    allow_unverified: false,
                    container_name: String::new(),
                    setup: Some(key.to_owned()),
                },
                row.actor.clone(),
            )
            .await
        } else {
            support::provision(
                state.clone(),
                row.actor.clone(),
                key,
                json!({"name":service_name,"kind":row.kind,"container_id":container,"port":port,
                    "credentials":{"username":credentials.username,"secret":credentials.secret},
                    "native_url":row.native_url,"url_base":thelxinoe_core::service_url_base(&row.kind)}),
            )
            .await
        };
        match registration {
            Ok(Json(value)) => {
                registered = Some(value);
                break;
            }
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let registered = registered.ok_or_else(|| last.unwrap_or_else(unavailable))?;
    let integration = registered["id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    storage::provision_write_stack_provisions(registered, key.to_owned(), &state.db).await?;
    let authority = ownership::Authority {
        service: integration.clone(),
        scope: ownership::Scope::Setup {
            provision: key.to_owned(),
        },
    };
    let origin = row.origin.clone();
    let kind = row.kind.clone();
    ownership::with_authority(authority, async move {
        if origin == "installed" && matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            let _guard = state.managers.guard.service(&kind).await;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(90);
            loop {
                let setup = async {
                    super::prepare_library(state, &super::service(state, &integration).await?)
                        .await?;
                    super::quality::ensure_defaults(
                        state,
                        &super::service(state, &integration).await?,
                    )
                    .await
                }
                .await;
                match setup {
                    Ok(()) => break,
                    Err(error)
                        if error.1 == "dependency_unavailable"
                            && tokio::time::Instant::now() < deadline =>
                    {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if kind == "seerr" && origin == "installed" {
            super::seerr::initialize(state, &integration).await?;
        } else if kind == "prowlarr" {
            super::indexers::ensure_hosts(state, &integration).await?;
        }
        // Required check: authenticated API readiness of this exact container.
        if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            let s = super::service(state, &integration).await?;
            access::verify_endpoint(&Connection::open(state, &s).await?, None).await?;
        } else {
            let s = support::load(state, &integration).await?;
            access::verify_endpoint(&support::connect(state, &s).await?, Some(&s.credentials))
                .await?;
        }
        Ok(())
    })
    .await
}

async fn wire(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Err(ApiError::conflict(
        "Compatible services connect automatically; manage individual connections in service settings",
    ))
}

async fn releases(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(controller(&state, "/releases", None).await?))
}

async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&key).is_err() {
        return Err(ApiError::bad("Invalid provision identity"));
    }
    let observed = controller(&state, "", None).await?;
    if let Some(service) = observed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"] == key)
        && (!matches!(
            service["phase"].as_str(),
            Some("active" | "creating" | "recreating")
        ) || service["drift"] == true)
    {
        return Err(ApiError::conflict(
            "Reconcile the controller operation before retrying API connection",
        ));
    }
    let changed = storage::retry(&state.db, key).await?;
    if !changed {
        return Err(ApiError::conflict(
            "Only a finished, blocked installation can be retried",
        ));
    }
    Ok(Json(json!({"queued":true})))
}

pub(super) async fn service_kind(state: &AppState, key: String) -> anyhow::Result<Option<String>> {
    storage::kind(&state.db, key).await
}
