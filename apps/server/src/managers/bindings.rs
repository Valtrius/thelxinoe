#[path = "../storage/managers/bindings.rs"]
mod storage;

use super::*;
use std::collections::BTreeMap;

pub(super) struct ServiceInventory {
    pub generation: String,
    pub kind: String,
    pub claims: Result<BTreeMap<(i64, i64), Claim>>,
}
pub(super) struct ManagerInventory {
    pub services: BTreeMap<String, ServiceInventory>,
    files: BTreeMap<String, (String, String)>,
}
impl ServiceInventory {
    pub fn from_claims(generation: String, kind: String, result: Result<Vec<Claim>>) -> Self {
        let claims = result.and_then(|claims| {
            let mut indexed = BTreeMap::new();
            for claim in claims {
                if indexed
                    .insert((claim.entity_id, claim.manager_file_id), claim)
                    .is_some()
                {
                    return Err(ApiError::conflict(
                        "Manager inventory contains duplicate file identities",
                    ));
                }
            }
            Ok(indexed)
        });
        Self {
            generation,
            kind,
            claims,
        }
    }
    pub fn current(&self) -> Result<&BTreeMap<(i64, i64), Claim>> {
        self.claims
            .as_ref()
            .map_err(|e| ApiError(e.0, e.1, e.2.clone()))
    }
}

pub(super) async fn observe(state: &AppState) -> Result<ManagerInventory> {
    let files = storage::file_generations(&state.db).await?;
    let keys = storage::reconcile_read_manager_services(&state.db).await?;
    let mut services = BTreeMap::new();
    for key in keys {
        let s = service(state, &key).await?;
        let result = inventory(state, &s).await;
        services.insert(
            key,
            ServiceInventory::from_claims(s.generation, s.kind, result),
        );
    }
    Ok(ManagerInventory { services, files })
}

pub(super) async fn reconcile_observation(
    state: &AppState,
    observed: &ManagerInventory,
) -> Result<()> {
    let keys = storage::reconcile_read_manager_services(&state.db).await?;
    if keys != observed.services.keys().cloned().collect::<Vec<_>>()
        || storage::file_generations(&state.db).await? != observed.files
    {
        return Err(ApiError::conflict(
            "Manager configuration or local files changed during inventory acquisition",
        ));
    }
    for (key, inventory) in &observed.services {
        if service(state, key).await?.generation != inventory.generation {
            return Err(ApiError::conflict(
                "Manager configuration changed during inventory acquisition",
            ));
        }
    }
    for (key, inventory) in &observed.services {
        let result = inventory
            .current()
            .map(|claims| claims.values().cloned().collect());
        storage::reconcile_write_media_files(
            key.clone(),
            result,
            inventory.generation.clone(),
            &state.db,
        )
        .await?;
    }
    storage::refresh_ownership(&state.db).await?;
    super::metadata::enqueue_managed_refreshes(state).await?;
    Ok(())
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/managers/reconcile", post(reconcile_api))
        .route("/api/v1/admin/managers/bindings", get(list))
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Claim {
    pub service_id: String,
    pub service_generation: String,
    pub manager_file_id: i64,
    pub entity_id: i64,
    pub manager_path: String,
    pub external_id: String,
    // Exact manager episode/track IDs; never derived from displayed numbering.
    pub members: Vec<i64>,
    pub server_path: String,
}

fn positive(row: &Value, field: &str) -> Result<i64> {
    row[field]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)
}
pub(super) async fn inventory(state: &AppState, s: &Service) -> Result<Vec<Claim>> {
    let c = Connection::open(state, s).await?;
    let entities = c.get(requests::endpoint(&s.kind)).await?;
    let mut claims = Vec::new();
    for entity in entities.as_array().ok_or_else(unavailable)? {
        if (s.kind == "radarr" && entity["hasFile"] == false)
            || (s.kind == "sonarr" && entity["statistics"]["episodeFileCount"].as_u64() == Some(0))
            || (s.kind == "lidarr" && entity["statistics"]["trackFileCount"].as_u64() == Some(0))
        {
            continue;
        }
        let entity_id = positive(entity, "id")?;
        let external_id = requests::external(&s.kind, entity).ok_or_else(unavailable)?;
        let (files, members) = match s.kind.as_str() {
            "radarr" => (
                c.call(
                    reqwest::Method::GET,
                    "moviefile",
                    &[("movieId", entity_id.to_string())],
                    None,
                )
                .await?,
                json!([]),
            ),
            "sonarr" => (
                c.call(
                    reqwest::Method::GET,
                    "episodefile",
                    &[("seriesId", entity_id.to_string())],
                    None,
                )
                .await?,
                c.call(
                    reqwest::Method::GET,
                    "episode",
                    &[("seriesId", entity_id.to_string())],
                    None,
                )
                .await?,
            ),
            _ => (
                c.call(
                    reqwest::Method::GET,
                    "trackfile",
                    &[("albumId", entity_id.to_string())],
                    None,
                )
                .await?,
                c.call(
                    reqwest::Method::GET,
                    "track",
                    &[("albumId", entity_id.to_string())],
                    None,
                )
                .await?,
            ),
        };
        for file in files.as_array().ok_or_else(unavailable)? {
            let manager_file_id = positive(file, "id")?;
            let path = file["path"].as_str().ok_or_else(unavailable)?;
            if !clean_path(path) || suffix(path, canonical_root(&s.kind)).is_none() {
                return Err(ApiError::conflict(
                    "Manager files must use the canonical /media library path; update existing paths in the service's bulk editor",
                ));
            }
            let server_path = path.to_owned();
            let field = if s.kind == "sonarr" {
                "episodeFileId"
            } else {
                "trackFileId"
            };
            let members = members
                .as_array()
                .ok_or_else(unavailable)?
                .iter()
                .filter(|m| m[field].as_i64() == Some(manager_file_id))
                .map(|m| positive(m, "id"))
                .collect::<Result<Vec<_>>>()?;
            if s.kind != "radarr" && members.is_empty() {
                return Err(ApiError::conflict(
                    "Manager file has no exact episode or track identity",
                ));
            }
            claims.push(Claim {
                service_id: s.id.clone(),
                service_generation: s.generation.clone(),
                manager_file_id,
                entity_id,
                manager_path: path.into(),
                external_id: external_id.clone(),
                members,
                server_path,
            });
        }
    }
    Ok(claims)
}
pub(super) async fn reconcile(state: &AppState) -> Result<()> {
    let observed = observe(state).await?;
    reconcile_observation(state, &observed).await
}
async fn reconcile_api(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.managers.guard.lock().await;
    reconcile(&state).await?;
    Ok(Json(json!({"reconciled":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
