//! Server-owned provisioning jobs keep credentials out of job payloads and controller responses.
use super::*;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/stack", get(list))
        .route("/api/v1/admin/stack/templates", get(templates))
        .route("/api/v1/admin/stack/releases", get(releases))
        .route("/api/v1/admin/stack/install", post(install))
        .route("/api/v1/admin/stack/adopt", post(adopt))
        .route("/api/v1/admin/stack/wire", post(wire))
        .route("/api/v1/admin/stack/{id}/action", post(action))
        .route("/api/v1/admin/stack/{id}/retry", post(retry))
}
async fn controller(state: &AppState, path: &str, body: Option<Value>) -> Result<Value> {
    #[cfg(unix)]
    {
        let client = reqwest::Client::builder()
            .unix_socket(state.config.controller_socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(240))
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
    value["provisions"]=state.db.call(|db|Ok(json!(db.prepare("SELECT id,kind,state,host_port,container_id,service_id,error,native_url FROM stack_provisions ORDER BY created_at")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"host_port":r.get::<_,u16>(3)?,"container_id":r.get::<_,Option<String>>(4)?,"service_id":r.get::<_,Option<String>>(5)?,"error":r.get::<_,Option<String>>(6)?,"native_url":r.get::<_,String>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?))).await?;
    Ok(Json(value))
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
    let inserted=state.db.call(move|db|{let tx=db.transaction()?;if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1)",[&input.kind],|r|r.get::<_,bool>(0))?{return Ok(false);}tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at,native_url) VALUES (?1,?2,?3,?4,?5,'queued',?6,?6,?7)",params![key,input.kind,p.user.id,input.host_port,credential,now(),input.native_url])?;tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.install',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(true)}).await?;
    if !inserted {
        return Err(ApiError::conflict(
            "This service already has a provisioning record",
        ));
    }
    Ok(Json(json!({"id":returned,"state":"queued"})))
}
#[derive(Deserialize)]
struct Action {
    action: String,
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
            "start" | "stop" | "restart" | "reconcile"
        )
    {
        return Err(ApiError::bad("Invalid managed service action"));
    }
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    // Media managers can be stopped only after their activity is inspected.
    let container = controller(&state, "", None).await?["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"] == key)
        .cloned()
        .ok_or_else(ApiError::not_found)?;
    if !matches!(input.action.as_str(), "start" | "reconcile") {
        let c = container["container_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let service = state
            .db
            .call(move |db| {
                Ok(db
                    .query_row(
                        "SELECT id FROM manager_services WHERE container_id=?1",
                        [c],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?)
            })
            .await?;
        if let Some(s) = service {
            operations::ensure_idle(&state, &s).await?;
        }
    }
    let result = controller(
        &state,
        &format!("/{key}/action"),
        Some(json!({"action":input.action})),
    )
    .await?;
    if input.action == "reconcile" {
        let key = key.clone();
        state.db.call(move|db|{let tx=db.transaction()?;
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND state='blocked')",[&key],|r|r.get::<_,bool>(0))? {
                tx.execute("UPDATE stack_provisions SET state='connecting',error=NULL WHERE id=?1",[&key])?;
                tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1 WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2 AND state IN ('failed','complete')",params![now(),key])?;
            }tx.commit()?;Ok(())}).await?;
    }
    state
        .db
        .call(move |db| {
            db.execute(
                "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
                params![p.user.id, format!("stack.{}", input.action), key, now()],
            )?;
            Ok(())
        })
        .await?;
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
    state.db.call(move|db|{db.execute("UPDATE stack_provisions SET state=?1,container_id=COALESCE(?2,container_id),error=?3,updated_at=?4 WHERE id=?5",params![stage,container,error,now(),key])?;Ok(())}).await?;
    Ok(())
}
pub(crate) async fn provision(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let key = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing provision identity"))?
        .to_owned();
    let lookup = key.clone();
    let row=state.db.call(move|db|Ok(db.query_row("SELECT kind,actor_id,host_port,credential,state,container_id,service_id,origin,native_url FROM stack_provisions WHERE id=?1",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u16>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?)))?)).await?;
    if row.4 == "complete" {
        return Ok(());
    }
    let actor = row.1.clone();
    let admin = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND role='admin')",
                [actor],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
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
  let installed=if let Some(current)=current{if current["phase"]!="active"||current["drift"]==true{return Err(ApiError::conflict("Interrupted installation requires controller reconciliation"));}current}else{
   if row.4!="queued"{return Err(ApiError::conflict("Interrupted Docker submission requires review before retry"));}
   progress(state,&key,"installing",None,None).await?;
   if row.7=="adopted" {
       let _lease=state.media_operations.write().await;let _guard=state.managers.guard.lock().await;
       if matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr"){operations::ensure_idle(state,row.6.as_deref().ok_or_else(unavailable)?).await?;}
       controller(state,"/adopt",Some(json!({"operation_id":key,"kind":row.0,"container_id":row.5}))).await?
   } else {controller(state,"/install",Some(json!({"operation_id":key,"kind":row.0,"host_port":row.2,"username":"thelxinoe","secret":secret}))).await?}
  };
  let container=installed["container_id"].as_str().ok_or_else(unavailable)?.to_owned();progress(state,&key,"connecting",Some(container.clone()),None).await?;
  if row.7=="adopted" {let service_id=row.6.clone().ok_or_else(unavailable)?;let container=container.clone();let kind=row.0.clone();state.db.call(move|db|{let table=if matches!(kind.as_str(),"radarr"|"sonarr"|"lidarr"){"manager_services"}else{"support_services"};db.execute(&format!("UPDATE {table} SET container_id=?1,generation=?2 WHERE id=?3"),params![container,id(),service_id])?;Ok(())}).await?;}
  let mut last=None;let mut registered=None;
  for _ in 0..60 {
   let registration=if matches!(row.0.as_str(),"radarr"|"sonarr"|"lidarr") {super::register_with_actor(state.clone(),super::Register{name:format!("Managed {}",row.0),kind:row.0.clone(),container_id:container.clone(),port:template["port"].as_u64().ok_or_else(unavailable)? as u16,api_key:secret.clone()},row.1.clone()).await}
   else {support::provision(state.clone(),row.1.clone(),json!({"name":format!("Managed {}",row.0),"kind":row.0,"container_id":container,"port":template["port"],"credentials":{"username":credentials.username,"secret":secret},"native_url":row.8})).await};
   match registration {Ok(Json(value))=>{registered=Some(value);break;},Err(error)=>last=Some(error)};
   tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  }
  let registered=registered.ok_or_else(||last.unwrap_or_else(unavailable))?;let key=key.clone();state.db.call(move|db|{db.execute("UPDATE stack_provisions SET service_id=?1,state='connecting',error=NULL,updated_at=?2 WHERE id=?3",params![registered["id"].as_str(),now(),key])?;Ok(())}).await?;
  Ok::<(),ApiError>(())
 }.await;
    if let Err(error) = result {
        progress(state, &key, "blocked", None, Some(error.2.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.2))?;
        anyhow::bail!("{}", error.2);
    }
    if row.7 == "installed" {
        if let Err(error) = support::wire_managed(state).await {
            progress(state, &key, "blocked", None, Some(error.2.clone()))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.2))?;
            anyhow::bail!("{}", error.2);
        }
    } else {
        if let Err(error) = controller(
            state,
            &format!("/{key}/action"),
            Some(json!({"action":"retire_original"})),
        )
        .await
        {
            progress(state, &key, "blocked", None, Some(error.2.clone()))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.2))?;
            anyhow::bail!("{}", error.2);
        }
    }
    progress(state, &key, "complete", None, None)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.2))?;
    state.emit(None, "stack.changed", json!({"id":key})).await?;
    Ok(())
}

async fn wire(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    support::wire_managed(&state).await?;
    Ok(Json(json!({"connected":true})))
}

#[derive(Deserialize)]
struct Adopt {
    service_id: String,
}
async fn adopt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let service_id = input.service_id.clone();
    let item=state.db.call(move|db|Ok(db.query_row("SELECT kind,container_id,'' FROM manager_services WHERE id=?1 UNION ALL SELECT kind,container_id,native_url FROM support_services WHERE id=?1",[service_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?)).await?.ok_or_else(ApiError::not_found)?;
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
    let evidence = docker(&state, &format!("containers/{}", item.1)).await?;
    if evidence["compose_project"]
        .as_str()
        .is_some_and(|v| !v.is_empty())
    {
        return Err(ApiError::conflict(
            "Remove the existing Compose owner before adopting this container",
        ));
    }
    let key = id();
    let returned = key.clone();
    let credential = state.secrets.encrypt(
        &format!("provision:{key}"),
        &serde_json::to_vec(&credentials).map_err(|_| unavailable())?,
    )?;
    state.db.call(move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,origin,created_at,updated_at,native_url) VALUES (?1,?2,?3,0,?4,'queued',?5,?6,'adopted',?7,?7,?8)",params![key,item.0,p.user.id,credential,item.1,input.service_id,now(),item.2])?;tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.adopt',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(())}).await?;
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
        && (service["phase"] != "active" || service["drift"] == true)
    {
        return Err(ApiError::conflict(
            "Reconcile the controller operation before retrying API connection",
        ));
    }
    let changed=state.db.call(move|db|{let tx=db.transaction()?;
        let ready=tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions p JOIN jobs j ON json_extract(j.payload,'$.id')=p.id WHERE p.id=?1 AND p.state='blocked' AND j.kind='stack.install' AND j.state='failed')",[&key],|r|r.get::<_,bool>(0))?;
        if !ready {return Ok(false);}
        tx.execute("UPDATE stack_provisions SET state='queued',error=NULL,updated_at=?1 WHERE id=?2",params![now(),key])?;
        tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1 WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2",params![now(),key])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !changed {
        return Err(ApiError::conflict(
            "Only a finished, blocked provisioning attempt can be retried",
        ));
    }
    Ok(Json(json!({"queued":true})))
}
