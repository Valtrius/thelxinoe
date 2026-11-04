//! Durable destructive commands share the same file-generation and ownership checks.

#[path = "../storage/managers/operations.rs"]
pub(super) mod storage;

pub(super) use super::domain::Target;
use super::domain::{MediaAction, PreparedOperation, RadarrOptions, ValidatedSelection};
use super::*;
#[path = "validated_file.rs"]
mod validated_file;
use validated_file::ValidatedFile;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/media/operations", get(list).post(prepare))
        .route("/api/v1/admin/media/operations/{id}/execute", post(execute))
        .route("/api/v1/admin/media/{id}/keep", axum::routing::put(keep))
}
pub(super) async fn targets(state: &AppState, media: &str) -> Result<Vec<Target>> {
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
    prepare_operation(&state, Some(p.user.id), input.media_id, input.action).await
}
pub(super) async fn prepare_operation(
    state: &AppState,
    actor: Option<String>,
    media_id: String,
    action: String,
) -> Result<Json<Value>> {
    let action: MediaAction = serde_json::from_value(json!(action))
        .map_err(|_| ApiError::bad("Unknown media operation"))?;
    let observed = bindings::observe(state, bindings::ALL).await?;
    let _lease = state.media_operations.write().await;
    let _manager = state.managers.guard.lock().await;
    bindings::reconcile_observation(state, &observed).await?;
    let operation = prepare_from_inventory(state, actor, media_id, action, &observed).await?;
    persist_prepared(state, operation).await
}
async fn persist_prepared(state: &AppState, operation: PreparedOperation) -> Result<Json<Value>> {
    let response =
        json!({"id":operation.id,"state":"pending","files":operation.selection.files.len()});
    storage::prepare_locked(operation, &state.db).await?;
    Ok(Json(response))
}
pub(super) async fn prepare_from_inventory(
    state: &AppState,
    actor: Option<String>,
    media_id: String,
    action: MediaAction,
    observed: &bindings::ManagerInventory,
) -> Result<PreparedOperation> {
    let captured = targets(state, &media_id).await?;
    if captured.is_empty() {
        return Err(ApiError::conflict("No current files belong to this target"));
    }
    validate_ownership(&captured, action.as_str())?;
    validate_scope(&captured, &observed.services)?;
    Ok(PreparedOperation {
        id: id(),
        actor,
        media_id,
        action,
        selection: ValidatedSelection { files: captured },
    })
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
/// The movies, series or albums a selection touches in each of its managers.
fn selected_entities(files: &[Target]) -> std::collections::BTreeMap<&str, Vec<i64>> {
    let mut entities = std::collections::BTreeMap::<&str, std::collections::BTreeSet<i64>>::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        entities
            .entry(&claim.service_id)
            .or_default()
            .insert(claim.entity_id);
    }
    entities
        .into_iter()
        .map(|(key, entities)| (key, entities.into_iter().collect()))
        .collect()
}
/// Recheck the selection against its managers' current files. Only the selected movies,
/// series or albums are read: nothing else can change what this operation affects.
async fn complete_manager_scope(state: &AppState, files: &[Target]) -> Result<()> {
    let mut inventories = std::collections::BTreeMap::new();
    for (key, entities) in selected_entities(files) {
        let s = service(state, key).await?;
        let inventory = bindings::inventory(state, &s, Some(&entities)).await;
        inventories.insert(
            s.id,
            bindings::ServiceInventory::from_claims(s.generation, s.kind, inventory),
        );
    }
    validate_scope(files, &inventories)
}
fn validate_scope(
    files: &[Target],
    inventories: &std::collections::BTreeMap<String, bindings::ServiceInventory>,
) -> Result<()> {
    let mut checked = std::collections::HashSet::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        if !checked.insert(&claim.service_id) {
            continue;
        }
        let s = inventories
            .get(&claim.service_id)
            .ok_or_else(|| ApiError::conflict("Owning manager is absent from this inventory"))?;
        let inventory = s.current()?;
        for current in inventory.values() {
            let selected = files
                .iter()
                .flat_map(|f| &f.claims)
                .any(|c| c.service_id == current.service_id && c.entity_id == current.entity_id);
            if !selected {
                continue;
            }
            if s.kind != "sonarr" && !files.iter().flat_map(|f| &f.claims).any(|c| c == current) {
                return Err(ApiError::conflict(
                    "Movie or album monitoring affects files outside this selection; select its complete file set",
                ));
            }
        }
        for selected in files
            .iter()
            .flat_map(|f| &f.claims)
            .filter(|c| c.service_id == claim.service_id)
        {
            if selected.service_generation != s.generation
                || inventory.get(&(selected.entity_id, selected.manager_file_id)) != Some(selected)
            {
                return Err(ApiError::conflict(
                    "Manager file or episode identity changed",
                ));
            }
        }
    }
    Ok(())
}
async fn physical(file: &Target) -> Result<Arc<ValidatedFile>> {
    let file = file.clone();
    let proof = tokio::task::spawn_blocking(move || ValidatedFile::capture(file))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| {
            ApiError::conflict("File generation changed; rescan and prepare a new operation")
        })?;
    #[cfg(test)]
    if let Ok(control) = crate::test_support::MEDIA_VALIDATION.try_with(Clone::clone) {
        control.reached().await;
    }
    Ok(Arc::new(proof))
}
async fn recheck(proof: &Arc<ValidatedFile>) -> Result<()> {
    let proof = proof.clone();
    tokio::task::spawn_blocking(move || proof.recheck())
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| {
            ApiError::conflict("File identity or content changed; prepare a new operation")
        })
}
pub(super) async fn protected_or_active(
    state: &AppState,
    media: &str,
    files: &[Target],
) -> Result<()> {
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
/// Refuse while a manager runs commands, or has downloads that would import into `selected`:
/// the movies or series about to change, or everything it holds when `None`. A stuck download
/// of something else can't affect them. A manager that ignores the filter counts its whole
/// queue, which only refuses more.
async fn manager_idle(c: &Connection<'_>, selected: Option<&[i64]>) -> Result<()> {
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
    let filter = match (c.kind.as_str(), selected) {
        ("radarr", Some(ids)) => Some(("movieIds", ids)),
        ("sonarr", Some(ids)) => Some(("seriesIds", ids)),
        _ => None,
    };
    let mut query = vec![("pageSize", "1".to_owned())];
    if let Some((name, ids)) = filter {
        query.extend(ids.iter().map(|id| (name, id.to_string())));
    }
    let queue = c.call(reqwest::Method::GET, "queue", &query, None).await?;
    if queue["totalRecords"].as_u64() != Some(0) {
        return Err(ApiError::conflict(if filter.is_some() {
            "Manager has a download queued for this title"
        } else {
            "Manager download queue is not empty"
        }));
    }
    Ok(())
}
/// Every manager of a selection must be idle for what is selected.
async fn managers_idle(state: &AppState, files: &[Target]) -> Result<()> {
    for (key, entities) in selected_entities(files) {
        let s = service(state, key).await?;
        manager_idle(&Connection::open(state, &s).await?, Some(&entities)).await?;
    }
    Ok(())
}
/// How long a deletion under way waits for a manager, and how often it asks. Managers run short
/// scheduled commands every minute, so one will often land among a season's many files, and
/// stopping there would leave only part of the season deleted.
const IDLE_WAIT: std::time::Duration =
    std::time::Duration::from_millis(if cfg!(test) { 1_000 } else { 30_000 });
const IDLE_POLL: std::time::Duration =
    std::time::Duration::from_millis(if cfg!(test) { 20 } else { 2_000 });
async fn wait_until_idle(c: &Connection<'_>, selected: &[i64]) -> Result<()> {
    let mut refusal = None;
    let waiting = async {
        loop {
            match manager_idle(c, Some(selected)).await {
                Ok(()) => return Ok::<(), ApiError>(()),
                Err(error) => refusal = Some(error),
            }
            tokio::time::sleep(IDLE_POLL).await;
        }
    };
    // The limit includes requests a manager that stopped answering leaves hanging.
    match tokio::time::timeout(IDLE_WAIT, waiting).await {
        Ok(result) => result,
        Err(_) => Err(refusal.unwrap_or_else(unavailable)),
    }
}
/// Set Sonarr's own flag on the seasons these episodes are in. Sonarr applies a change to all
/// of a season's episodes and monitors what it later adds to a monitored season, while a series
/// search skips a season that isn't monitored.
pub(super) async fn set_seasons_monitored(
    c: &Connection<'_>,
    entity: i64,
    external: &str,
    episodes: &std::collections::BTreeSet<i64>,
    monitored: bool,
) -> Result<()> {
    let listed = c
        .call(
            reqwest::Method::GET,
            "episode",
            &[("seriesId", entity.to_string())],
            None,
        )
        .await?;
    let seasons = listed
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .filter(|e| e["id"].as_i64().is_some_and(|id| episodes.contains(&id)))
        .filter_map(|e| e["seasonNumber"].as_i64())
        .collect::<std::collections::BTreeSet<_>>();
    let mut series = c.get(&format!("series/{entity}")).await?;
    if requests::external("sonarr", &series).as_deref() != Some(external) {
        return Err(ApiError::conflict("Manager series identity changed"));
    }
    let mut changed = false;
    for season in series["seasons"].as_array_mut().ok_or_else(unavailable)? {
        let selected = season["seasonNumber"]
            .as_i64()
            .is_some_and(|number| seasons.contains(&number));
        if selected && season["monitored"] != monitored {
            season["monitored"] = json!(monitored);
            changed = true;
        }
    }
    if changed {
        c.call(
            reqwest::Method::PUT,
            &format!("series/{entity}"),
            &[],
            Some(series),
        )
        .await?;
    }
    Ok(())
}
/// Sonarr's own flag follows the episodes of a season auto-delete removed. A refresh under way
/// could write the old flag back and monitor every episode again, so this waits for it.
async fn unmonitor_seasons(state: &AppState, files: &[Target]) -> Result<()> {
    let mut selected =
        std::collections::BTreeMap::<(&str, i64, &str), std::collections::BTreeSet<i64>>::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        selected
            .entry((&claim.service_id, claim.entity_id, &claim.external_id))
            .or_default()
            .extend(&claim.members);
    }
    for ((key, entity, external), episodes) in selected {
        let s = service(state, key).await?;
        let c = Connection::open(state, &s).await?;
        wait_until_idle(&c, &[entity]).await?;
        set_seasons_monitored(&c, entity, external, &episodes, false).await?;
    }
    Ok(())
}
/// `changed` is set before the first call that can change anything upstream, so that it stays
/// false only when an attempt certainly changed nothing.
#[allow(clippy::too_many_arguments)]
async fn mutate(
    state: &AppState,
    media: &str,
    action: &str,
    files: &[Target],
    operation: &str,
    automatic: bool,
    proofs: &[Arc<ValidatedFile>],
    changed: &mut bool,
) -> Result<()> {
    validate_ownership(files, action)?;
    for (file, proof) in files.iter().zip(proofs) {
        // The wait comes first because it could outdate the checks after it.
        let manager = match file.claims.first() {
            Some(claim) => {
                let s = service(state, &claim.service_id).await?;
                if s.generation != claim.service_generation {
                    return Err(ApiError::conflict("Manager configuration changed"));
                }
                let c = Connection::open(state, &s).await?;
                wait_until_idle(&c, &[claim.entity_id]).await?;
                Some((claim, s, c))
            }
            None => None,
        };
        super::retention::revalidate_operation(state, operation, automatic).await?;
        protected_or_active(state, media, files).await?;
        recheck(proof).await?;
        if let Some((claim, s, c)) = manager {
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
                recheck(proof).await?;
                *changed = true;
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
                recheck(proof).await?;
                *changed = true;
                c.call(
                    reqwest::Method::PUT,
                    &format!("{entity}/{}", claim.entity_id),
                    &[],
                    Some(item),
                )
                .await?;
            }
            if action == "delete" {
                recheck(proof).await?;
                c.call(
                    reqwest::Method::DELETE,
                    &format!("{endpoint}/{}", claim.manager_file_id),
                    &[],
                    None,
                )
                .await?;
            }
        } else if action == "delete" {
            // Automatic deletion separately requires confirmed manager tracking.
            *changed = true;
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
    Ok(())
}
/// Exclude and remove the Radarr movies whose files a deletion removed.
async fn finish_movies(
    state: &AppState,
    media: &str,
    files: &[Target],
    options: RadarrOptions,
    actor: Option<String>,
) -> Result<()> {
    if !options.any() {
        return Ok(());
    }
    let mut finished = std::collections::BTreeSet::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        if !finished.insert((&claim.service_id, claim.entity_id)) {
            continue;
        }
        let s = service(state, &claim.service_id).await?;
        if s.generation != claim.service_generation {
            return Err(ApiError::conflict("Manager configuration changed"));
        }
        let c = Connection::open(state, &s).await?;
        super::movies::apply(&c, claim.entity_id, &claim.external_id, options).await?;
    }
    if finished.is_empty() {
        return Ok(());
    }
    super::movies::audit(state, actor, options, media.to_owned()).await
}
async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    options: Option<Json<RadarrOptions>>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let options = options.map(|Json(options)| options).unwrap_or_default();
    execute_locked(&state, key, Some(p.user.id), false, options).await
}
pub(super) async fn execute_locked(
    state: &AppState,
    key: String,
    actor: Option<String>,
    automatic: bool,
    options: RadarrOptions,
) -> Result<Json<Value>> {
    let operation_key = format!("operation:{key}");
    let _operation = state.media_resources.write(&[&operation_key]).await;
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
    // Waiting on a manager is the usual reason a deletion can't run. Asking first spares each
    // retry the whole-library read and file hashing below.
    managers_idle(state, &captured).await?;
    if automatic {
        bindings::reconcile_retained(state).await?;
    } else {
        bindings::reconcile(state).await?;
    }
    let keys = captured
        .iter()
        .map(|file| format!("file:{}", file.id))
        .collect::<Vec<_>>();
    let _files = state
        .media_resources
        .write(&keys.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    let current = targets(state, &media).await?;
    if current != captured {
        return Err(ApiError::conflict(
            "File set, generation or ownership changed; prepare a new operation",
        ));
    }
    protected_or_active(state, &media, &current).await?;
    super::retention::revalidate_operation(state, &key, automatic).await?;
    let mut proofs = Vec::new();
    for file in &current {
        proofs.push(physical(file).await?);
    }
    let _lease = state.media_operations.read().await;
    let mut services = Vec::new();
    for claim in current.iter().flat_map(|file| &file.claims) {
        services.push(service(state, &claim.service_id).await?.kind);
    }
    if options.any()
        && (action != "delete"
            || services.is_empty()
            || services.iter().any(|kind| kind != "radarr"))
    {
        return Err(ApiError::bad(
            "Only deleting a Radarr movie's files can remove or exclude it",
        ));
    }
    // Policy edits wait for the lease above, so these hold while the deletion runs.
    let retained = super::retention::options(state, &key).await?;
    let options = RadarrOptions {
        remove: options.remove || retained.is_some_and(|r| r.remove),
        ..options
    };
    let _managers = state
        .managers
        .guard
        .services(&services.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    if targets(state, &media).await? != current {
        return Err(ApiError::conflict(
            "File set, generation or ownership changed; prepare a new operation",
        ));
    }
    protected_or_active(state, &media, &current).await?;
    super::retention::revalidate_operation(state, &key, automatic).await?;
    for proof in &proofs {
        recheck(proof).await?;
    }
    // Read-only checks a busy or briefly unreachable manager can fail run while the
    // operation is still pending, so a later pass can try again; nothing has changed yet.
    complete_manager_scope(state, &current).await?;
    managers_idle(state, &current).await?;
    let k = key.clone();
    storage::execute_locked_write_media_operations(k, &state.db).await?;
    // Once executing is durable, interruption never automatically repeats a destructive call.
    let mut changed = false;
    let result = async {
        // Adding the same exclusion again changes nothing, so this alone doesn't count.
        if retained.is_some_and(|r| r.exclude) {
            super::retention::exclude(state, &current).await?;
        }
        mutate(
            state,
            &media,
            &action,
            &current,
            &key,
            automatic,
            &proofs,
            &mut changed,
        )
        .await?;
        // The files are gone by now, so this alone doesn't make the deletion a failure.
        if retained.is_some_and(|r| r.season)
            && let Err(error) = unmonitor_seasons(state, &current).await
        {
            tracing::warn!(media, error = %error.2, "Could not unmonitor the deleted season");
        }
        finish_movies(state, &media, &current, options, actor.clone()).await
    }
    .await;
    let result = match result {
        Err(error) if !changed => {
            storage::reopen_operation(key, &state.db).await?;
            return Err(error);
        }
        result => result,
    };
    let error = result.as_ref().err().map(|e| e.2.clone());
    let status = if result.is_ok() {
        "complete"
    } else {
        "uncertain"
    };
    storage::finish_operation(media, action, error, status, &state.db, key, actor).await?;
    // Monitoring and files may have changed upstream even when the operation failed.
    super::status::refresh_soon(state);
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
    storage::keep(&state.db, media, input.keep, p.user.id).await?;
    Ok(Json(json!({"saved":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}

/// Read-only: Connected services may be checked before a transfer stops them.
pub(super) async fn ensure_idle(state: &AppState, key: &str) -> Result<()> {
    let s = connected(state, key).await?;
    let c = Connection::open(state, &s).await?;
    manager_idle(&c, None).await
}
