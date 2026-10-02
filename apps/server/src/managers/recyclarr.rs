#[path = "../storage/managers/recyclarr.rs"]
mod storage;
use super::*;

#[derive(Clone)]
struct Target {
    service_id: String,
    kind: String,
    trash_id: String,
    revision: String,
    profile_id: Option<i64>,
    quality_sizes: bool,
    reset_scores: bool,
    groups: Value,
    overrides: Value,
}
pub(super) struct Profile {
    pub trash_id: String,
    pub profile_id: i64,
    pub url: Option<String>,
}
pub(super) async fn profiles(state: &AppState, service: &str) -> Result<Vec<Profile>> {
    Ok(storage::profiles(&state.db, service.into()).await?)
}
#[derive(Deserialize)]
struct Schedule {
    paused: bool,
    hour: u32,
}
#[derive(Deserialize)]
struct Selection {
    trash_id: String,
    #[serde(default)]
    quality_sizes: bool,
    #[serde(default = "enabled")]
    reset_scores: bool,
    #[serde(default = "groups")]
    groups: Value,
    #[serde(default = "overrides")]
    overrides: Value,
}
fn enabled() -> bool {
    true
}
fn groups() -> Value {
    json!({"add":[],"skip":[]})
}
fn overrides() -> Value {
    json!({})
}
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/recyclarr/configuration/editor",
            get(editor_assistance),
        )
        .route(
            "/api/v1/admin/recyclarr/configuration",
            get(configuration_get).put(configuration_save),
        )
        .route(
            "/api/v1/admin/recyclarr/configuration/validate",
            post(configuration_validate),
        )
        .route(
            "/api/v1/admin/recyclarr/configuration/defaults",
            post(configuration_defaults),
        )
        .route(
            "/api/v1/admin/recyclarr/configuration/candidate",
            post(configuration_candidate),
        )
        .route("/api/v1/admin/recyclarr", get(list))
        .route("/api/v1/admin/recyclarr/runs/{id}", get(details))
        .route("/api/v1/admin/recyclarr/catalog/{kind}", get(catalog))
        .route("/api/v1/admin/recyclarr/schedule", post(schedule))
        .route("/api/v1/admin/recyclarr/targets/{id}", post(select))
        .route("/api/v1/admin/recyclarr/sync", post(sync))
        .route("/api/v1/admin/recyclarr/preview", post(preview))
        .route("/api/v1/admin/recyclarr/adopt/preview", post(adopt_preview))
        .route("/api/v1/admin/recyclarr/adopt", post(adopt))
        .layer(axum::extract::DefaultBodyLimit::max(7 * 1024 * 1024))
}

async fn editor_assistance(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let key = provision(&state).await?;
    let candidate = query.get("candidate").is_some_and(|value| value == "true");
    let mut metadata = controller_request(
        &state,
        &format!("/{key}/recyclarr/editor?candidate={candidate}"),
        None,
    )
    .await?;
    let Some(revision) = metadata["revision"].as_str().map(str::to_owned) else {
        metadata["schema_error"] =
            json!("This image has no verified schema identity. Recyclarr validation is available.");
        return Ok(Json(metadata));
    };
    static CACHE: std::sync::OnceLock<
        tokio::sync::Mutex<std::collections::BTreeMap<String, Value>>,
    > = std::sync::OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().await;
    if let Some(documents) = cache.get(&revision) {
        metadata["documents"] = documents.clone();
        return Ok(Json(metadata));
    }
    let result = async {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| unavailable())?;
        let mut pending = std::collections::BTreeSet::from(["config-schema.json".to_owned(), "settings-schema.json".to_owned()]);
        let mut documents = serde_json::Map::new();
        fn references(value: &Value, result: &mut Vec<String>) {
            if let Some(reference) = value.get("$ref").and_then(Value::as_str) { result.push(reference.into()); }
            if let Some(object) = value.as_object() { for child in object.values() { references(child, result); } }
            if let Some(array) = value.as_array() { for child in array { references(child, result); } }
        }
        let mut total = 0;
        while let Some(path) = pending.pop_first() {
            if documents.contains_key(&path) { continue; }
            if documents.len() >= 64 { return Err(ApiError::conflict("Schema contains too many documents")); }
            let response = client.get(format!("https://raw.githubusercontent.com/recyclarr/recyclarr/{revision}/schemas/{path}"))
                .send().await.map_err(|_| unavailable())?;
            if !response.status().is_success() { return Err(ApiError::conflict("The image's schema is unavailable")); }
            let value = read(response).await?;
            total += value.to_string().len();
            if total > 2 * 1024 * 1024 { return Err(ApiError::conflict("Schema exceeds its size limit")); }
            let mut refs = Vec::new();
            references(&value, &mut refs);
            for reference in refs {
                let relative = reference.split('#').next().unwrap_or("");
                if relative.is_empty() { continue; }
                if !relative.ends_with(".json") || !relative.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && part.bytes().all(|c|c.is_ascii_alphanumeric() || matches!(c,b'.'|b'_'|b'-'))) {
                    return Err(ApiError::conflict("Schema has an unsupported reference"));
                }
                let next = path.rsplit_once('/').map_or_else(||relative.to_owned(), |(parent,_)|format!("{parent}/{relative}"));
                if !documents.contains_key(&next) { pending.insert(next); }
            }
            documents.insert(path, value);
        }
        Ok::<Value,ApiError>(json!(documents))
    }.await;
    match result {
        Ok(documents) => {
            if cache.len() >= 8 {
                cache.pop_first();
            }
            cache.insert(revision, documents.clone());
            metadata["documents"] = documents;
        }
        Err(_) => {
            metadata["schema_error"] = json!(
                "Version-matched schema could not be loaded. Recyclarr validation is available."
            )
        }
    }
    Ok(Json(metadata))
}

async fn configuration_context(state: &AppState, key: &str) -> Result<Value> {
    let targets = storage::targets(&state.db, key.into()).await?;
    let mut result = Vec::new();
    for target in targets {
        let service = service(state, &target.service_id).await?;
        result.push(
            json!({"service_id":service.id,"kind":service.kind,"container_id":service.container,
            "port":service.port,"url_base":service.url_base,"generation":service.generation,
            "trash_id":target.trash_id,"revision":target.revision,"profile_id":target.profile_id,
            "quality_sizes":target.quality_sizes,"reset_scores":target.reset_scores,
            "groups":target.groups,"overrides":target.overrides,"secret":""}),
        );
    }
    Ok(json!(result))
}

pub(super) async fn current_configuration(state: &AppState, key: &str) -> Result<Value> {
    let mut configuration =
        controller_request(state, &format!("/{key}/recyclarr/configuration"), None).await?;
    if configuration["initialized"] != true {
        let targets = storage::targets(&state.db, key.into()).await?;
        let mut catalogs = std::collections::HashMap::new();
        let mut enrolled = std::collections::HashSet::new();
        for target in &targets {
            if !catalogs.contains_key(&target.kind) {
                catalogs.insert(
                    target.kind.clone(),
                    fetch_catalog(state, key, &target.kind).await?,
                );
            }
            if enrolled.insert(target.service_id.clone()) {
                storage::enroll(
                    &state.db,
                    key.into(),
                    target.service_id.clone(),
                    catalogs[&target.kind].clone(),
                )
                .await?;
            }
        }
        if catalogs.is_empty() {
            fetch_catalog(state, key, "radarr").await?;
        }
        configuration = controller_request(
            state,
            &format!("/{key}/recyclarr/configuration"),
            Some(json!({
                "operation":"initialize","targets":configuration_context(state,key).await?
            })),
        )
        .await?;
    }
    storage::configuration(&state.db, key.into(), configuration.clone(), None).await?;
    Ok(configuration)
}

async fn configuration_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.managers.guard.service("recyclarr").await;
    let key = provision(&state).await?;
    let mut configuration = current_configuration(&state, &key).await?;
    let mut bindings = std::collections::BTreeMap::new();
    for target in storage::targets(&state.db, key).await? {
        let instance = format!("managed_{}", target.service_id.replace('-', ""));
        bindings.insert(target.service_id.clone(),json!({"service_id":target.service_id,"kind":target.kind,"instance":instance,"base_url":format!("{instance}_base_url"),"api_key":format!("{instance}_api_key")}));
    }
    configuration["bindings"] = json!(bindings.into_values().collect::<Vec<_>>());
    Ok(Json(configuration))
}

async fn change_configuration(
    state: AppState,
    headers: HeaderMap,
    mut input: Value,
    operation: &str,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let _maintenance = state.managers.maintenance(&state).await;
    let _guard = state
        .managers
        .guard
        .services(&["recyclarr", "radarr", "sonarr"])
        .await;
    let key = provision(&state).await?;
    let configuration = current_configuration(&state, &key).await?;
    if !input.is_object() {
        return Err(ApiError::bad("Supply a YAML file set"));
    }
    input["operation"] = json!(operation);
    input["targets"] = configuration_context(&state, &key).await?;
    if operation != "candidate" {
        let files = if operation == "defaults" {
            &configuration["defaults"]["files"]
        } else {
            &input["files"]
        };
        let inspected = controller_request(&state, &format!("/{key}/recyclarr/configuration"),
            Some(json!({"operation":"inspect","revision":input["revision"],"files":files,"targets":input["targets"]}))).await?;
        if inspected["valid"] != true {
            if operation == "validate" {
                return Ok(Json(inspected));
            }
            return Err(ApiError::bad(
                "Invalid YAML file set; validate the draft to see its problems",
            ));
        }
        let mut secrets = std::collections::HashMap::new();
        for target in input["targets"].as_array_mut().ok_or_else(unavailable)? {
            let id = target["service_id"]
                .as_str()
                .ok_or_else(unavailable)?
                .to_owned();
            if !inspected["used_services"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|used| used == &id)
            {
                continue;
            }
            if !secrets.contains_key(&id) {
                let service = service(&state, &id).await?;
                secrets.insert(id.clone(), Connection::open(&state, &service).await?.key);
            }
            target["secret"] = json!(secrets[&id]);
        }
    }
    input["required_profiles"] = json!(storage::required_profiles(&state.db).await?);
    let result = controller_request(
        &state,
        &format!("/{key}/recyclarr/configuration"),
        Some(input),
    )
    .await?;
    if result["revision"].is_string() {
        storage::configuration(&state.db, key, result.clone(), Some(actor.user.id)).await?;
        state
            .emit(
                None,
                "recyclarr.changed",
                json!({"configuration_revision":result["revision"]}),
            )
            .await?;
    }
    Ok(Json(result))
}
async fn configuration_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "save").await
}
async fn configuration_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "validate").await
}
async fn configuration_defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "defaults").await
}
async fn configuration_candidate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "candidate").await
}
async fn provision(state: &AppState) -> Result<String> {
    storage::provision(&state.db)
        .await?
        .ok_or_else(|| ApiError::conflict("Install Recyclarr in Media services"))
}
pub(super) async fn installed(
    state: &AppState,
    key: &str,
    actor: &str,
    adopted: bool,
) -> Result<()> {
    storage::installed(&state.db, key.into(), actor.into()).await?;
    let targets = storage::targets(&state.db, key.into()).await?;
    if adopted && !targets.is_empty() {
        controller_request(state,&format!("/{key}/recyclarr/import"),Some(json!({"targets":targets.iter().map(|t|json!({"service_id":t.service_id,"kind":t.kind,"trash_id":t.trash_id})).collect::<Vec<_>>()}))).await?;
    }
    Ok(())
}
pub(super) async fn owns(state: &AppState, service: &str) -> Result<bool> {
    Ok(storage::owns(&state.db, service.into()).await?)
}
pub(crate) async fn tick(state: &AppState) -> anyhow::Result<()> {
    use std::sync::atomic::Ordering;
    let at = now();
    let previous = state.managers.recyclarr_tick.load(Ordering::Relaxed);
    if at - previous < 30 || state.release_quiescing.load(Ordering::SeqCst) {
        return Ok(());
    }
    if state
        .managers
        .recyclarr_tick
        .compare_exchange(previous, at, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        if let Some(key) = storage::provision(&state.db).await?
            && let Ok(configuration) =
                controller_request(state, &format!("/{key}/recyclarr/configuration"), None).await
            && configuration["initialized"] == true
        {
            storage::configuration(&state.db, key, configuration, None).await?;
        }
        storage::tick(&state.db).await?;
    }
    Ok(())
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(storage::list(&state.db).await?))
}
async fn details(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(
        storage::details(&state.db, id)
            .await?
            .ok_or_else(ApiError::not_found)?,
    ))
}
async fn fetch_catalog(state: &AppState, key: &str, kind: &str) -> Result<Value> {
    controller_request(
        state,
        &format!("/{key}/recyclarr/catalog"),
        Some(json!({"kind":kind})),
    )
    .await
}
async fn catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(
        fetch_catalog(&state, &provision(&state).await?, &kind).await?,
    ))
}
async fn schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Schedule>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    if input.hour > 23 {
        return Err(ApiError::bad("Choose a daily hour from 0 to 23"));
    }
    storage::schedule(&state.db, provision(&state).await?, actor.user.id, input).await?;
    Ok(Json(json!({"accepted":true})))
}
async fn select(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Selection>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let s = service(&state, &key).await?;
    if current_configuration(&state, &provision(&state).await?).await?["mode"] == "customized" {
        return Err(ApiError::conflict(
            "Edit guide profiles in Recyclarr YAML while using a customized configuration",
        ));
    }
    let catalog = fetch_catalog(&state, &provision(&state).await?, &s.kind).await?;
    let selected = catalog["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["trash_id"] == input.trash_id)
        .ok_or_else(|| ApiError::bad("Choose a current guide profile"))?;
    if !input.overrides.as_object().is_some_and(|m| {
        m.iter().all(|(k, v)| match k.as_str() {
            "min_format_score" | "upgrade_until_score" => {
                v.as_i64().is_some_and(|v| (-100000..=100000).contains(&v))
            }
            "upgrade_allowed" => v.is_boolean(),
            _ => false,
        })
    }) {
        return Err(ApiError::bad("Unsupported guide override"));
    }
    if !input
        .groups
        .as_object()
        .is_some_and(|m| m.keys().all(|k| matches!(k.as_str(), "add" | "skip")))
    {
        return Err(ApiError::bad("Invalid custom-format groups"));
    }
    for field in ["add", "skip"] {
        let rows = input.groups[field]
            .as_array()
            .ok_or_else(|| ApiError::bad("Choose compatible custom-format groups"))?;
        for value in rows {
            let trash = if field == "add" {
                value["trash_id"].as_str()
            } else {
                value.as_str()
            };
            if trash.is_none()
                || !selected["groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|g| g["trash_id"].as_str() == trash)
                || (field == "add" && value.as_object().is_none_or(|m| m.len() != 1))
            {
                return Err(ApiError::bad(
                    "Group is incompatible with this guide profile",
                ));
            }
        }
    }
    let run = storage::select(&state.db, key, input, actor.user.id).await?;
    Ok(Json(json!({"id":run,"state":"queued"})))
}
async fn queue(
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
async fn sync(
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
async fn preview(
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
#[derive(Deserialize)]
struct Adoption {
    review_id: String,
    container_id: String,
    released_compose: bool,
}
async fn adopt_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let mut review = controller_request(
        &state,
        "/adopt/preview",
        Some(json!({"kind":"recyclarr","container_id":input["container_id"]})),
    )
    .await?;
    let managers = super::storage::list(&state.db).await?;
    let mut targets = Vec::new();
    for imported in review["configuration"]["targets"]
        .as_array()
        .ok_or_else(unavailable)?
    {
        let s = managers
            .iter()
            .find(|s| s["kind"] == imported["kind"])
            .ok_or_else(|| {
                ApiError::conflict(
                    "Connect every imported Radarr/Sonarr instance before transferring Recyclarr",
                )
            })?;
        let s = service(&state, s["id"].as_str().ok_or_else(unavailable)?).await?;
        let c = Connection::open(&state, &s).await?;
        let supplied = url::Url::parse(imported["url"].as_str().ok_or_else(unavailable)?)
            .map_err(|_| ApiError::bad("Invalid imported target URL"))?;
        let live = url::Url::parse(&c.base).map_err(|_| unavailable())?;
        let raw = docker(&state, &format!("containers/{}", s.container)).await?;
        let alias = supplied.host_str().is_some_and(|host| {
            raw["networks"].as_array().into_iter().flatten().any(|n| {
                n["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|a| a == host)
            }) || raw["name"]
                .as_str()
                .is_some_and(|name| name.trim_start_matches('/') == host)
        });
        if supplied.scheme() != "http"
            || supplied.port_or_known_default() != Some(s.port)
            || supplied.path().trim_end_matches('/') != s.url_base
            || (supplied.host_str() != live.host_str() && !alias)
            || imported["secret"] != c.key
            || !supplied.username().is_empty()
            || supplied.password().is_some()
            || supplied.query().is_some()
            || supplied.fragment().is_some()
        {
            return Err(ApiError::conflict(
                "Imported target URL or API key does not match its connected manager",
            ));
        }
        targets.push(json!({"service_id":s.id,"generation":s.generation,"kind":s.kind,"name":s.name,"trash_id":imported["trash_id"],"quality_sizes":imported["quality_sizes"],"groups":imported["groups"],"overrides":imported["overrides"],"guide_name":imported["guide_name"]}));
    }
    review.as_object_mut().unwrap().remove("configuration");
    review["targets"] = json!(targets);
    review["actor_id"] = json!(actor.user.id);
    review["created_at"] = json!(now());
    storage::review(
        &state.db,
        review["review_id"].as_str().ok_or_else(unavailable)?.into(),
        review.clone(),
    )
    .await?;
    Ok(Json(review))
}
async fn adopt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Adoption>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    controller_request(&state,"/adopt/check",Some(json!({"operation_id":input.review_id,"kind":"recyclarr","container_id":input.container_id,"released_compose":input.released_compose}))).await?;
    let key = input.review_id.clone();
    storage::adopt(&state.db, actor.user.id, input).await?;
    Ok(Json(json!({"id":key,"state":"queued"})))
}
async fn snapshot(c: &Connection<'_>) -> Result<Value> {
    Ok(
        json!({"profiles":c.get("qualityprofile").await?,"formats":c.get("customformat").await?,"sizes":c.get("qualitydefinition").await?,"naming":c.get("config/naming").await?,"management":c.get("config/mediamanagement").await?}),
    )
}
pub(super) async fn capture_update(state: &AppState, key: &str) -> Result<()> {
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
        storage::progress(
            &state.db,
            run.into(),
            "cancelled".into(),
            json!({}),
            Some("Automatic sync paused".into()),
        )
        .await?;
        return Ok(true);
    }
    let mut targets = storage::targets(&state.db, key.clone()).await?;
    let result=async {
        if !authorized { return Err(ApiError::forbidden()); }
        if targets.is_empty() { return Err(ApiError::conflict("Connect Radarr or Sonarr before syncing")); }
        let configuration=current_configuration(state,&key).await?;
        let customized=configuration["mode"]=="customized";
        storage::progress(&state.db,run.into(),"running".into(),json!({}),None).await?;
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
            return Err(ApiError::conflict("Configuration revision changed; preview or sync the current files"));
        }
        // A lost response is reconciled from a terminal controller result before
        // another operation is submitted; a crash during writes is retried only
        // after recording the current Arr snapshots and Recyclarr state.
        let observed=controller_request(state,&format!("/{key}/recyclarr/results/{run}"),None).await?;
        let mut evidence=if observed["state"]!="interrupted" { observed } else {
            controller_request(state,&format!("/{key}/recyclarr/run"),Some(json!({"operation_id":run,"image":catalog["image"],"resources":catalog["resources"],"targets":remote,"preview":job.payload["preview"]==true,"configuration_revision":configuration["revision"],"files":job.payload["files"],"upstream":before}))).await?
        };
        evidence["before"]=json!(before);
        let mut after=serde_json::Map::new();
        for target in &targets { if !after.contains_key(&target.service_id) { after.insert(target.service_id.clone(),snapshot(&Connection::open(state,&service(state,&target.service_id).await?).await?).await?); } }
        evidence["after"]=json!(after);
        if evidence["state"]=="previewed" {
            if evidence["before"]!=evidence["after"] { return Err(ApiError::conflict("Preview unexpectedly changed Arr settings")); }
            storage::progress(&state.db,run.into(),"previewed".into(),evidence,None).await?;
            return Ok(());
        }
        if evidence["state"]!="applied" {
            let stage = if evidence["state"] == "partial" { "partial" } else { "blocked" };
            storage::progress(&state.db,run.into(),stage.into(),evidence,Some("Sync did not finish; inspect the run output before retrying".into())).await?;
            return Err(ApiError::conflict("Recyclarr reported errors; the last valid acquisition defaults were retained"));
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
        storage::progress(&state.db,run.into(),"complete".into(),evidence,None).await?;
        state.managers.connection_wake.notify_one();
        state.emit(None,"recyclarr.changed",json!({"id":run})).await?;
        Ok::<(),ApiError>(())
    }.await;
    if let Err(error) = result {
        let retryable = error.2.contains("unavailable")
            || error.2.contains("access")
            || error.2.contains("revision changed")
                && job.payload["configuration_revision"].is_null();
        let retry = retryable && job.attempts < 4;
        storage::progress(
            &state.db,
            run.into(),
            if retry { "retrying" } else { "blocked" }.into(),
            json!({}),
            Some(error.2.clone()),
        )
        .await?;
        if retry {
            storage::retry(
                &state.db,
                job.id.clone(),
                30_i64 * 2_i64.pow(job.attempts as u32),
            )
            .await?;
            return Ok(false);
        }
        storage::failure(&state.db, targets, error.2.clone()).await?;
        anyhow::bail!("{}", error.2);
    }
    Ok(true)
}
