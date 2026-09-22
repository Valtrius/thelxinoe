//! Watched-state retention uses the same captured file generations as manual deletion.

#[path = "../storage/managers/retention.rs"]
mod storage;

use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
fn exclusion_endpoint(kind: &str) -> &'static str {
    if kind == "radarr" {
        "exclusions"
    } else {
        "importlistexclusion"
    }
}
#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/retention", get(list))
        .route("/api/v1/admin/retention/policy/{domain}", post(policy))
        .route("/api/v1/admin/retention/root/{id}", post(root_policy))
        .route("/api/v1/admin/retention/evaluate", post(evaluate))
        .route("/api/v1/admin/retention/{id}/{action}", post(action))
}
#[derive(Deserialize, Serialize, Clone)]
struct Policy {
    enabled: bool,
    grace_seconds: i64,
    exclude_specials: bool,
    trigger_users: Vec<String>,
}
#[derive(Clone)]
struct Eligible {
    stamp: String,
    user: String,
    grace: i64,
    automatic: bool,
}

#[cfg(test)]
use storage::eligibility;

pub(super) async fn revalidate_operation(
    state: &AppState,
    operation: &str,
    automatic: bool,
) -> Result<()> {
    let operation = operation.to_owned();
    let valid = storage::revalidate_operation(operation, &state.db, automatic).await?;
    if !valid {
        return Err(ApiError::conflict(
            "Retention eligibility, watched state, protection or file generation changed",
        ));
    }
    Ok(())
}

async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let value = storage::list(&state.db).await?;
    Ok(Json(value))
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(domain): Path<String>,
    Json(mut input): Json<Policy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !["movies", "shows"].contains(&domain.as_str())
        || !(0..=31536000).contains(&input.grace_seconds)
        || input.trigger_users.len() > 100
    {
        return Err(ApiError::bad("Invalid retention policy"));
    }
    input.trigger_users.sort();
    input.trigger_users.dedup();
    if input.enabled && input.trigger_users.is_empty() {
        return Err(ApiError::bad("Choose at least one retention-trigger user"));
    }
    let _lease = state.media_operations.write().await;
    let valid = storage::policy(&state.db, domain, input, p).await?;
    if !valid {
        return Err(ApiError::bad("Unknown retention-trigger user"));
    }
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct RootPolicy {
    automatic_unmanaged_deletion: bool,
}
async fn root_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<RootPolicy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let changed = storage::root_policy(&state.db, key, input, p).await?;
    if !changed {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"saved":true})))
}
async fn evaluate(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let count = evaluate_all(&state, Some(p.user.id)).await?;
    Ok(Json(json!({"created":count})))
}
async fn evaluate_all(state: &AppState, actor: Option<String>) -> Result<usize> {
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    let enabled = storage::evaluate_all_read_retention_policies(&state.db).await?;
    if !enabled {
        return Ok(0);
    };
    bindings::reconcile(state).await?;
    let candidates = storage::evaluate_all_write_media(&state.db).await?;
    let mut count = 0;
    for (media, e) in candidates {
        let operation =
            operations::prepare_locked(state, actor.clone(), media.clone(), "delete".into())
                .await?
                .0;
        let key = id();
        let actor = actor.clone();
        storage::evaluate_all_write_retention_candidates(
            media, e, operation, key, actor, &state.db,
        )
        .await?;
        count += 1;
    }
    if count > 0 {
        state
            .emit(None, "retention.changed", json!({"created":count}))
            .await?;
    }
    Ok(count)
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((key, action)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    if action == "delete" {
        delete_locked(&state, &key, Some(p.user.id), false).await?;
    } else if ["cancel", "keep"].contains(&action.as_str()) {
        let changed = storage::action(&state.db, key, action, p).await?;
        if !changed {
            return Err(ApiError::conflict(
                "Only pending retention can be cancelled or kept here",
            ));
        }
    } else {
        return Err(ApiError::bad("Unsupported retention action"));
    }
    state.emit(None, "retention.changed", json!({})).await?;
    Ok(Json(json!({"saved":true})))
}
async fn delete_locked(
    state: &AppState,
    key: &str,
    actor: Option<String>,
    automatic: bool,
) -> Result<()> {
    let lookup = key.to_owned();
    let operation = storage::delete_locked_read_retention_candidates(lookup, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let result = operations::execute_locked(state, operation, actor, automatic).await;
    if let Err(error) = result {
        let key = key.to_owned();
        let message = error.2.clone();
        storage::delete_locked_write_retention_candidates(key, message, &state.db).await?;
        return Err(error);
    }
    Ok(())
}
pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        let _ = evaluate_all(&state, None).await;
        let _lease = state.media_operations.write().await;
        let _guard = state.managers.guard.lock().await;
        let ready = storage::run(&state.db).await?;
        for key in ready {
            let _ = delete_locked(&state, &key, None, true).await;
        }
        drop(_guard);
        drop(_lease);
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

pub(super) async fn exclude(
    state: &AppState,
    operation: &str,
    files: &[operations::Target],
) -> Result<()> {
    let operation = operation.to_owned();
    let retained = storage::exclude_read_retention_candidates(operation, &state.db).await?;
    if !retained {
        return Ok(());
    };
    let mut seen = BTreeSet::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        if !seen.insert((claim.service_id.clone(), claim.external_id.clone())) {
            continue;
        }
        let s = service(state, &claim.service_id).await?;
        let c = Connection::open(state, &s).await?;
        let field = if s.kind == "radarr" {
            "tmdbId"
        } else if s.kind == "sonarr" {
            "tvdbId"
        } else {
            return Err(ApiError::conflict("Music is excluded from retention"));
        };
        let external = claim
            .external_id
            .parse::<i64>()
            .map_err(|_| unavailable())?;
        let endpoint = exclusion_endpoint(&s.kind);
        let existing = c.get(endpoint).await?;
        let found = existing
            .as_array()
            .ok_or_else(unavailable)?
            .iter()
            .find(|v| v[field].as_i64() == Some(external));
        let (exclusion, created) = if let Some(found) = found {
            (found.clone(), false)
        } else {
            let item = c
                .get(&format!(
                    "{}/{}",
                    requests::endpoint(&s.kind),
                    claim.entity_id
                ))
                .await?;
            if requests::external(&s.kind, &item).as_deref() != Some(&claim.external_id) {
                return Err(ApiError::conflict(
                    "Manager identity changed before retention exclusion",
                ));
            }
            let body = if s.kind == "radarr" {
                json!({"tmdbId":external,"movieTitle":item["title"],"movieYear":item["year"]})
            } else {
                json!({"tvdbId":external,"title":item["title"]})
            };
            (
                c.call(reqwest::Method::POST, endpoint, &[], Some(body))
                    .await?,
                true,
            )
        };
        let eid = exclusion["id"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or_else(unavailable)?;
        let key = s.id.clone();
        let external = claim.external_id.clone();
        storage::exclude_write_retention_exclusions(created, eid, key, external, &state.db).await?;
    }
    Ok(())
}

/// Called under the media and manager leases before an approved acquisition search.
pub(super) async fn reacquire(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    external: &str,
    entity: i64,
) -> Result<()> {
    if !["radarr", "sonarr"].contains(&service.kind.as_str()) {
        return Ok(());
    }
    let sid = service.id.clone();
    let identity = external.to_owned();
    let (records, exclusion) =
        storage::reacquire_read_retention_candidates(sid, identity, &state.db).await?;
    let mut affected = Vec::new();
    let mut episodes = BTreeSet::new();
    for (key, targets) in records {
        let targets: Vec<operations::Target> =
            serde_json::from_str(&targets).map_err(|_| unavailable())?;
        for claim in targets
            .iter()
            .flat_map(|t| &t.claims)
            .filter(|c| c.service_id == service.id && c.external_id == external)
        {
            if claim.service_generation != service.generation || claim.entity_id != entity {
                return Err(ApiError::conflict(
                    "Retained manager identity changed; reconcile before reacquisition",
                ));
            }
            episodes.extend(claim.members.iter().copied());
            affected.push(key.clone());
        }
    }
    if service.kind == "sonarr" && !episodes.is_empty() {
        let current = connection
            .call(
                reqwest::Method::GET,
                "episode",
                &[("seriesId", entity.to_string())],
                None,
            )
            .await?;
        let current = current.as_array().ok_or_else(unavailable)?;
        if episodes.iter().any(|id| {
            !current
                .iter()
                .any(|e| e["id"].as_i64() == Some(*id) && e["seriesId"].as_i64() == Some(entity))
        }) {
            return Err(ApiError::conflict(
                "Retained episode identities changed; reconcile before reacquisition",
            ));
        }
        connection
            .call(
                reqwest::Method::PUT,
                "episode/monitor",
                &[],
                Some(json!({"episodeIds":episodes,"monitored":true})),
            )
            .await?;
    }
    if let Some((id, true)) = exclusion {
        let endpoint = exclusion_endpoint(&service.kind);
        let all = connection.get(endpoint).await?;
        let field = if service.kind == "radarr" {
            "tmdbId"
        } else {
            "tvdbId"
        };
        let external_id = external.parse::<i64>().map_err(|_| unavailable())?;
        if all.as_array().ok_or_else(unavailable)?.iter().any(|item| {
            item["id"].as_i64() == Some(id) && item[field].as_i64() == Some(external_id)
        }) {
            connection
                .call(
                    reqwest::Method::DELETE,
                    &format!("{endpoint}/{id}"),
                    &[],
                    None,
                )
                .await?;
        }
    }
    let sid = service.id.clone();
    let identity = external.to_owned();
    storage::reacquire_write_media_operations(affected, sid, identity, &state.db).await?;
    Ok(())
}
