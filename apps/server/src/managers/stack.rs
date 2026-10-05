//! Server-owned provisioning jobs keep credentials out of job payloads and controller responses.

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
        .route("/api/v1/admin/stack/adopt", post(adopt))
        .route("/api/v1/admin/stack/adopt/preview", post(adopt_preview))
        .route(
            "/api/v1/admin/stack/{id}/restore-original",
            post(restore_original),
        )
        .route("/api/v1/admin/stack/wire", post(wire))
        .route("/api/v1/admin/stack/{id}/action", post(action))
        .route("/api/v1/admin/stack/{id}/retry", post(retry))
}
pub(crate) async fn controller(state: &AppState, path: &str, body: Option<Value>) -> Result<Value> {
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
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            let status = response.status();
            tracing::warn!(
                path,
                status = status.as_u16(),
                "Docker controller request failed"
            );
            let message = response.text().await.unwrap_or_default();
            let code = if status.is_server_error() {
                "dependency_unavailable"
            } else if matches!(
                status,
                reqwest::StatusCode::LOCKED | reqwest::StatusCode::TOO_MANY_REQUESTS
            ) {
                "operation_contended"
            } else if status == reqwest::StatusCode::PRECONDITION_FAILED {
                "revision_changed"
            } else {
                "conflict"
            };
            let message = if status.is_client_error() {
                message.chars().take(300).collect()
            } else {
                "Docker controller operation unavailable".into()
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
        let registered = provisions
            .iter()
            .any(|provision| provision["id"] == item["id"] && provision["kind"] == item["kind"]);
        item["registered"] = json!(registered);
        if !registered {
            item["can_recreate"] = json!(false);
            item["can_retire"] = json!(false);
            item["can_remove"] = json!(false);
        }
    }
    Ok(Json(value))
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (kind, encrypted, origin) = storage::login(key.clone(), &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if kind != "nzbget" {
        return Err(ApiError::not_found());
    }
    let raw = String::from_utf8(
        state
            .secrets
            .decrypt(&format!("provision:{key}"), &encrypted)?,
    )
    .map_err(|_| unavailable())?;
    let credentials = if origin == "adopted" {
        serde_json::from_str::<ProvisionCredentials>(&raw)
            .map_err(|_| unavailable())?
            .credentials
    } else {
        support::Credentials {
            username: "thelxinoe".into(),
            secret: raw,
        }
    };
    Ok(Json(json!({
        "username": credentials.username,
        "password": credentials.secret,
    })))
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
            "This service is already connected or managed",
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
            "start"
                | "stop"
                | "restart"
                | "reconcile"
                | "recreate"
                | "retire"
                | "remove"
                | "release"
        )
    {
        return Err(ApiError::bad("Invalid managed service action"));
    }
    if input.action == "release" {
        if storage::released(&state.db, key.clone()).await? {
            return Ok(Json(json!({"accepted":true,"released":true})));
        }
        if !storage::imported(&state.db, key.clone()).await? {
            return Err(ApiError::conflict(
                "Release ownership applies to imported deployments",
            ));
        }
    }
    let kind = storage::kind(&state.db, key.clone())
        .await?
        .ok_or_else(|| ApiError::conflict("This saved installation has no matching server record. Restore the matching server profile before managing it."))?;
    let _lease = state.managers.maintenance(&state).await;
    let _guard = state.managers.guard.service(&kind).await;
    if input.action == "release" {
        if !storage::begin_retirement(&state.db, key.clone()).await? {
            return Err(ApiError::conflict(
                "Finish the current setup or update before releasing ownership",
            ));
        }
        let result = controller(
            &state,
            &format!("/{key}/action"),
            Some(json!({"action":"release"})),
        )
        .await?;
        if result["released"] != true {
            return Err(ApiError::conflict("The controller did not confirm release"));
        }
        storage::release(&state.db, key.clone(), p.user.id).await?;
        state.emit(None, "stack.changed", json!({"id":key})).await?;
        return Ok(Json(result));
    }
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
        if !storage::begin_retirement(&state.db, key.clone()).await? {
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
        if !storage::begin_retirement(&state.db, key.clone()).await? {
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
            .ok_or_else(|| {
                ApiError::conflict("Connect the service API before checking its activity")
            })?;
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

pub(crate) async fn provision(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let key = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing provision identity"))?
        .to_owned();
    let lookup = key.clone();
    let row = storage::provision_read_stack_provisions(lookup, &state.db).await?;
    if row.4 == "complete" {
        return Ok(());
    }
    if row.4 == "retiring" {
        anyhow::bail!("Installation is being retired");
    }
    let actor = row.1.clone();
    let admin = storage::provision_read_users(actor, &state.db).await?;
    if !admin {
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
    if row.7 == "adopted" && row.0 != "recyclarr" {
        let _lease = state.managers.maintenance(state).await;
        let _guard = state.managers.guard.service(&row.0).await;
        let registered = async {
            let result = controller(state, "/adopt", Some(json!({"operation_id":key,"kind":row.0,"container_id":row.5,"released_compose":job.payload["released_compose"] == true}))).await?;
            let container = result["container_id"].as_str().ok_or_else(unavailable)?.to_owned();
            storage::complete_adoption(&state.db, key.clone(), container).await?;
            Ok::<_, ApiError>(())
        }.await;
        if let Err(error) = registered {
            progress(state, &key, "blocked", None, Some(error.2.clone()))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.2))?;
            anyhow::bail!("{}", error.2);
        }
        state.emit(None, "stack.changed", json!({"id":key})).await?;
        return Ok(());
    }
    let secret_input = if let Some(encrypted) = &row.3 {
        String::from_utf8(
            state
                .secrets
                .decrypt(&format!("provision:{key}"), encrypted)?,
        )?
    } else {
        String::new()
    };
    let credentials = serde_json::from_str::<ProvisionCredentials>(&secret_input)
        .map(|stored| stored.credentials)
        .unwrap_or(support::Credentials {
            username: "thelxinoe".into(),
            secret: secret_input,
        });
    let secret = credentials.secret;
    let result=async {
  let templates=controller(state,"/templates",None).await?;let template=templates["items"].as_array().into_iter().flatten().find(|t|t["kind"]==row.0).cloned().ok_or_else(unavailable)?;
  let current=controller(state,"",None).await?["items"].as_array().into_iter().flatten().find(|s|s["id"]==key).cloned();
   let installed=if let Some(current)=current{
    if row.7=="installed" && matches!(current["phase"].as_str(),Some("creating"|"recreating")) {
        controller(state,&format!("/{key}/action"),Some(json!({"action":"reconcile"}))).await?
    } else {
        if (current["phase"]!="active" && !(row.7=="adopted" && current["phase"]=="connecting"))||current["drift"]==true{return Err(ApiError::conflict("Interrupted installation requires controller reconciliation"));}current
    }
   }else{
   if row.4!="queued"{return Err(ApiError::conflict("Interrupted Docker submission requires review before retry"));}
   progress(state,&key,"installing",None,None).await?;
   if row.7=="adopted" {
       let _lease=state.managers.maintenance(state).await;let _guard=state.managers.guard.service(&row.0).await;
       let request=json!({"operation_id":key,"kind":row.0,"container_id":row.5,"released_compose":true});
       controller(state,"/adopt",Some(request)).await?
   } else {controller(state,"/install",Some(json!({"operation_id":key,"kind":row.0,"host_port":row.2,"username":"thelxinoe","secret":secret}))).await?}
  };
  let container=installed["container_id"].as_str().ok_or_else(unavailable)?.to_owned();progress(state,&key,"connecting",Some(container.clone()),None).await?;
  if row.0 == "recyclarr" {
    super::recyclarr::installed(state, &key, &row.1, row.7=="adopted").await?;
    return Ok(());
  }
  if row.7=="adopted" {let service_id=row.6.clone().ok_or_else(unavailable)?;let container=container.clone();let kind=row.0.clone();storage::provision_write(service_id, container, kind, &state.db).await?;}
  let mut last=None;let mut registered=None;
   let service_name=if let Some(integration)=row.6.clone() {storage::provision_read_manager_services(integration, &state.db).await?}else{format!("Managed {}",row.0)};
  for _ in 0..60 {
   let registration=if matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr") {super::register_with_actor(state.clone(),super::Register{name:service_name.clone(),kind:row.0.clone(),container_id:container.clone(),port:template["port"].as_u64().ok_or_else(unavailable)? as u16,api_key:secret.clone(),url_base:thelxinoe_core::service_url_base(&row.0).into()},row.1.clone()).await}
   else {support::provision(state.clone(),row.1.clone(),json!({"name":service_name,"kind":row.0,"container_id":container,"port":template["port"],"credentials":{"username":credentials.username,"secret":secret},"native_url":row.8,"url_base":thelxinoe_core::service_url_base(&row.0)})).await};
   match registration {Ok(Json(value))=>{registered=Some(value);break;},Err(error)=>last=Some(error)};
   tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  }
  let registered=registered.ok_or_else(||last.unwrap_or_else(unavailable))?;
  let integration=registered["id"].as_str().ok_or_else(unavailable)?.to_owned();
  let key=key.clone();storage::provision_write_stack_provisions(registered, key, &state.db).await?;
  if row.7=="installed" && matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr") {
    let _guard=state.managers.guard.service(&row.0).await;
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(90);
    loop {
        let setup=async {
            super::prepare_library(state,&super::service(state,&integration).await?).await?;
            super::quality::ensure_defaults(state,&super::service(state,&integration).await?).await
        }.await;
        match setup {
            Ok(())=>break,
            Err(error) if error.2==unavailable().2 && tokio::time::Instant::now()<deadline=> {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error)=>return Err(error),
        }
    }
  }
  if row.0=="seerr" && row.7=="installed" {super::seerr::initialize(state).await?;}
  else if row.0=="prowlarr" {super::indexers::ensure_hosts(state,&integration).await?;}
  else if matches!(row.0.as_str(),"radarr"|"sonarr") {super::seerr::sync_managers(state).await?;}
  Ok::<(),ApiError>(())
 }.await;
    if let Err(error) = result {
        progress(state, &key, "blocked", None, Some(error.2.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        anyhow::bail!("{}", error.2);
    }
    if row.7 == "adopted"
        && let Err(error) = controller(
            state,
            &format!("/{key}/action"),
            Some(json!({"action":"complete_adoption"})),
        )
        .await
    {
        progress(state, &key, "blocked", None, Some(error.2.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        anyhow::bail!("{}", error.2);
    }
    progress(state, &key, "complete", None, None)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.2))?;
    state.managers.connection_wake.notify_one();
    state.emit(None, "stack.changed", json!({"id":key})).await?;
    Ok(())
}

async fn wire(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Err(ApiError::conflict(
        "Compatible services connect automatically; manage individual connections in service settings",
    ))
}

#[derive(Deserialize)]
struct AdoptPreview {
    service_id: String,
}

#[derive(Deserialize, Serialize)]
struct ProvisionCredentials {
    #[serde(flatten)]
    credentials: support::Credentials,
}

fn nzbget_value<'a>(config: &'a Value, key: &str) -> Result<&'a str> {
    let rows = config.as_array().ok_or_else(unavailable)?;
    let mut matching = rows.iter().filter(|row| {
        row["Name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(key))
    });
    let value = matching
        .next()
        .and_then(|row| row["Value"].as_str())
        .ok_or_else(unavailable)?;
    if matching.next().is_some() {
        return Err(ApiError::conflict("NZBGet returned duplicate settings"));
    }
    Ok(value)
}
async fn adopt_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AdoptPreview>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let key = input.service_id.clone();
    let (kind, container, _) = storage::adopt_preview(&state.db, key)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let mut review = controller(
        &state,
        "/adopt/preview",
        Some(json!({"kind":kind,"container_id":container})),
    )
    .await?;
    let observed = docker(&state, &format!("containers/{container}")).await?;
    let access = if observed["running"] == true {
        Some(adoption_access(&state, &input.service_id, &kind).await)
    } else {
        None
    };
    review["integration_ready"] = json!(access.as_ref().is_some_and(Result::is_ok));
    review["integration_error"] = json!(access.and_then(Result::err).map(|e| e.2));
    if kind == "nzbget"
        && observed["running"] == true
        && let Ok(service) = support::load(&state, &input.service_id).await
        && let Ok(c) = support::connect(&state, &service).await
        && let Ok(config) = support::rpc(&c, &service.credentials, "config", json!([])).await
    {
        let mut warnings = Vec::new();
        if nzbget_value(&config, "WriteLog").is_ok_and(|v| v.eq_ignore_ascii_case("append")) {
            warnings.push("The log file may grow indefinitely");
        }
        if nzbget_value(&config, "ControlPassword").is_ok_and(str::is_empty) {
            warnings.push("NZBGet authentication has an empty password");
        }
        if nzbget_value(&config, "CertCheck").is_ok_and(|v| !v.eq_ignore_ascii_case("yes")) {
            warnings.push("Outgoing TLS certificate verification is disabled");
        }
        review["warnings"] = json!(warnings);
    }
    Ok(Json(review))
}

async fn adoption_access(state: &AppState, key: &str, kind: &str) -> Result<()> {
    if ["radarr", "sonarr", "lidarr"].contains(&kind) {
        let service = service(state, key).await?;
        let c = Connection::open(state, &service).await?;
        access::check_connection(&c).await
    } else {
        let service = support::load(state, key).await?;
        let c = support::connect(state, &service).await?;
        if kind == "nzbget" {
            let version = support::rpc(&c, &service.credentials, "version", json!([])).await?;
            if !version.as_str().is_some_and(|v| !v.is_empty()) {
                return Err(unavailable());
            }
            Ok(())
        } else {
            access::check_connection(&c).await
        }
    }
}

async fn restore_original(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    if uuid::Uuid::parse_str(&key).is_err() {
        return Err(ApiError::bad("Invalid transfer identity"));
    }
    let kind = storage::kind(&state.db, key.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let _lease = state.managers.maintenance(&state).await;
    let _guard = state.managers.guard.service(&kind).await;
    let lookup = key.clone();
    let (kind, integration) = storage::restore_original_read_stack_provisions(&state.db, lookup)
        .await?
        .ok_or_else(|| {
            ApiError::conflict("Only a finished, blocked transfer can restore the original")
        })?;
    if kind != "recyclarr" {
        return Err(ApiError::conflict(
            "In-place adoption retains the original container; use Release ownership to cancel management",
        ));
    }
    let result = controller(
        &state,
        &format!("/{key}/action"),
        Some(json!({"action":"restore_original"})),
    )
    .await?;
    let container = result["container_id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    storage::restore_original_write_jobs(&state.db, key, actor, kind, integration, container, None)
        .await?;
    state
        .emit(None, "stack.changed", json!({"restored":true}))
        .await?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Adopt {
    service_id: String,
    review_id: String,
    #[serde(default)]
    released_compose: bool,
}
async fn adopt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let service_id = input.service_id.clone();
    let item = storage::adopt_read_manager_services(&state.db, service_id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let credentials = if matches!(item.0.as_str(), "radarr" | "sonarr" | "lidarr") {
        let service = service(&state, &input.service_id).await?;
        support::Credentials {
            username: String::new(),
            secret: String::from_utf8(
                state
                    .secrets
                    .decrypt(&format!("manager:{}", service.id), &service.credential)?,
            )
            .map_err(|_| unavailable())?,
        }
    } else {
        support::load(&state, &input.service_id).await?.credentials
    };
    if uuid::Uuid::parse_str(&input.review_id).is_err() {
        return Err(ApiError::bad(
            "Review this service before transferring ownership",
        ));
    }
    let stored = ProvisionCredentials { credentials };
    let request = json!({"operation_id":input.review_id,"kind":item.0,"container_id":item.1,"released_compose":input.released_compose});
    if storage::adoption_exists(&state.db, input.review_id.clone(), input.service_id.clone())
        .await?
    {
        return Ok(Json(json!({"id":input.review_id,"state":"queued"})));
    }
    controller(&state, "/adopt/check", Some(request)).await?;
    let key = input.review_id.clone();
    let returned = key.clone();
    let credential = state.secrets.encrypt(
        &format!("provision:{key}"),
        &serde_json::to_vec(&stored).map_err(|_| unavailable())?,
    )?;
    let inserted =
        storage::adopt_write_stack_provisions(&state.db, input, p, item, key, credential).await?;
    if !inserted {
        return Err(ApiError::conflict(
            "This service already has a provisioning record",
        ));
    }
    Ok(Json(json!({"id":returned,"state":"queued"})))
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
            Some("active" | "connecting" | "creating" | "recreating")
        ) || service["drift"] == true)
    {
        return Err(ApiError::conflict(
            "Reconcile the controller operation before retrying API connection",
        ));
    }
    let changed = storage::retry(&state.db, key).await?;
    if !changed {
        return Err(ApiError::conflict(
            "Only a finished, blocked provisioning attempt can be retried",
        ));
    }
    Ok(Json(json!({"queued":true})))
}

pub(super) async fn service_kind(state: &AppState, key: String) -> anyhow::Result<Option<String>> {
    storage::kind(&state.db, key).await
}
