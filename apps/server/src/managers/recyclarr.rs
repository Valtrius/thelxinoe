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
        .layer(axum::extract::DefaultBodyLimit::max(7 * 1024 * 1024))
}

async fn provision(state: &AppState) -> Result<String> {
    storage::provision(&state.db)
        .await?
        .ok_or_else(|| ApiError::conflict("Install Recyclarr in Media services"))
}
pub(super) async fn installed(state: &AppState, key: &str, actor: &str) -> Result<()> {
    storage::installed(&state.db, key.into(), actor.into()).await?;
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
