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
            let message = response.text().await.unwrap_or_default();
            return Err(ApiError::conflict(
                if status == reqwest::StatusCode::CONFLICT
                    || status == reqwest::StatusCode::BAD_REQUEST
                {
                    message.chars().take(300).collect::<String>()
                } else {
                    "Docker controller operation unavailable".into()
                },
            ));
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
        serde_json::from_str::<support::Credentials>(&raw).map_err(|_| unavailable())?
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
    host_port: u16,
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
        || input.host_port < 1024
        || !matches!(
            input.kind.as_str(),
            "radarr" | "sonarr" | "lidarr" | "bazarr" | "prowlarr" | "nzbget"
        )
    {
        return Err(ApiError::bad("Choose a supported service and local port"));
    }
    let key = id();
    let returned = key.clone();
    let credential = state.secrets.encrypt(
        &format!("provision:{key}"),
        id().replace('-', "").as_bytes(),
    )?;
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
            "start" | "stop" | "restart" | "reconcile" | "recreate" | "retire" | "remove"
        )
    {
        return Err(ApiError::bad("Invalid managed service action"));
    }
    let kind = storage::kind(&state.db, key.clone())
        .await?
        .ok_or_else(|| ApiError::conflict("This saved installation has no matching server record. Restore the matching server profile before managing it."))?;
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
    if matches!(input.action.as_str(), "stop" | "restart") && !confirmed_stopped(&container)? {
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
    let secret_input =
        String::from_utf8(state.secrets.decrypt(&format!("provision:{key}"), &row.3)?)?;
    let credentials: support::Credentials =
        serde_json::from_str(&secret_input).unwrap_or(support::Credentials {
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
       if matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr"){operations::ensure_idle(state,row.6.as_deref().ok_or_else(unavailable)?).await?;}
       controller(state,"/adopt",Some(json!({"operation_id":key,"kind":row.0,"container_id":row.5,"released_compose":true}))).await?
   } else {controller(state,"/install",Some(json!({"operation_id":key,"kind":row.0,"host_port":row.2,"username":"thelxinoe","secret":secret}))).await?}
  };
  let container=installed["container_id"].as_str().ok_or_else(unavailable)?.to_owned();progress(state,&key,"connecting",Some(container.clone()),None).await?;
  if row.7=="adopted" {let service_id=row.6.clone().ok_or_else(unavailable)?;let container=container.clone();let kind=row.0.clone();storage::provision_write(service_id, container, kind, &state.db).await?;}
  let mut last=None;let mut registered=None;
   let service_name=if let Some(integration)=row.6.clone() {storage::provision_read_manager_services(integration, &state.db).await?}else{format!("Managed {}",row.0)};
  for _ in 0..60 {
   let registration=if matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr") {super::register_with_actor(state.clone(),super::Register{name:service_name.clone(),kind:row.0.clone(),container_id:container.clone(),port:template["port"].as_u64().ok_or_else(unavailable)? as u16,api_key:secret.clone()},row.1.clone()).await}
   else {support::provision(state.clone(),row.1.clone(),json!({"name":service_name,"kind":row.0,"container_id":container,"port":template["port"],"credentials":{"username":credentials.username,"secret":secret},"native_url":row.8})).await};
   match registration {Ok(Json(value))=>{registered=Some(value);break;},Err(error)=>last=Some(error)};
   tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  }
  let registered=registered.ok_or_else(||last.unwrap_or_else(unavailable))?;
  let integration=registered["id"].as_str().ok_or_else(unavailable)?.to_owned();
  let key=key.clone();storage::provision_write_stack_provisions(registered, key, &state.db).await?;
  if row.7=="installed" && matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr") {
    let _guard=state.managers.guard.service(&row.0).await;
    super::prepare_library(state,&super::service(state,&integration).await?).await?;
  }
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
    state.emit(None, "stack.changed", json!({"id":key})).await?;
    Ok(())
}

async fn wire(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Err(ApiError::conflict(
        "Choose Connect for each optional connection in the service settings",
    ))
}

#[derive(Deserialize)]
struct AdoptPreview {
    service_id: String,
}
async fn adopt_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AdoptPreview>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let key = input.service_id.clone();
    let (kind, container, port) = storage::adopt_preview(&state.db, key)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let templates = controller(&state, "/templates", None).await?;
    if !templates["items"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|t| t["kind"] == kind && t["port"] == port)
    {
        return Err(ApiError::conflict(
            "Restore the service's standard HTTP port before transferring ownership",
        ));
    }
    if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
        let s = service(&state, &input.service_id).await?;
        let status = Connection::open(&state, &s)
            .await?
            .get("system/status")
            .await?;
        if status["branch"]
            .as_str()
            .is_some_and(|branch| !matches!(branch, "master" | "main" | "stable"))
        {
            return Err(ApiError::conflict(
                "Only stable service releases can become managed services",
            ));
        }
    }
    Ok(Json(
        controller(
            &state,
            "/adopt/preview",
            Some(json!({"kind":kind,"container_id":container})),
        )
        .await?,
    ))
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
    storage::restore_original_write_jobs(&state.db, key, actor, kind, integration, container)
        .await?;
    state
        .emit(None, "stack.changed", json!({"restored":true}))
        .await?;
    Ok(Json(result))
}
#[derive(Deserialize)]
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
    controller(&state,"/adopt/check",Some(json!({"operation_id":input.review_id,"kind":item.0,"container_id":item.1,"released_compose":input.released_compose}))).await?;
    let key = input.review_id.clone();
    let returned = key.clone();
    let credential = state.secrets.encrypt(
        &format!("provision:{key}"),
        &serde_json::to_vec(&credentials).map_err(|_| unavailable())?,
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
