//! Optional, independently retried service connections. Installation never enables a link.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[path = "connection_adapters.rs"]
mod adapters;
#[path = "../storage/managers/connections.rs"]
mod storage;
#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Endpoint {
    id: String,
    name: String,
    kind: String,
    container: String,
    port: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Link {
    id: String,
    source: String,
    target: String,
    source_kind: String,
    target_kind: String,
    kind: String,
    enabled: bool,
    cleanup: bool,
    state: String,
    error: Option<String>,
    attempts: u32,
    next_attempt: i64,
    upstream_id: Option<i64>,
    applied_hash: Option<String>,
    pending_hash: Option<String>,
    prepared: bool,
    recreate_missing: bool,
    updated_at: i64,
}

fn digest(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn link_id(source: &str, target: &str) -> String {
    digest(&json!([source, target]))
}
fn kind(source: &str, target: &str) -> Option<&'static str> {
    match (source, target) {
        ("prowlarr", "radarr" | "sonarr" | "lidarr") => Some("application"),
        ("radarr" | "sonarr" | "lidarr", "nzbget") => Some("download_client"),
        ("bazarr", "radarr" | "sonarr") => Some("subtitles"),
        _ => None,
    }
}
impl Link {
    fn new(source: &Endpoint, target: &Endpoint) -> Self {
        Self {
            id: link_id(&source.id, &target.id),
            source: source.id.clone(),
            target: target.id.clone(),
            source_kind: source.kind.clone(),
            target_kind: target.kind.clone(),
            kind: kind(&source.kind, &target.kind).unwrap().into(),
            enabled: false,
            cleanup: false,
            state: "available".into(),
            error: None,
            attempts: 0,
            next_attempt: 0,
            upstream_id: None,
            applied_hash: None,
            pending_hash: None,
            prepared: false,
            recreate_missing: false,
            updated_at: now(),
        }
    }
    fn presentation(&self, source: Option<&Endpoint>, target: Option<&Endpoint>) -> Value {
        // Upstream fingerprints never expose API keys or passwords to clients.
        json!({"id":self.id,"source_id":self.source,"target_id":self.target,"kind":self.kind,
            "source_kind":source.map(|s|s.kind.as_str()).unwrap_or(&self.source_kind),"source_name":source.map(|s|s.name.as_str()).unwrap_or(&self.source_kind),
            "target_kind":target.map(|s|s.kind.as_str()).unwrap_or(&self.target_kind),"target_name":target.map(|s|s.name.as_str()).unwrap_or(&self.target_kind),
            "enabled":self.enabled,"state":self.state,"error":self.error,
            "updated_at":self.updated_at,"next_attempt":self.next_attempt,"cleanup_pending":self.cleanup})
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/v1/admin/service-connections",
        get(list).post(configure),
    )
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let endpoints = storage::endpoints(&state.db).await?;
    let links = storage::list(&state.db).await?;
    let mut items = Vec::new();
    for source in &endpoints {
        for target in &endpoints {
            if kind(&source.kind, &target.kind).is_none() {
                continue;
            }
            let link = links
                .iter()
                .find(|l| l.source == source.id && l.target == target.id)
                .cloned()
                .unwrap_or_else(|| Link::new(source, target));
            items.push(link.presentation(Some(source), Some(target)));
        }
    }
    for link in &links {
        let source = endpoints.iter().find(|s| s.id == link.source);
        let target = endpoints.iter().find(|s| s.id == link.target);
        if link.cleanup && (source.is_none() || target.is_none()) {
            items.push(link.presentation(source, target));
        }
    }
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
struct Configure {
    source_id: String,
    target_id: String,
    action: String,
}
fn lock(state: &AppState, key: &str) -> Arc<tokio::sync::Mutex<()>> {
    state
        .managers
        .connection_locks
        .lock()
        .unwrap()
        .entry(key.into())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Configure>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    if !matches!(input.action.as_str(), "connect" | "disconnect" | "retry") {
        return Err(ApiError::bad("Choose Connect, Disconnect or Retry"));
    }
    let key = link_id(&input.source_id, &input.target_id);
    let mutex = lock(&state, &key);
    let _guard = mutex.lock().await;
    let _lease = state.media_operations.read().await;
    let endpoints = storage::endpoints(&state.db).await?;
    let source = endpoints
        .iter()
        .find(|s| s.id == input.source_id)
        .ok_or_else(ApiError::not_found)?;
    let target = endpoints.iter().find(|s| s.id == input.target_id);
    let saved = storage::load(&state.db, &key).await?;
    let mut link = if let Some(target) = target {
        if kind(&source.kind, &target.kind).is_none() {
            return Err(ApiError::bad(
                "These services do not support this connection",
            ));
        }
        saved.unwrap_or_else(|| Link::new(source, target))
    } else {
        saved
            .filter(|link| input.action == "retry" && !link.enabled && link.cleanup)
            .ok_or_else(ApiError::not_found)?
    };
    match input.action.as_str() {
        "connect" => {
            link.enabled = true;
            link.cleanup = false;
            link.state = "pending".into();
            link.recreate_missing = true;
        }
        "disconnect" => {
            link.enabled = false;
            link.cleanup =
                link.prepared || link.applied_hash.is_some() || link.upstream_id.is_some();
            link.state = if link.cleanup {
                "disconnecting"
            } else {
                "disconnected"
            }
            .into();
        }
        _ => {
            if !link.enabled && !link.cleanup {
                return Err(ApiError::conflict(
                    "This connection is disabled; choose Connect to enable it",
                ));
            }
            link.state = if link.enabled {
                "pending"
            } else {
                "disconnecting"
            }
            .into();
            link.recreate_missing = link.enabled;
        }
    }
    link.error = None;
    link.attempts = 0;
    link.next_attempt = 0;
    link.updated_at = now();
    storage::configure(&state.db, link.clone(), actor.user.id, input.action).await?;
    state.managers.connection_wake.notify_one();
    Ok(Json(link.presentation(Some(source), target)))
}

struct Failure {
    state: &'static str,
    message: String,
}
impl Failure {
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            state: "conflict",
            message: message.into(),
        }
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            state: "unavailable",
            message: message.into(),
        }
    }
}
impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        Self::unavailable(error.2)
    }
}
impl From<anyhow::Error> for Failure {
    fn from(_: anyhow::Error) -> Self {
        Self::unavailable("Connection state could not be saved; retrying")
    }
}
type Attempt<T> = std::result::Result<T, Failure>;

async fn process(state: &AppState, key: &str) -> anyhow::Result<()> {
    let mutex = lock(state, key);
    let _guard = mutex.lock().await;
    let _lease = state.media_operations.read().await;
    let Some(mut link) = storage::load(&state.db, key).await? else {
        return Ok(());
    };
    if (!link.enabled && !link.cleanup) || link.state == "conflict" || link.next_attempt > now() {
        return Ok(());
    }
    let endpoints = storage::endpoints(&state.db).await?;
    let source = endpoints.iter().find(|s| s.id == link.source);
    let target = endpoints.iter().find(|s| s.id == link.target);
    let outcome = if let Some(source) = source {
        adapters::apply(state, &mut link, source, target).await
    } else {
        Err(Failure::unavailable(
            "Source service is no longer connected",
        ))
    };
    match outcome {
        Ok(()) => {
            link.state = if link.enabled {
                "connected"
            } else {
                "disconnected"
            }
            .into();
            link.cleanup = false;
            link.error = None;
            link.attempts = 0;
            link.next_attempt = now() + 30;
            link.recreate_missing = false;
            if target.is_none() {
                storage::remove(&state.db, link.id).await?;
                return Ok(());
            }
        }
        Err(error) => {
            link.state = error.state.into();
            link.error = Some(error.message);
            link.attempts = link.attempts.saturating_add(1);
            link.next_attempt = now() + (2_i64.pow(link.attempts.min(5)) * 2).min(60);
        }
    }
    link.updated_at = now();
    storage::save(&state.db, link).await?;
    Ok(())
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    let _gate = state.release_gate.read().await;
    if state
        .release_quiescing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(());
    }
    let links = storage::list(&state.db).await?;
    let mut work = tokio::task::JoinSet::new();
    for link in links
        .into_iter()
        .filter(|l| (l.enabled || l.cleanup) && l.state != "conflict" && l.next_attempt <= now())
    {
        let state = state.clone();
        work.spawn(async move { process(&state, &link.id).await });
    }
    while let Some(result) = work.join_next().await {
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("A service connection attempt could not finish; it remains retryable");
        }
    }
    Ok(())
}
pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    let mut seerr_sync = tokio::time::Instant::now();
    loop {
        if tick(&state).await.is_err() {
            tracing::warn!("Service connection state is temporarily unavailable");
        }
        if tokio::time::Instant::now() >= seerr_sync {
            let _gate = state.release_gate.read().await;
            if !state
                .release_quiescing
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let _ = seerr::sync_managers(&state).await;
            }
            seerr_sync = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        }
        tokio::select! {
            _=state.managers.connection_wake.notified()=>{},
            _=tokio::time::sleep(std::time::Duration::from_secs(5))=>{},
        }
    }
}
