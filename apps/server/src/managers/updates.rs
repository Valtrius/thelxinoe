//! Administrator policy and durable orchestration; Docker journal remains authoritative.
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
    if (key != "default" && uuid::Uuid::parse_str(&key).is_err())
        || !["automatic", "notify", "manual", "inherit"].contains(&input.policy.as_str())
        || (key == "default" && input.policy == "inherit")
        || input.window_start > 23
        || input.window_end > 23
    {
        return Err(ApiError::bad("Invalid update policy or maintenance window"));
    }
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;tx.execute("INSERT INTO service_update_policy(service_id,policy,window_start,window_end) VALUES (?1,?2,?3,?4) ON CONFLICT(service_id) DO UPDATE SET policy=excluded.policy,window_start=excluded.window_start,window_end=excluded.window_end",params![key,input.policy,input.window_start,input.window_end])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.update.policy',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(())}).await?;
    Ok(Json(json!({"saved":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut value=state.db.call(|db|{
        let policies=db.prepare("SELECT service_id,policy,window_start,window_end,candidate,error,checked_at FROM service_update_policy")?.query_map([],|r|Ok(json!({"service_id":r.get::<_,String>(0)?,"policy":r.get::<_,String>(1)?,"window_start":r.get::<_,u8>(2)?,"window_end":r.get::<_,u8>(3)?,"candidate":r.get::<_,Option<String>>(4)?,"error":r.get::<_,Option<String>>(5)?,"checked_at":r.get::<_,i64>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=db.prepare("SELECT id,service_id,state,candidate,error,created_at FROM service_updates ORDER BY created_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"service_id":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"candidate":r.get::<_,Option<String>>(3)?,"error":r.get::<_,Option<String>>(4)?,"created_at":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let services=db.prepare("SELECT id,kind FROM stack_provisions WHERE state='complete'")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"policies":policies,"items":items,"services":services,"timezone":crate::timezones::server_zone(db)?.name()}))
    }).await?;
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
    let inserted=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let available=tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND state='complete' AND service_id IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM service_updates WHERE service_id=?1 AND state IN ('queued','submitting','preparing','snapshotting','preflight','queued-activate','activating','recovery-snapshot','isolated-live-validation','recovery-required','queued-recover'))",[&service],|r|r.get::<_,bool>(0))?;
        if !available{return Ok(false);}
        tx.execute("INSERT INTO service_updates(id,service_id,actor_id,state,created_at,updated_at,automatic) VALUES (?1,?2,?3,'queued',?4,?4,?5)",params![key,service,actor,now(),actor.is_none()])?;
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.update',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key,"action":"preflight"}).to_string(),format!("update:{key}:preflight"),now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.update.preflight',?2,?3)",params![actor,key,now()])?;
        tx.commit()?;Ok(true)
    }).await?;
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
    let changed=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT state FROM service_updates WHERE id=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        let allowed=if action=="activate" {current.as_deref()==Some("ready")} else {current.as_deref().is_some_and(|s|["blocked","recovery-required"].contains(&s))};
        if !allowed{return Ok(false);}
        tx.execute("UPDATE service_updates SET state=?1,error=NULL,actor_id=?2,updated_at=?3,automatic=?5 WHERE id=?4",params![format!("queued-{action}"),actor,now(),key,actor.is_none()])?;
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.update',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key,"action":action}).to_string(),format!("update:{key}:{action}:{}",id()),now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor,format!("service.update.{action}"),key,now()])?;
        tx.commit()?;Ok(true)
    }).await?;
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
    state.db.call(move|db|{db.execute("UPDATE service_updates SET state=?1,candidate=COALESCE(?2,candidate),error=?3,updated_at=?4 WHERE id=?5",params![stage,candidate,error,now(),key])?;Ok(())}).await?;
    Ok(())
}
async fn idle(state: &AppState, provision: &str) -> Result<()> {
    let provision = provision.to_owned();
    let (kind, service) = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT kind,service_id FROM stack_provisions WHERE id=?1 AND state='complete'",
                [provision],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?)
        })
        .await?;
    let active=state.db.call(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1)",[now()-120],|r|r.get::<_,bool>(0))?)).await?;
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
    let automatic=state.db.call(move|db|Ok(db.query_row("SELECT u.automatic,u.service_id,COALESCE(NULLIF(p.policy,'inherit'),d.policy),CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_start ELSE p.window_start END,CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_end ELSE p.window_end END,u.state FROM service_updates u JOIN service_update_policy d ON d.service_id='default' LEFT JOIN service_update_policy p ON p.service_id=u.service_id WHERE u.id=?1",[lookup],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?,r.get::<_,u32>(4)?,r.get::<_,String>(5)?)))?)).await?;
    if automatic.0
        && ["queued", "queued-activate"].contains(&automatic.5.as_str())
        && action != "recover"
    {
        let wait = automatic.2 != "automatic"
            || !crate::timezones::in_server_window(state, automatic.3, automatic.4).await?
            || idle(state, &automatic.1).await.is_err();
        if wait {
            let job_id = job.id.clone();
            state.db.call(move|db|{db.execute("UPDATE jobs SET state='queued',available_at=?1,started_at=NULL WHERE id=?2",params![now()+60,job_id])?;Ok(())}).await?;
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
    let (service,stage,authorized)=state.db.call(move|db|Ok(db.query_row("SELECT service_id,state,automatic=1 OR EXISTS(SELECT 1 FROM users WHERE id=actor_id AND role='admin') FROM service_updates WHERE id=?1",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?)).await?;
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
    state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let (kind, service) = tx.query_row(
                "SELECT kind,service_id FROM stack_provisions WHERE id=?1",
                [&provision],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            let table = if ["radarr", "sonarr", "lidarr"].contains(&kind.as_str()) {
                "manager_services"
            } else {
                "support_services"
            };
            tx.execute(
                &format!("UPDATE {table} SET container_id=?1 WHERE id=?2"),
                params![container, service],
            )?;
            tx.execute(
                "UPDATE stack_provisions SET container_id=?1 WHERE id=?2",
                params![container, provision],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}
async fn check_releases(state: &AppState, force: bool) -> Result<()> {
    let rows=state.db.call(|db|Ok(db.prepare("SELECT s.id,s.kind,COALESCE(NULLIF(p.policy,'inherit'),d.policy),CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_start ELSE p.window_start END,CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_end ELSE p.window_end END,COALESCE(p.checked_at,0) FROM stack_provisions s JOIN service_update_policy d ON d.service_id='default' LEFT JOIN service_update_policy p ON p.service_id=s.id WHERE s.state='complete'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?,r.get::<_,u32>(4)?,r.get::<_,i64>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
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
        state.db.call(move|db|{db.execute("INSERT INTO service_update_policy(service_id,policy,window_start,window_end,checked_at,candidate,error) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(service_id) DO UPDATE SET checked_at=excluded.checked_at,candidate=excluded.candidate,error=excluded.error",params![key,mode,start,end,now(),found,if found.is_none(){Some("Stable release discovery unavailable")}else{None}])?;Ok(())}).await?;
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
        let ready=state.db.call(|db|Ok(db.prepare("SELECT u.id,CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_start ELSE p.window_start END,CASE WHEN p.policy IS NULL OR p.policy='inherit' THEN d.window_end ELSE p.window_end END FROM service_updates u JOIN service_update_policy d ON d.service_id='default' LEFT JOIN service_update_policy p ON p.service_id=u.service_id WHERE u.state='ready' AND COALESCE(NULLIF(p.policy,'inherit'),d.policy)='automatic'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,u32>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
        for (key, start, end) in ready {
            if crate::timezones::in_server_window(&state, start, end).await? {
                let _ = queue_action(&state, key, "activate".into(), None).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
