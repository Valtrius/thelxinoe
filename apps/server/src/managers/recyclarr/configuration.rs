use super::*;

pub(super) async fn editor_assistance(
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

pub(super) async fn configuration_context(state: &AppState, key: &str) -> Result<Value> {
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

pub(in crate::managers) async fn current_configuration(
    state: &AppState,
    key: &str,
) -> Result<Value> {
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

pub(super) async fn configuration_get(
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

pub(super) async fn change_configuration(
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
pub(super) async fn configuration_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "save").await
}
pub(super) async fn configuration_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "validate").await
}
pub(super) async fn configuration_defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "defaults").await
}
pub(super) async fn configuration_candidate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    change_configuration(state, headers, input, "candidate").await
}
