#[path = "../storage/managers/bindings.rs"]
mod storage;
#[cfg(test)]
#[path = "bindings_tests.rs"]
pub(super) mod tests;

use super::*;
use std::collections::BTreeMap;

/// Every manager kind; retention reads only the movie and series managers it deletes through.
pub(super) const ALL: &[&str] = &["radarr", "sonarr", "lidarr"];

pub(super) struct ServiceInventory {
    pub generation: String,
    pub kind: String,
    pub claims: Result<BTreeMap<(i64, i64), Claim>>,
}
pub(super) struct ManagerInventory {
    /// The manager kinds read, and so the files whose ownership this inventory decides.
    kinds: &'static [&'static str],
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

/// Read the operational managers of `kinds`. This only reads, so it runs without any lock;
/// `reconcile_observation` rejects it if anything changed meanwhile.
pub(super) async fn observe(
    state: &AppState,
    kinds: &'static [&'static str],
) -> Result<ManagerInventory> {
    let files = storage::file_generations(&state.db).await?;
    let keys = storage::reconcile_read_manager_services(&state.db, kinds).await?;
    let mut services = BTreeMap::new();
    for key in keys {
        let s = service(state, &key).await?;
        let result = inventory(state, &s, None).await;
        services.insert(
            key,
            ServiceInventory::from_claims(s.generation, s.kind, result),
        );
    }
    Ok(ManagerInventory {
        kinds,
        services,
        files,
    })
}

/// Record an observation. Callers hold the exclusive manager lock for this write only.
pub(super) async fn reconcile_observation(
    state: &AppState,
    observed: &ManagerInventory,
) -> Result<()> {
    let keys = storage::reconcile_read_manager_services(&state.db, observed.kinds).await?;
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
    storage::refresh_ownership(&state.db, observed.kinds).await?;
    super::metadata::enqueue_managed_refreshes(state).await?;
    Ok(())
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/managers/reconcile", post(reconcile_api))
        .route("/api/v1/admin/managers/bindings", get(list))
        .route(
            "/api/v1/admin/managers/{id}/retention",
            axum::routing::put(retention_permission),
        )
}
#[derive(Deserialize)]
struct RetentionPermission {
    enabled: bool,
}
async fn retention_permission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<RetentionPermission>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let service = service(&state, &key).await?;
    let _guard = state.managers.guard.service(&service.kind).await;
    storage::retention_permission(&state.db, key, input.enabled, actor.user.id).await?;
    Ok(Json(json!({"saved":true})))
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

/// What reconciliation keeps of a movie, series or album. Whole libraries are decoded
/// straight into this, so memory never depends on everything else a manager lists.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entity {
    id: i64,
    tmdb_id: Option<u64>,
    tvdb_id: Option<u64>,
    foreign_album_id: Option<String>,
    has_file: Option<bool>,
    statistics: Option<Statistics>,
    /// Radarr lists a movie's file with the movie.
    movie_file: Option<FileRecord>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Statistics {
    episode_file_count: Option<u64>,
    track_file_count: Option<u64>,
}
#[derive(Deserialize)]
struct FileRecord {
    id: i64,
    path: String,
}
/// A Sonarr episode or Lidarr track, and the file it is in.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    id: i64,
    episode_file_id: Option<i64>,
    track_file_id: Option<i64>,
    /// Sonarr 4 lists an episode's file with the episode when asked to.
    episode_file: Option<FileRecord>,
}
impl Entity {
    fn external(&self, kind: &str) -> Option<String> {
        match kind {
            "radarr" => self.tmdb_id.map(|v| v.to_string()),
            "sonarr" => self.tvdb_id.map(|v| v.to_string()),
            _ => self.foreign_album_id.clone(),
        }
    }
    fn without_files(&self, kind: &str) -> bool {
        let statistics = self.statistics.as_ref();
        match kind {
            "radarr" => self.has_file == Some(false),
            "sonarr" => statistics.and_then(|s| s.episode_file_count) == Some(0),
            _ => statistics.and_then(|s| s.track_file_count) == Some(0),
        }
    }
}
impl Member {
    fn file(&self, kind: &str) -> Option<i64> {
        if kind == "sonarr" {
            self.episode_file_id
        } else {
            self.track_file_id
        }
    }
}
fn positive(value: i64) -> Result<i64> {
    Some(value).filter(|v| *v > 0).ok_or_else(unavailable)
}
/// Every file a manager claims, or only those of `entities` when an operation rechecks its
/// own selection. Movies need nothing beyond their list; each series or album needs its own.
pub(super) async fn inventory(
    state: &AppState,
    s: &Service,
    entities: Option<&[i64]>,
) -> Result<Vec<Claim>> {
    let c = Connection::open(state, s).await?;
    let endpoint = requests::endpoint(&s.kind);
    let listed: Vec<Entity> = match entities {
        None => c.fetch(endpoint, &[]).await?,
        Some(ids) => {
            let mut listed = Vec::with_capacity(ids.len());
            for id in ids {
                listed.push(c.fetch(&format!("{endpoint}/{id}"), &[]).await?);
            }
            listed
        }
    };
    let mut claims = Vec::new();
    for entity in listed {
        if entity.without_files(&s.kind) {
            continue;
        }
        let entity_id = positive(entity.id)?;
        let external_id = entity.external(&s.kind).ok_or_else(unavailable)?;
        let id = entity_id.to_string();
        let (files, members): (Vec<FileRecord>, Vec<Member>) = match s.kind.as_str() {
            "radarr" => match entity.movie_file {
                Some(file) => (vec![file], Vec::new()),
                None => (c.fetch("moviefile", &[("movieId", id)]).await?, Vec::new()),
            },
            "sonarr" => {
                let mut members: Vec<Member> = c
                    .fetch(
                        "episode",
                        &[
                            ("seriesId", id.clone()),
                            ("includeEpisodeFile", "true".into()),
                        ],
                    )
                    .await?;
                // Sonarr 3 ignores includeEpisodeFile, so its files need their own list.
                let files = if members
                    .iter()
                    .any(|m| m.episode_file_id.is_some_and(|f| f > 0) && m.episode_file.is_none())
                {
                    c.fetch("episodefile", &[("seriesId", id)]).await?
                } else {
                    let mut files = BTreeMap::new();
                    for file in members.iter_mut().filter_map(|m| m.episode_file.take()) {
                        files.insert(file.id, file);
                    }
                    files.into_values().collect()
                };
                (files, members)
            }
            _ => (
                c.fetch("trackfile", &[("albumId", id.clone())]).await?,
                c.fetch("track", &[("albumId", id)]).await?,
            ),
        };
        for file in files {
            let manager_file_id = positive(file.id)?;
            let Some(server_path) =
                storage_paths::resolve(state, &s.media_source, &file.path).await?
            else {
                continue;
            };
            let members = members
                .iter()
                .filter(|m| m.file(&s.kind) == Some(manager_file_id))
                .map(|m| positive(m.id))
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
                manager_path: file.path,
                external_id: external_id.clone(),
                members,
                server_path,
            });
        }
    }
    Ok(claims)
}
/// Read every manager without a lock, then record the result under the exclusive one.
pub(super) async fn reconcile(state: &AppState) -> Result<()> {
    let observed = observe(state, ALL).await?;
    let _guard = state.managers.guard.lock().await;
    reconcile_observation(state, &observed).await
}
async fn reconcile_api(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    reconcile(&state).await?;
    Ok(Json(json!({"reconciled":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
