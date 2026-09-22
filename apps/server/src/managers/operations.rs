//! Durable destructive commands share the same file-generation and ownership checks.

#[path = "../storage/managers/operations.rs"]
mod storage;

use super::*;
use std::path::PathBuf;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/media/operations", get(list).post(prepare))
        .route("/api/v1/admin/media/operations/{id}/execute", post(execute))
        .route("/api/v1/admin/media/{id}/keep", axum::routing::put(keep))
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Target {
    id: String,
    generation: String,
    path: String,
    root: String,
    size: u64,
    modified: String,
    fingerprint: String,
    ownership: String,
    pub(super) claims: Vec<bindings::Claim>,
}
async fn targets(state: &AppState, media: &str) -> Result<Vec<Target>> {
    let media = media.to_owned();
    storage::targets(media, &state.db)
        .await
        .map_err(ApiError::from)
}
#[derive(Deserialize)]
struct Prepare {
    media_id: String,
    action: String,
}
async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Prepare>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !matches!(input.action.as_str(), "delete" | "unmonitor" | "monitor") {
        return Err(ApiError::bad("Unknown media operation"));
    }
    let _lease = state.media_operations.write().await;
    let _manager = state.managers.guard.lock().await;
    prepare_locked(&state, Some(p.user.id), input.media_id, input.action).await
}
pub(super) async fn prepare_locked(
    state: &AppState,
    actor: Option<String>,
    media_id: String,
    action: String,
) -> Result<Json<Value>> {
    bindings::reconcile(state).await?;
    let captured = targets(state, &media_id).await?;
    if captured.is_empty() {
        return Err(ApiError::conflict("No current files belong to this target"));
    }
    validate_ownership(&captured, &action)?;
    complete_manager_scope(state, &captured).await?;
    let key = id();
    let returned = key.clone();
    let count = captured.len();
    storage::prepare_locked(captured, key, &state.db, actor, media_id, action).await?;
    Ok(Json(json!({"id":returned,"state":"pending","files":count})))
}
fn validate_ownership(files: &[Target], action: &str) -> Result<()> {
    for file in files {
        match file.ownership.as_str() {
            "managed" => (),
            "unmanaged" if action == "delete" && file.claims.is_empty() => (),
            _ => {
                return Err(ApiError::conflict(
                    "Ownership is unresolved, or this action requires an owning manager",
                ));
            }
        }
    }
    Ok(())
}
async fn complete_manager_scope(state: &AppState, files: &[Target]) -> Result<()> {
    let mut checked = std::collections::HashSet::new();
    for file in files {
        for claim in &file.claims {
            if !checked.insert(claim.service_id.clone()) {
                continue;
            }
            let s = service(state, &claim.service_id).await?;
            let inventory = bindings::inventory(state, &s).await?;
            for current in &inventory {
                let selected = files.iter().flat_map(|f| &f.claims).any(|c| {
                    c.service_id == current.service_id && c.entity_id == current.entity_id
                });
                if !selected {
                    continue;
                }
                if s.kind != "sonarr" && !files.iter().flat_map(|f| &f.claims).any(|c| c == current)
                {
                    return Err(ApiError::conflict(
                        "Movie or album monitoring affects files outside this selection; select its complete file set",
                    ));
                }
            }
            for selected in files
                .iter()
                .flat_map(|f| &f.claims)
                .filter(|c| c.service_id == s.id)
            {
                if !inventory.contains(selected) {
                    return Err(ApiError::conflict(
                        "Manager file or episode identity changed",
                    ));
                }
            }
        }
    }
    Ok(())
}
async fn physical(file: &Target) -> Result<()> {
    let path = PathBuf::from(&file.path);
    let root = tokio::fs::canonicalize(&file.root)
        .await
        .map_err(|_| unavailable())?;
    let actual = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| ApiError::conflict("Target file moved or disappeared"))?;
    if !actual.starts_with(&root)
        || actual != path
        || tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| unavailable())?
            .file_type()
            .is_symlink()
    {
        return Err(ApiError::conflict(
            "Target path is no longer an ordinary file inside its library",
        ));
    }
    let expected = file.clone();
    let valid = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let before = file.metadata()?;
        if !before.is_file()
            || before.len() != expected.size
            || before
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
                .to_string()
                != expected.modified
        {
            return Ok(false);
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        let after = std::fs::metadata(path)?;
        Ok(hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            == expected.fingerprint
            && before.len() == after.len()
            && before.modified()? == after.modified()?)
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    if !valid {
        return Err(ApiError::conflict(
            "File generation changed; rescan and prepare a new operation",
        ));
    }
    Ok(())
}
async fn protected_or_active(state: &AppState, media: &str, files: &[Target]) -> Result<()> {
    let media = media.to_owned();
    let ids = json!(files.iter().map(|f| &f.id).collect::<Vec<_>>()).to_string();
    let blocked = storage::protected_or_active(media, ids, &state.db).await?;
    if blocked {
        return Err(ApiError::conflict(
            "Media is protected by Keep or has an active playback session",
        ));
    }
    Ok(())
}
async fn manager_idle(c: &Connection<'_>) -> Result<()> {
    let commands = c.get("command").await?;
    if commands
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .any(|v| matches!(v["status"].as_str(), Some("queued" | "started")))
    {
        return Err(ApiError::conflict(
            "Manager is busy; retry after its active commands finish",
        ));
    }
    let queue = c
        .call(
            reqwest::Method::GET,
            "queue",
            &[("pageSize", "1".into())],
            None,
        )
        .await?;
    if queue["totalRecords"].as_u64() != Some(0) {
        return Err(ApiError::conflict("Manager download queue is not empty"));
    }
    Ok(())
}
async fn mutate(
    state: &AppState,
    media: &str,
    action: &str,
    files: &[Target],
    operation: &str,
    automatic: bool,
) -> Result<()> {
    validate_ownership(files, action)?;
    complete_manager_scope(state, files).await?;
    protected_or_active(state, media, files).await?;
    for file in files {
        physical(file).await?;
    }
    // Recheck every manager before the first mutation, then each immediately before its call.
    for file in files {
        for claim in &file.claims {
            let s = service(state, &claim.service_id).await?;
            let c = Connection::open(state, &s).await?;
            manager_idle(&c).await?;
        }
    }
    for file in files {
        super::retention::revalidate_operation(state, operation, automatic).await?;
        protected_or_active(state, media, files).await?;
        physical(file).await?;
        if let Some(claim) = file.claims.first() {
            let s = service(state, &claim.service_id).await?;
            if s.generation != claim.service_generation {
                return Err(ApiError::conflict("Manager configuration changed"));
            }
            let c = Connection::open(state, &s).await?;
            manager_idle(&c).await?;
            let endpoint = match s.kind.as_str() {
                "radarr" => "moviefile",
                "sonarr" => "episodefile",
                _ => "trackfile",
            };
            let current = c
                .get(&format!("{endpoint}/{}", claim.manager_file_id))
                .await?;
            if current["path"].as_str() != Some(&claim.manager_path) {
                return Err(ApiError::conflict("Manager file identity changed"));
            }
            let monitored = action == "monitor";
            if s.kind == "sonarr" {
                if claim.members.is_empty() {
                    return Err(ApiError::conflict("Exact manager episodes are unresolved"));
                }
                let series = c.get(&format!("series/{}", claim.entity_id)).await?;
                let episodes = c
                    .call(
                        reqwest::Method::GET,
                        "episode",
                        &[("seriesId", claim.entity_id.to_string())],
                        None,
                    )
                    .await?;
                let current_members = episodes
                    .as_array()
                    .ok_or_else(unavailable)?
                    .iter()
                    .filter(|e| e["episodeFileId"].as_i64() == Some(claim.manager_file_id))
                    .filter_map(|e| e["id"].as_i64())
                    .collect::<std::collections::BTreeSet<_>>();
                if requests::external(&s.kind, &series).as_deref() != Some(&claim.external_id)
                    || current_members != claim.members.iter().copied().collect()
                {
                    return Err(ApiError::conflict(
                        "Manager episode ownership changed before deletion",
                    ));
                }
                c.call(
                    reqwest::Method::PUT,
                    "episode/monitor",
                    &[],
                    Some(json!({"episodeIds":claim.members,"monitored":monitored})),
                )
                .await?;
            } else {
                let entity = if s.kind == "radarr" { "movie" } else { "album" };
                let mut item = c.get(&format!("{entity}/{}", claim.entity_id)).await?;
                if requests::external(&s.kind, &item).as_deref() != Some(&claim.external_id) {
                    return Err(ApiError::conflict("Manager entity identity changed"));
                }
                item["monitored"] = json!(monitored);
                c.call(
                    reqwest::Method::PUT,
                    &format!("{entity}/{}", claim.entity_id),
                    &[],
                    Some(item),
                )
                .await?;
            }
            if action == "delete" {
                c.call(
                    reqwest::Method::DELETE,
                    &format!("{endpoint}/{}", claim.manager_file_id),
                    &[],
                    None,
                )
                .await?;
            }
        } else if action == "delete" {
            // Retention additionally requires the root's automatic-deletion opt-in.
            tokio::fs::remove_file(&file.path)
                .await
                .map_err(|_| ApiError::conflict("Could not remove the confirmed-unmanaged file"))?;
        }
    }
    if action == "delete" {
        for file in files {
            if tokio::fs::try_exists(&file.path)
                .await
                .map_err(|_| unavailable())?
            {
                return Err(ApiError::conflict(
                    "Manager accepted deletion but the file is still present; reconcile before retrying",
                ));
            }
        }
        let ids = json!(files.iter().map(|f| &f.id).collect::<Vec<_>>()).to_string();
        storage::mutate(ids, &state.db).await?;
    }
    bindings::reconcile(state).await?;
    Ok(())
}
async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let _manager = state.managers.guard.lock().await;
    execute_locked(&state, key, Some(p.user.id), false).await
}
pub(super) async fn execute_locked(
    state: &AppState,
    key: String,
    actor: Option<String>,
    automatic: bool,
) -> Result<Json<Value>> {
    let k = key.clone();
    let (media, action, status, saved) =
        storage::execute_locked_read_media_operations(k, &state.db)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "Operation already attempted; reconcile and prepare a new command",
        ));
    }
    let captured: Vec<Target> = serde_json::from_str(&saved).map_err(|_| unavailable())?;
    bindings::reconcile(state).await?;
    let current = targets(state, &media).await?;
    if current != captured {
        return Err(ApiError::conflict(
            "File set, generation or ownership changed; prepare a new operation",
        ));
    }
    protected_or_active(state, &media, &current).await?;
    super::retention::revalidate_operation(state, &key, automatic).await?;
    for file in &current {
        physical(file).await?;
    }
    let k = key.clone();
    storage::execute_locked_write_media_operations(k, &state.db).await?;
    // Once executing is durable, interruption never automatically repeats a destructive call.
    let result = async {
        super::retention::exclude(state, &key, &current).await?;
        mutate(state, &media, &action, &current, &key, automatic).await
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    let status = if result.is_ok() {
        "complete"
    } else {
        "uncertain"
    };
    storage::finish_operation(media, action, error, status, &state.db, key, actor).await?;
    result?;
    Ok(Json(json!({"state":"complete"})))
}
#[derive(Deserialize)]
struct Keep {
    keep: bool,
}
async fn keep(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Json(input): Json<Keep>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    storage::keep(&state.db, media, input, p).await?;
    Ok(Json(json!({"saved":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}

pub(super) async fn ensure_idle(state: &AppState, key: &str) -> Result<()> {
    let s = service(state, key).await?;
    let c = Connection::open(state, &s).await?;
    manager_idle(&c).await
}
