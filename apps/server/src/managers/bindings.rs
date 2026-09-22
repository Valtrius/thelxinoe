#[path = "../storage/managers/bindings.rs"]
mod storage;

use super::*;

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
    let services = storage::reconcile_read_manager_services(&state.db).await?;
    for key in services {
        let s = service(state, &key).await?;
        let result = inventory(state, &s).await;
        let generation = s.generation;
        // Failed reconciliation preserves every historical claim.
        storage::reconcile_write_media_files(key, result, generation, &state.db).await?;
    }
    storage::refresh_ownership(&state.db).await?;
    super::metadata::enqueue_managed_refreshes(state).await?;
    Ok(())
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
