#[path = "../storage/managers/recyclarr.rs"]
mod storage;
use super::*;
mod configuration;
use configuration::*;
mod catalog;
use catalog::*;
mod sync;
pub(super) use catalog::profiles;
pub(super) use configuration::current_configuration;
pub(super) use sync::capture_update;
pub(crate) use sync::run_job;
use sync::*;
#[cfg(test)]
#[path = "recyclarr_tests.rs"]
mod tests;

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
                n["address"] == host
                    || n["ipv6"] == host.trim_matches(['[', ']'])
                    || n["aliases"]
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
