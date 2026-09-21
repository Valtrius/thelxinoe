//! Local Docker manager adapters. Docker evidence stays behind the private controller socket.
mod bindings;
mod controls;
mod metadata;
mod operations;
mod requests;
mod retention;
mod stack;
mod support;
pub(crate) use support::operational_health;
mod updates;
pub(crate) use retention::run as run_retention;
pub(crate) use stack::controller as controller_request;
pub(crate) use stack::provision;
pub(crate) use updates::{run as run_updates, run_job as update_service};
#[cfg(test)]
mod tests;
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
use thelxinoe_core::{Capability, id, now};
pub(crate) struct Runtime {
    http: reqwest::Client,
    guard: tokio::sync::Mutex<()>,
    #[cfg(test)]
    docker: std::sync::Mutex<std::collections::HashMap<String, Value>>,
}
impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            guard: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            docker: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }
}
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .merge(requests::router())
        .merge(bindings::router())
        .merge(controls::router())
        .merge(metadata::router())
        .merge(operations::router())
        .merge(support::router())
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
    ApiError::conflict(
        "Manager or its Docker evidence is unavailable; check the service and controller",
    )
}
async fn docker(state: &AppState, path: &str) -> Result<Value> {
    #[cfg(test)]
    if let Some(v) = state.managers.docker.lock().unwrap().get(path) {
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
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
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
    if bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
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
fn media_source(container: &Value) -> Result<String> {
    let mounts = container["mounts"].as_array().ok_or_else(unavailable)?;
    if mounts.iter().any(|m| {
        m["destination"]
            .as_str()
            .is_some_and(|p| suffix(p, "/media").is_some_and(|tail| !tail.is_empty()))
    }) {
        return Err(ApiError::conflict(
            "Mount one shared host directory at /media; child mounts such as /media/movies are not supported",
        ));
    }
    let roots = mounts
        .iter()
        .filter(|m| m["destination"] == "/media")
        .collect::<Vec<_>>();
    if roots.len() != 1 || roots[0]["kind"] != "bind" || roots[0]["writable"] != true {
        return Err(ApiError::conflict(
            "Every media service needs one writable bind mount at /media",
        ));
    }
    roots[0]["source"]
        .as_str()
        .and_then(host_path)
        .ok_or_else(unavailable)
}
fn shared_media_source(server: &Value, manager: &Value, media: &str) -> Result<String> {
    if media != "/media" {
        return Err(ApiError::conflict(
            "Docker integrations require the server media root /media",
        ));
    }
    let source = media_source(server)?;
    if source != media_source(manager)? {
        return Err(ApiError::conflict(
            "Mount the same host media directory at /media in Thelxinoe and this service",
        ));
    }
    Ok(source)
}
async fn evidence(state: &AppState, container: &str, port: u16) -> Result<(String, String)> {
    evidence_for(state, container, port, true).await
}
async fn evidence_for(
    state: &AppState,
    container: &str,
    port: u16,
    needs_media: bool,
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
    let server = docker(state, &format!("containers/{own}")).await?;
    let manager = docker(state, &format!("containers/{container}")).await?;
    if manager["running"] != true || !manager["id"].as_str().is_some_and(|id| id == container) {
        return Err(unavailable());
    }
    let networks = server["networks"].as_array().ok_or_else(unavailable)?;
    let address = manager["networks"]
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .filter(|n| {
            networks
                .iter()
                .any(|s| s["id"].is_string() && s["id"] == n["id"])
        })
        .find_map(|n| {
            n["address"]
                .as_str()
                .and_then(|a| a.parse::<std::net::Ipv4Addr>().ok())
                .filter(|ip| ip.is_private() || ip.is_loopback())
        })
        .ok_or_else(|| {
            ApiError::conflict("Manager must share a Docker network with this server")
        })?;
    Ok((
        format!("http://{address}:{port}"),
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
    generation: String,
    credential: Vec<u8>,
    media_source: String,
    defaults: Value,
}
async fn service(state: &AppState, id: &str) -> Result<Service> {
    let id = id.to_owned();
    state.db.call(move|db|Ok(db.query_row("SELECT id,name,kind,container_id,port,generation,credential,media_source,defaults FROM manager_services WHERE id=?1 AND enabled=1",[id],|r|Ok(Service{id:r.get(0)?,name:r.get(1)?,kind:r.get(2)?,container:r.get(3)?,port:r.get(4)?,generation:r.get(5)?,credential:r.get(6)?,media_source:r.get(7)?,defaults:serde_json::from_str(&r.get::<_,String>(8)?).unwrap_or(Value::Null)})).optional()?)).await?.ok_or_else(ApiError::not_found)
}
struct Connection<'a> {
    state: &'a AppState,
    base: String,
    key: String,
    kind: String,
}
impl Connection<'_> {
    async fn open<'a>(state: &'a AppState, s: &Service) -> Result<Connection<'a>> {
        let (base, source) = evidence(state, &s.container, s.port).await?;
        if source != s.media_source {
            return Err(ApiError::conflict(
                "Manager media mount changed; reconnect the service before continuing",
            ));
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
            key,
            kind: s.kind.clone(),
        })
    }
    fn version(&self) -> u8 {
        if matches!(self.kind.as_str(), "lidarr" | "prowlarr") {
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
        let mut request = self
            .state
            .managers
            .http
            .request(
                method,
                if self.kind == "bazarr" {
                    format!("{}/api/{path}", self.base)
                } else {
                    format!("{}/api/v{}/{path}", self.base, self.version())
                },
            )
            .header("X-Api-Key", &self.key)
            .query(query);
        if let Some(body) = body {
            request = request.json(&body)
        }
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(ApiError::conflict(format!(
                "Manager rejected the request (HTTP {})",
                response.status().as_u16()
            )));
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
    let rows=state.db.call(|db|Ok(db.prepare("SELECT id,name,kind,container_id,port,version,defaults,checked_at,error FROM manager_services ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"container_id":r.get::<_,String>(3)?,"port":r.get::<_,u16>(4)?,"version":r.get::<_,String>(5)?,"defaults":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or(Value::Null),"checked_at":r.get::<_,i64>(7)?,"error":r.get::<_,Option<String>>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
struct Register {
    name: String,
    kind: String,
    container_id: String,
    port: u16,
    api_key: String,
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
    input: Register,
    actor_id: String,
) -> Result<Json<Value>> {
    if !matches!(input.kind.as_str(), "radarr" | "sonarr" | "lidarr")
        || input.name.trim().is_empty()
        || input.name.len() > 100
        || !(16..=256).contains(&input.api_key.len())
        || input.api_key.chars().any(char::is_control)
    {
        return Err(ApiError::bad("Enter a manager type, name and API key"));
    }
    let _guard = state.managers.guard.lock().await;
    let (base, media_source) = evidence(&state, &input.container_id, input.port).await?;
    let connection = Connection {
        state: &state,
        base,
        key: input.api_key.clone(),
        kind: input.kind.clone(),
    };
    let status = connection.get("system/status").await?;
    if !status["appName"]
        .as_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(&input.kind))
    {
        return Err(ApiError::bad(
            "The selected container is not the requested manager",
        ));
    }
    validate_roots(&input.kind, &connection.get("rootfolder").await?)?;
    let version = status["version"]
        .as_str()
        .filter(|v| v.len() < 100)
        .ok_or_else(unavailable)?
        .to_owned();
    let container = input.container_id.clone();
    let existing = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT id FROM manager_services WHERE container_id=?1",
                    [container],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?;
    let key = existing.unwrap_or_else(id);
    let returned = key.clone();
    let credential = state
        .secrets
        .encrypt(&format!("manager:{key}"), input.api_key.as_bytes())?;
    state.db.call(move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,port=excluded.port,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,version=excluded.version,checked_at=excluded.checked_at,error=NULL",params![key,input.name.trim(),input.kind,input.container_id,input.port,id(),credential,media_source,version,now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'manager.register',?2,?3)",params![actor_id,key,now()])?;tx.commit()?;Ok(())}).await?;
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
    let mut roots = c.get("rootfolder").await?;
    validate_roots(&s.kind, &roots)?;
    if installed_here(&state, &s.id).await? {
        let path = canonical_root(&s.kind);
        let rows = roots.as_array_mut().ok_or_else(unavailable)?;
        if !rows.iter().any(|r| r["path"] == path) {
            rows.push(json!({"id":0,"path":path}));
        }
    }
    let profiles = c.get("qualityprofile").await?;
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
                            json!({"id":r["id"],"name":r["name"]})
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Ok(Json(
        json!({"roots":summarize(roots,true),"profiles":summarize(profiles,false),"metadata_profiles":summarize(metadata,false)}),
    ))
}
#[derive(Deserialize, Serialize, Clone)]
struct Defaults {
    root_folder: String,
    quality_profile: i64,
    metadata_profile: Option<i64>,
    #[serde(default = "yes")]
    monitored: bool,
}
fn yes() -> bool {
    true
}
async fn defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Defaults>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.managers.guard.lock().await;
    let s = service(&state, &id).await?;
    let c = Connection::open(&state, &s).await?;
    if input.root_folder != canonical_root(&s.kind)
        || !c.get("qualityprofile").await?.as_array().is_some_and(|a| {
            a.iter()
                .any(|r| r["id"].as_i64() == Some(input.quality_profile))
        })
    {
        return Err(ApiError::bad(
            "Choose the canonical library folder and a valid quality profile",
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
        if !installed_here(&state, &s.id).await? {
            return Err(ApiError::bad("Choose an existing manager root folder"));
        }
        let parent = tokio::fs::canonicalize(&state.config.media)
            .await
            .map_err(|_| ApiError::conflict("Media storage is unavailable"))?;
        let path = std::path::PathBuf::from(&input.root_folder);
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
            ApiError::conflict(
                "The server needs permission to create the canonical library directory",
            )
        })?;
        c.call(reqwest::Method::POST,"rootfolder",&[],Some(json!({"path":input.root_folder,"name":"Thelxinoe music","defaultQualityProfileId":input.quality_profile,"defaultMetadataProfileId":input.metadata_profile,"defaultMonitorOption":"none","defaultTags":[]}))).await?;
    }
    state.db.call(move|db|{let tx=db.transaction()?;tx.execute("UPDATE manager_services SET defaults=?1,generation=?3 WHERE id=?2",params![serde_json::to_string(&input)?,id,thelxinoe_core::id()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'manager.defaults',?2,?3)",params![p.user.id,id,now()])?;tx.commit()?;Ok(())}).await?;
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
            return Err(unavailable());
        }
        Ok::<_, ApiError>(v["version"].clone())
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE manager_services SET checked_at=?1,error=?2 WHERE id=?3",
                params![now(), error, id],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"healthy":true,"version":result?})))
}

fn validate_roots(kind: &str, roots: &Value) -> Result<()> {
    if roots
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .any(|r| r["path"] != canonical_root(kind))
    {
        return Err(ApiError::conflict(format!(
            "Configure this service's library root as {} and update existing library paths in its bulk editor",
            canonical_root(kind)
        )));
    }
    Ok(())
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
    Ok(state.db.call(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=?1 AND origin='installed')",[key],|r|r.get::<_,bool>(0))?)).await?)
}
