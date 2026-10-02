use super::*;

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum OutcomeKind {
    Applied,
    Previewed,
    Partial,
    Blocked,
}
pub(super) struct ControllerOutcome {
    pub kind: OutcomeKind,
    pub evidence: Value,
}
impl ControllerOutcome {
    fn parse(evidence: Value) -> Result<Self> {
        let kind = serde_json::from_value(evidence["state"].clone())
            .map_err(|_| ApiError::conflict("Controller returned an unknown Recyclarr outcome"))?;
        Ok(Self { kind, evidence })
    }
}
enum RetryClass {
    Dependency,
    Contended,
    RevisionChanged,
    Blocked,
}
impl RetryClass {
    fn of(error: &ApiError) -> Self {
        match error.1 {
            "dependency_unavailable" | "database_unavailable" => Self::Dependency,
            "operation_contended" => Self::Contended,
            "revision_changed" => Self::RevisionChanged,
            _ => Self::Blocked,
        }
    }
}

pub(super) async fn queue(
    state: AppState,
    headers: HeaderMap,
    preview: bool,
    mut input: Value,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    if !preview && input.get("files").is_some() {
        return Err(ApiError::bad("Save draft files before syncing"));
    }
    let key = provision(&state).await?;
    let current = current_configuration(&state, &key).await?;
    if input.get("files").is_some() {
        if input["revision"].as_str() != current["revision"].as_str() {
            return Err(ApiError::conflict(
                "Supply the current configuration revision to preview a draft",
            ));
        }
        let inspected=controller_request(&state,&format!("/{key}/recyclarr/configuration"),Some(json!({"operation":"inspect","revision":current["revision"],"files":input["files"],"targets":configuration_context(&state,&key).await?}))).await?;
        if inspected["valid"] != true {
            return Err(ApiError::bad(
                "Invalid YAML file set; validate the draft to see its problems",
            ));
        }
        input["used_services"] = inspected["used_services"].clone();
    }
    if input["revision"]
        .as_str()
        .is_some_and(|revision| current["revision"] != revision)
    {
        return Err(ApiError::conflict(
            "Configuration revision changed; reload the latest files",
        ));
    }
    let id = storage::queue(&state.db, key, actor.user.id, preview, input).await?;
    Ok(Json(json!({"id":id,"state":"queued"})))
}
pub(super) async fn sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Option<Json<Value>>,
) -> Result<Json<Value>> {
    queue(
        state,
        headers,
        false,
        input.map_or_else(|| json!({}), |v| v.0),
    )
    .await
}
pub(super) async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    input: Option<Json<Value>>,
) -> Result<Json<Value>> {
    queue(
        state,
        headers,
        true,
        input.map_or_else(|| json!({}), |v| v.0),
    )
    .await
}
pub(super) async fn snapshot(c: &Connection<'_>) -> Result<Value> {
    Ok(
        json!({"profiles":c.get("qualityprofile").await?,"formats":c.get("customformat").await?,"sizes":c.get("qualitydefinition").await?,"naming":c.get("config/naming").await?,"management":c.get("config/mediamanagement").await?}),
    )
}
pub(in crate::managers) async fn capture_update(state: &AppState, key: &str) -> Result<()> {
    let configuration = current_configuration(state, key).await?;
    let targets = storage::targets(&state.db, key.into()).await?;
    let mut snapshots = serde_json::Map::new();
    for target in targets {
        if snapshots.contains_key(&target.service_id) {
            continue;
        }
        if configuration["mode"] == "customized"
            && !configuration["used_services"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|id| id == &target.service_id)
        {
            continue;
        }
        let service = service(state, &target.service_id).await?;
        snapshots.insert(
            service.id.clone(),
            snapshot(&Connection::open(state, &service).await?).await?,
        );
    }
    controller_request(state,&format!("/{key}/recyclarr/configuration"),Some(json!({"operation":"capture","revision":configuration["revision"],"snapshots":snapshots,"required_profiles":storage::required_profiles(&state.db).await?}))).await?;
    Ok(())
}
pub(crate) async fn run_job(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<bool> {
    let run = job.payload["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing Recyclarr run"))?;
    let (key, stage, _) = storage::run(&state.db, run.into()).await?;
    if matches!(
        stage.as_str(),
        "complete" | "previewed" | "blocked" | "partial" | "cancelled"
    ) {
        return Ok(true);
    }
    let _maintenance = state.managers.maintenance(state).await;
    let _services = state
        .managers
        .guard
        .services(&["recyclarr", "radarr", "sonarr"])
        .await;
    let (_, _, authorized) = storage::run(&state.db, run.into()).await?;
    if job.payload["automatic"] == true && storage::paused(&state.db, key.clone()).await? {
        storage::cancel(&state.db, run.into(), "Automatic sync paused".into()).await?;
        return Ok(true);
    }
    let mut targets = storage::targets(&state.db, key.clone()).await?;
    let result=async {
        if !authorized { return Err(ApiError::forbidden()); }
        if targets.is_empty() { return Err(ApiError::conflict("Connect Radarr or Sonarr before syncing")); }
        let configuration=current_configuration(state,&key).await?;
        let customized=configuration["mode"]=="customized";
        storage::begin(&state.db,run.into()).await?;
        let mut catalogs=std::collections::HashMap::new();
        let mut enrolled=std::collections::HashSet::new();
        for target in &targets {
            if !catalogs.contains_key(&target.kind) { catalogs.insert(target.kind.clone(),fetch_catalog(state,&key,&target.kind).await?); }
            if !customized && enrolled.insert(target.service_id.clone()) { storage::enroll(&state.db,key.clone(),target.service_id.clone(),catalogs[&target.kind].clone()).await?; }
        }
        targets=storage::targets(&state.db,key.clone()).await?;
        let configuration=controller_request(state,&format!("/{key}/recyclarr/configuration"),Some(json!({"operation":"reconcile","targets":configuration_context(state,&key).await?}))).await?;
        storage::configuration(&state.db,key.clone(),configuration.clone(),None).await?;
        if customized || !job.payload["files"].is_null() {
            let used=if job.payload["files"].is_null() { &configuration["used_services"] } else { &job.payload["used_services"] };
            targets.retain(|target|used.as_array().into_iter().flatten().any(|id|id==&target.service_id));
        }
        let mut remote=Vec::new();
        let mut before=serde_json::Map::new();
        for target in &targets {
            let catalog=&catalogs[&target.kind];
            if !customized && !catalog["items"].as_array().into_iter().flatten().any(|p|p["trash_id"]==target.trash_id) { return Err(ApiError::conflict("Selected guide disappeared; choose another guide")); }
            let s=service(state,&target.service_id).await?;
            let c=Connection::open(state,&s).await?;
            if !before.contains_key(&s.id) { before.insert(s.id.clone(),snapshot(&c).await?); }
            remote.push(json!({"service_id":s.id,"kind":s.kind,"container_id":s.container,"port":s.port,"url_base":s.url_base,"generation":s.generation,"trash_id":target.trash_id,"revision":target.revision,"profile_id":target.profile_id,"quality_sizes":target.quality_sizes,"reset_scores":target.reset_scores,"groups":target.groups,"overrides":target.overrides,"secret":c.key}));
        }
        let catalog=catalogs.values().next().ok_or_else(unavailable)?;
        if job.payload["configuration_revision"].as_str().is_some_and(|revision|configuration["revision"]!=revision) {
            return Err(ApiError(axum::http::StatusCode::CONFLICT,"revision_changed","Configuration revision changed; preview or sync the current files".into()));
        }
        // A lost response is reconciled from a terminal controller result before
        // another operation is submitted; a crash during writes is retried only
        // after recording the current Arr snapshots and Recyclarr state.
        let observed=controller_request(state,&format!("/{key}/recyclarr/results/{run}"),None).await?;
        let mut evidence=if observed["state"]!="interrupted" { observed } else {
            controller_request(state,&format!("/{key}/recyclarr/run"),Some(json!({"operation_id":run,"image":catalog["image"],"resources":catalog["resources"],"targets":remote,"preview":job.payload["preview"]==true,"configuration_revision":configuration["revision"],"files":job.payload["files"],"upstream":before}))).await?
        };
        evidence["before"]=json!(before);
        let outcome = ControllerOutcome::parse(evidence)?;
        let kind = outcome.kind;
        let mut evidence = outcome.evidence.clone();
        storage::record_outcome(&state.db,run.into(),outcome).await?;
        if matches!(kind, OutcomeKind::Partial | OutcomeKind::Blocked) {
            return Err(ApiError::conflict("Recyclarr reported errors; inspect the run output before retrying"));
        }
        let mut after=serde_json::Map::new();
        for target in &targets { if !after.contains_key(&target.service_id) { after.insert(target.service_id.clone(),snapshot(&Connection::open(state,&service(state,&target.service_id).await?).await?).await?); } }
        evidence["after"]=json!(after);
        if kind == OutcomeKind::Previewed {
            if evidence["before"]!=evidence["after"] { return Err(ApiError::conflict("Preview unexpectedly changed Arr settings")); }
            storage::previewed(&state.db,run.into(),evidence).await?;
            return Ok(());
        }
        for target in targets.iter().filter(|_| !customized) {
            let applied=evidence["targets"].as_array().into_iter().flatten().find(|t|t["service_id"]==target.service_id && t["trash_id"]==target.trash_id).ok_or_else(unavailable)?;
            let Some(profile)=applied["profile_id"].as_i64() else {
                if customized { continue; }
                return Err(ApiError::conflict("Recyclarr did not record the synchronized profile ID"));
            };
            let selected=evidence["after"][&target.service_id]["profiles"].as_array().into_iter().flatten().find(|p|p["id"]==profile).ok_or_else(||ApiError::conflict("Synchronized profile is missing from the target"))?;
            let generation=applied["generation"].as_str().ok_or_else(unavailable)?;
            if !storage::applied(&state.db,target.clone(),generation.into(),profile,selected["name"].as_str().ok_or_else(unavailable)?.into()).await? { return Err(ApiError::conflict("Target or selection changed during sync; retry the current selection")); }
        }
        storage::configured(&state.db,key.clone(),evidence["configured_profiles"].clone(),evidence["after"].clone()).await?;
        storage::complete(&state.db,run.into(),evidence).await?;
        state.managers.connection_wake.notify_one();
        state.emit(None,"recyclarr.changed",json!({"id":run})).await?;
        Ok::<(),ApiError>(())
    }.await;
    if let Err(error) = result {
        let retry = match RetryClass::of(&error) {
            RetryClass::Dependency | RetryClass::Contended => true,
            RetryClass::RevisionChanged => job.payload["configuration_revision"].is_null(),
            RetryClass::Blocked => false,
        } && job.attempts < 4;
        if retry
            && storage::retry(
                &state.db,
                run.into(),
                job.id.clone(),
                30_i64 * 2_i64.pow(job.attempts as u32),
                error.2.clone(),
            )
            .await?
        {
            return Ok(false);
        }
        storage::block(&state.db, run.into(), error.2.clone()).await?;
        storage::failure(&state.db, targets, error.2.clone()).await?;
        anyhow::bail!("{}", error.2);
    }
    Ok(true)
}
