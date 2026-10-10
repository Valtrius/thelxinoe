//! Local Docker manager adapters. Docker evidence stays behind the private controller socket.

#[path = "../storage/managers.rs"]
mod storage;

pub(crate) mod access;
mod bindings;
mod connections;
mod controls;
mod domain;
mod failures;
mod indexers;
mod metadata;
mod operations;
mod quality;
mod recreation;
pub(crate) mod recyclarr;
mod requests;
mod retention;
mod seerr;
pub(crate) use seerr::observe_requests;
mod stack;
mod support;
pub(crate) use connections::run as run_connections;
pub(crate) use support::operational_health;
mod updates;
pub(crate) use retention::run as run_retention;
pub(crate) use stack::controller as controller_request;
pub(crate) use stack::provision;
pub(crate) use updates::{run as run_updates, run_job as update_service};
mod connection_settings;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
pub(crate) use metadata::run as run_metadata;
pub(crate) use requests::acquire;

pub(crate) async fn reconcile_after_scan(state: &AppState) {
    let _guard = state.managers.guard.lock().await;
    let _ = bindings::reconcile(state).await;
}
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use thelxinoe_core::{Capability, id, now};
pub(crate) struct Runtime {
    http: reqwest::Client,
    proxy_http: reqwest::Client,
    guard: thelxinoe_core::operation_locks::OperationLocks,
    maintenance: tokio::sync::Mutex<std::sync::Weak<tokio::sync::OwnedRwLockWriteGuard<()>>>,
    connection_locks:
        std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    connection_wake: tokio::sync::Notify,
    recyclarr_tick: std::sync::atomic::AtomicI64,
    #[cfg(test)]
    pub(crate) docker: std::sync::Mutex<std::collections::HashMap<String, Value>>,
}
impl Runtime {
    // Service mutations share one media lease while retaining their own service
    // lock. Playback and media deletion still cannot race an idle preflight.
    async fn maintenance(&self, state: &AppState) -> Arc<tokio::sync::OwnedRwLockWriteGuard<()>> {
        let mut lease = self.maintenance.lock().await;
        if let Some(active) = lease.upgrade() {
            return active;
        }
        let active = Arc::new(state.media_operations.clone().write_owned().await);
        *lease = Arc::downgrade(&active);
        active
    }

    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            proxy_http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(90))
                .build()?,
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            guard: Default::default(),
            maintenance: Default::default(),
            connection_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            connection_wake: tokio::sync::Notify::new(),
            recyclarr_tick: std::sync::atomic::AtomicI64::new(0),
            #[cfg(test)]
            docker: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }
}
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(access::router())
        .merge(requests::router())
        .merge(bindings::router())
        .merge(controls::router())
        .merge(quality::router())
        .merge(recyclarr::router())
        .merge(seerr::router())
        .merge(indexers::router())
        .merge(metadata::router())
        .merge(operations::router())
        .merge(support::router())
        .merge(connections::router())
        .merge(connection_settings::router())
        .merge(stack::router())
        .merge(updates::router())
        .merge(retention::router())
        .route("/api/v1/admin/managers/containers", get(containers))
        .route("/api/v1/admin/managers", get(list).post(register))
        .route("/api/v1/admin/managers/{id}/options", get(options))
        .route(
            "/api/v1/admin/managers/{id}/defaults",
            axum::routing::put(defaults),
        )
        .route("/api/v1/admin/managers/{id}/test", post(test))
}
fn unavailable() -> ApiError {
    ApiError(
        axum::http::StatusCode::CONFLICT,
        "dependency_unavailable",
        "Manager or its Docker evidence is unavailable; check the service and controller".into(),
    )
}
async fn docker(state: &AppState, path: &str) -> Result<Value> {
    #[cfg(test)]
    if let Some(v) = state.managers.docker.lock().unwrap().get(path) {
        // Tests record a removed container as null evidence.
        if v.is_null() {
            return Err(failures::container_missing());
        }
        return Ok(v.clone());
    }
    #[cfg(unix)]
    {
        let client = reqwest::Client::builder()
            .unix_socket(state.config.controller_socket.clone())
            .no_proxy()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|_| unavailable())?;
        let response = client
            .get(format!("http://controller/docker/{path}"))
            .send()
            .await
            .map_err(|_| failures::controller_unreachable())?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(failures::container_missing());
        }
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            let message = message.trim();
            return Err(if status.is_client_error() && !message.is_empty() {
                ApiError::conflict(message.chars().take(300).collect::<String>())
            } else {
                unavailable()
            });
        }
        read(response).await
    }
    #[cfg(not(unix))]
    {
        let _ = (state, path);
        Err(unavailable())
    }
}
async fn read(mut response: reqwest::Response) -> Result<Value> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err(ApiError::conflict(
                "Manager response exceeds the supported size",
            ));
        }
        bytes.extend(chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::conflict("Manager returned an invalid response"))
}
fn clean_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && !value.split('/').any(|p| p == ".." || p == ".")
}
fn host_path(value: &str) -> Option<String> {
    if clean_path(value) {
        return Some(value.trim_end_matches('/').into());
    }
    let bytes = value.as_bytes();
    if bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        let path = value.replace('\\', "/").to_ascii_lowercase();
        if !path.contains('\0') && !path.split('/').any(|p| p == "." || p == "..") {
            return Some(path.trim_end_matches('/').into());
        }
    }
    None
}
fn suffix<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim_end_matches('/');
    if path == root {
        Some("")
    } else {
        path.strip_prefix(root).filter(|v| v.starts_with('/'))
    }
}
#[path = "storage_paths.rs"]
mod storage_paths;
fn shared_media_source(server: &Value, manager: &Value, media: &str) -> Result<String> {
    storage_paths::evidence(server, manager, media)
}
async fn evidence(state: &AppState, container: &str, port: u16) -> Result<(String, String)> {
    evidence_for(state, container, port, true, true).await
}
async fn evidence_for(
    state: &AppState,
    container: &str,
    port: u16,
    needs_media: bool,
    named_host: bool,
) -> Result<(String, String)> {
    if !(12..=64).contains(&container.len())
        || !container.bytes().all(|b| b.is_ascii_hexdigit())
        || port == 0
    {
        return Err(ApiError::bad(
            "Select a local Docker container and its internal API port",
        ));
    }
    let own = std::env::var("HOSTNAME").unwrap_or_default();
    #[cfg(test)]
    let own = if state
        .managers
        .docker
        .lock()
        .unwrap()
        .contains_key("containers/self")
    {
        "self".into()
    } else {
        own
    };
    // Docker names a container's hostname after its ID unless Compose overrides it.
    #[cfg(not(test))]
    if !(12..=64).contains(&own.len()) || !own.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::conflict(
            "Thelxinoe can't identify its own container because the server's hostname was customized; remove the hostname setting from the server container",
        ));
    }
    let server = docker(state, &format!("containers/{own}")).await?;
    let manager = docker(state, &format!("containers/{container}")).await?;
    if manager["id"].as_str().is_none_or(|id| id != container) {
        return Err(unavailable());
    }
    let name = manager["name"]
        .as_str()
        .map_or(container, |name| name.trim_start_matches('/'));
    let networks = server["networks"].as_array().ok_or_else(unavailable)?;
    let shared = manager["networks"]
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .filter(|n| {
            networks
                .iter()
                .any(|s| s["id"].as_str().is_some_and(|id| !id.is_empty()) && s["id"] == n["id"])
        })
        .collect::<Vec<_>>();
    if shared.is_empty() {
        return Err(failures::isolated(state, &server, &manager, name).await);
    }
    if (named_host || manager["running"] == false) && shared.iter().all(|n| n["name"] == "bridge") {
        return Err(failures::default_bridge(&server, name));
    }
    let address = shared.iter().find_map(|n| {
        n["address"]
            .as_str()
            .and_then(|a| a.parse::<std::net::Ipv4Addr>().ok())
            .filter(|ip| ip.is_private() || ip.is_loopback())
            .map(|ip| ip.to_string())
    });
    let route = if named_host || manager["running"] == false {
        format!("http://{}:{port}", support::docker_host(&manager)?)
    } else if let Some(address) = address {
        format!("http://{address}:{port}")
    } else {
        let address = shared.iter().find_map(|n| {
            n["ipv6"]
                .as_str()
                .and_then(|a| a.parse::<std::net::Ipv6Addr>().ok())
                .filter(|a| a.is_unique_local() || a.is_loopback())
        });
        format!("http://[{}]:{port}", address.ok_or_else(unavailable)?)
    };
    Ok((
        route,
        if needs_media {
            shared_media_source(&server, &manager, &state.config.media.to_string_lossy())?
        } else {
            String::new()
        },
    ))
}
#[derive(Clone)]
struct Service {
    id: String,
    name: String,
    kind: String,
    container: String,
    port: u16,
    url_base: String,
    access_revision: String,
    generation: String,
    credential: Vec<u8>,
    media_source: String,
    defaults: Value,
}
async fn service(state: &AppState, id: &str) -> Result<Service> {
    let id = id.to_owned();
    storage::service(id, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)
}
struct Connection<'a> {
    state: &'a AppState,
    base: String,
    url_base: String,
    key: String,
    kind: String,
}
impl Connection<'_> {
    async fn open<'a>(state: &'a AppState, s: &Service) -> Result<Connection<'a>> {
        let (base, source) = match evidence(state, &s.container, s.port).await {
            Err(error) if error.1 == "container_missing" => {
                let container = recreation::follow(state, &s.id, &s.container).await?;
                evidence(state, &container, s.port).await?
            }
            evidence => evidence?,
        };
        if source != s.media_source {
            return Err(ApiError::conflict(format!(
                "{}'s storage mounts changed since it was connected. Save it again in Edit connection to accept the new layout",
                failures::label(&s.kind)
            )));
        }
        let key = String::from_utf8(
            state
                .secrets
                .decrypt(&format!("manager:{}", s.id), &s.credential)?,
        )
        .map_err(|_| unavailable())?;
        Ok(Connection {
            state,
            base,
            url_base: s.url_base.clone(),
            key,
            kind: s.kind.clone(),
        })
    }
    fn version(&self) -> u8 {
        if matches!(self.kind.as_str(), "lidarr" | "prowlarr" | "seerr") {
            1
        } else {
            3
        }
    }
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let endpoint = if self.kind == "bazarr" {
            format!("{}/api/{path}", self.url_base)
        } else {
            format!("{}/api/v{}/{path}", self.url_base, self.version())
        };
        let mut request = self
            .state
            .managers
            .http
            .request(method, format!("{}{endpoint}", self.base))
            .header("X-Api-Key", &self.key)
            .query(query);
        if let Some(body) = body {
            request = request.json(&body)
        }
        let response = request
            .send()
            .await
            .map_err(|error| failures::unreachable(&self.kind, &self.base, &error))?;
        if !response.status().is_success() {
            return Err(failures::rejected(&self.kind, &endpoint, response.status()));
        }
        if response.status() == reqwest::StatusCode::NO_CONTENT
            || response.content_length() == Some(0)
        {
            return Ok(Value::Null);
        }
        read(response).await
    }
    async fn get(&self, path: &str) -> Result<Value> {
        self.call(reqwest::Method::GET, path, &[], None).await
    }
}
async fn containers(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(docker(&state, "containers").await?))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
struct Register {
    name: String,
    kind: String,
    container_id: String,
    port: u16,
    api_key: String,
    #[serde(default)]
    url_base: String,
    /// A stopped container can only be registered after the administrator accepts that.
    #[serde(default)]
    allow_unverified: bool,
    #[serde(skip)]
    container_name: String,
}
async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Register>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    register_with_actor(state, input, p.user.id).await
}
async fn register_with_actor(
    state: AppState,
    mut input: Register,
    actor_id: String,
) -> Result<Json<Value>> {
    if !matches!(input.kind.as_str(), "radarr" | "sonarr" | "lidarr") {
        return Err(ApiError::bad("Choose Radarr, Sonarr or Lidarr"));
    }
    let service = failures::label(&input.kind);
    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(ApiError::bad("Enter a name of up to 100 characters"));
    }
    if !(16..=256).contains(&input.api_key.len()) || input.api_key.chars().any(char::is_control) {
        return Err(ApiError::bad(format!(
            "Enter the API key from {service}'s Settings → General"
        )));
    }
    let _guard = state.managers.guard.service(&input.kind).await;
    access::validate_base(&input.kind, &input.url_base)?;
    let (base, media_source) = evidence(&state, &input.container_id, input.port).await?;
    let connection = Connection {
        state: &state,
        base,
        url_base: input.url_base.clone(),
        key: input.api_key.clone(),
        kind: input.kind.clone(),
    };
    let observed = docker(&state, &format!("containers/{}", input.container_id)).await?;
    input.container_name = observed["name"]
        .as_str()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_owned();
    let status = if observed["running"] == false {
        if !input.allow_unverified {
            return Err(failures::stopped(&input.container_name));
        }
        json!({"appName":input.kind,"version":"Unverified", "urlBase":input.url_base})
    } else {
        connection.get("system/status").await?
    };
    access::check_reported_base(&input.kind, &input.url_base, &status)?;
    let version = status["version"]
        .as_str()
        .filter(|v| v.len() < 100)
        .ok_or_else(|| failures::identity(&input.kind, &status["appName"]))?
        .to_owned();
    let kind = input.kind.clone();
    let existing = storage::register_with_actor_read_manager_services(kind, &state.db).await?;
    let key = existing.unwrap_or_else(id);
    let returned = key.clone();
    let credential = state
        .secrets
        .encrypt(&format!("manager:{key}"), input.api_key.as_bytes())?;
    let allowed = storage::register_with_actor_write_stack_provisions(
        media_source,
        version,
        key,
        credential,
        &state.db,
        input,
        actor_id,
    )
    .await?;
    if !allowed {
        return Err(ApiError::conflict(format!(
            "Thelxinoe is installing or managing another {service} container; finish or remove that installation first"
        )));
    }
    Ok(Json(json!({"id":returned})))
}
async fn options(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let s = service(&state, &id).await?;
    let c = Connection::open(&state, &s).await?;
    let roots = c.get("rootfolder").await?;
    let profiles = c.get("qualityprofile").await?;
    let guides = recyclarr::profiles(&state, &id).await?;
    let metadata = if s.kind == "lidarr" {
        c.get("metadataprofile").await?
    } else {
        json!([])
    };
    // Return only configuration choices, never whole manager settings or credentials.
    let summarize = |rows: Value, root: bool| {
        rows.as_array()
            .map(|v| {
                v.iter()
                    .map(|r| {
                        if root {
                            json!({"id":r["id"],"path":r["path"]})
                        } else {
                            let guide=guides.iter().find(|p|r["id"].as_i64()==Some(p.profile_id));
                            json!({"id":r["id"],"name":r["name"],"trash_id":guide.map(|p|&p.trash_id),"url":guide.and_then(|p|p.url.as_ref())})
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Ok(Json(
        json!({"roots":summarize(roots,true),"profiles":summarize(profiles,false),"metadata_profiles":summarize(metadata,false),"defaults":service(&state,&id).await?.defaults}),
    ))
}
#[derive(Deserialize, Serialize, Clone)]
struct Defaults {
    root_folder: String,
    quality_profile: i64,
    #[serde(default)]
    quality_profile_trash_id: Option<String>,
    metadata_profile: Option<i64>,
    #[serde(default = "yes")]
    monitored: bool,
}
fn yes() -> bool {
    true
}
async fn prepare_library(state: &AppState, s: &Service) -> Result<()> {
    let c = Connection::open(state, s).await?;
    let roots = c.get("rootfolder").await?;
    let root = canonical_root(&s.kind);
    if roots
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .any(|r| r["path"] == root)
    {
        return Ok(());
    }
    let parent = tokio::fs::canonicalize(&state.config.media)
        .await
        .map_err(|_| ApiError::conflict("Media storage is unavailable"))?;
    let path = std::path::PathBuf::from(root);
    if !path.starts_with(&parent) {
        return Err(ApiError::conflict("Manager root is outside media storage"));
    }
    if tokio::fs::symlink_metadata(&path)
        .await
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(ApiError::conflict(
            "Canonical media roots cannot be symbolic links",
        ));
    }
    tokio::fs::create_dir_all(&path).await.map_err(|_| {
        ApiError::conflict("The server needs permission to create the canonical library directory")
    })?;
    let mut body = json!({"path":root});
    if s.kind == "lidarr" {
        // Lidarr requires profiles on its root record even before any artist is added.
        let profiles = c.get("qualityprofile").await?;
        let metadata = c.get("metadataprofile").await?;
        let quality = profiles[0]["id"].as_i64().ok_or_else(unavailable)?;
        let metadata = metadata[0]["id"].as_i64().ok_or_else(unavailable)?;
        body = json!({"path":root,"name":"Thelxinoe music","defaultQualityProfileId":quality,
            "defaultMetadataProfileId":metadata,"defaultMonitorOption":"none","defaultTags":[]});
    }
    c.call(reqwest::Method::POST, "rootfolder", &[], Some(body))
        .await?;
    Ok(())
}
async fn defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut input): Json<Defaults>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let kind = service(&state, &id).await?.kind;
    let _guard = state.managers.guard.service(&kind).await;
    let s = service(&state, &id).await?;
    input.quality_profile_trash_id = recyclarr::profiles(&state, &id)
        .await?
        .into_iter()
        .find(|p| p.profile_id == input.quality_profile)
        .map(|p| p.trash_id);
    let c = Connection::open(&state, &s).await?;
    if !clean_path(&input.root_folder)
        || !c.get("qualityprofile").await?.as_array().is_some_and(|a| {
            a.iter()
                .any(|r| r["id"].as_i64() == Some(input.quality_profile))
        })
    {
        return Err(ApiError::bad(
            "Choose an existing library folder and a valid quality profile",
        ));
    }
    if s.kind == "lidarr"
        && !c
            .get("metadataprofile")
            .await?
            .as_array()
            .is_some_and(|a| a.iter().any(|r| r["id"].as_i64() == input.metadata_profile))
    {
        return Err(ApiError::bad("Choose an existing metadata profile"));
    }
    if !c
        .get("rootfolder")
        .await?
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["path"] == input.root_folder)
    {
        if input.root_folder != canonical_root(&s.kind) || !installed_here(&state, &s.id).await? {
            return Err(ApiError::bad("Choose an existing manager root folder"));
        }
        prepare_library(&state, &s).await?;
    }
    storage::defaults(&state.db, id, input, p).await?;
    state.managers.connection_wake.notify_one();
    Ok(Json(json!({"saved":true})))
}
async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let s = service(&state, &id).await?;
    let result = async {
        let c = Connection::open(&state, &s).await?;
        let v = c.get("system/status").await?;
        if !v["appName"]
            .as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(&s.kind))
        {
            return Err(failures::identity(&s.kind, &v["appName"]));
        }
        Ok::<_, ApiError>(v["version"].clone())
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    storage::test(&state.db, id, error).await?;
    state.notify_attention();
    Ok(Json(json!({"healthy":true,"version":result?})))
}

fn canonical_root(kind: &str) -> &'static str {
    match kind {
        "radarr" => "/media/movies",
        "sonarr" => "/media/tv",
        _ => "/media/music",
    }
}
async fn installed_here(state: &AppState, key: &str) -> Result<bool> {
    let key = key.to_owned();
    Ok(storage::installed_here(key, &state.db).await?)
}
