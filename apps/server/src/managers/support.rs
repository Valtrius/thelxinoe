//! Narrow admin adapters for subtitle, indexer and download services.

#[path = "../storage/managers/support.rs"]
mod storage;

use super::*;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/support", get(list).post(register))
        .route("/api/v1/admin/support/{id}", get(inspect).post(action))
        .route("/api/v1/admin/support/{id}/login", post(login))
}
#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Credentials {
    #[serde(default)]
    pub(super) username: String,
    pub(super) secret: String,
}
pub(super) struct Support {
    id: String,
    kind: String,
    pub(super) container: String,
    port: u16,
    pub(super) url_base: String,
    pub(super) access_revision: String,
    media_source: String,
    generation: String,
    scope: ownership::Scope,
    pub(super) credentials: Credentials,
}
/// An operational Owned support service, or the service a running setup or transfer coordinates.
pub(super) async fn load(state: &AppState, key: &str) -> Result<Support> {
    let mut service = load_any(state, key).await?;
    match ownership::authority_for(key) {
        Some(scope) => service.scope = scope,
        None if service.scope != ownership::Scope::Operational => {
            return Err(ownership::held(&service.kind));
        }
        None => {}
    }
    Ok(service)
}
/// Any saved support connection, for read-only health and identity validation.
pub(super) async fn load_connected(state: &AppState, key: &str) -> Result<Support> {
    let mut service = load_any(state, key).await?;
    service.scope = ownership::Scope::Validation;
    Ok(service)
}
async fn load_any(state: &AppState, key: &str) -> Result<Support> {
    let row = storage::load(key.to_owned(), &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let credentials = serde_json::from_slice(
        &state
            .secrets
            .decrypt(&format!("support:{}", row.id), &row.credential)?,
    )
    .map_err(|_| unavailable())?;
    Ok(Support {
        id: row.id,
        kind: row.kind,
        container: row.container,
        port: row.port,
        url_base: row.url_base,
        access_revision: row.access_revision,
        media_source: row.media_source,
        generation: row.generation,
        scope: if row.operational {
            ownership::Scope::Operational
        } else {
            ownership::Scope::Validation
        },
        credentials,
    })
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let service = load(&state, &key).await?;
    if service.kind != "nzbget" {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({
        "username": service.credentials.username,
        "password": service.credentials.secret,
    })))
}
/// Read-only: Connected services may be checked before a transfer stops them.
pub(super) async fn ensure_idle(state: &AppState, key: &str) -> Result<()> {
    let s = load_connected(state, key).await?;
    let c = connect(state, &s).await?;
    let idle = match s.kind.as_str() {
        "nzbget" => {
            let status = rpc(&c, &s.credentials, "status", json!([])).await?;
            status["DownloadRate"].as_u64() == Some(0)
                && status["PostJobCount"].as_u64() == Some(0)
                && rpc(&c, &s.credentials, "listgroups", json!([0]))
                    .await?
                    .as_array()
                    .is_some_and(Vec::is_empty)
        }
        "prowlarr" => c.get("command").await?.as_array().is_some_and(|commands| {
            commands.iter().all(|v| {
                matches!(
                    v["status"].as_str(),
                    Some("completed" | "failed" | "aborted" | "cancelled")
                )
            })
        }),
        "seerr" => c
            .get("settings/jobs")
            .await?
            .as_array()
            .is_some_and(|jobs| jobs.iter().all(|job| job["running"] == false)),
        "bazarr" => c.get("system/tasks").await?["data"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().all(|v| v["job_running"] == false)),
        _ => false,
    };
    if !idle {
        return Err(ApiError::conflict(
            "Service is busy or its idle state is unknown",
        ));
    }
    Ok(())
}
pub(super) async fn connect<'a>(state: &'a AppState, s: &Support) -> Result<Connection<'a>> {
    let (base, media_source) = match api_evidence(state, &s.container, s.port, &s.kind).await {
        Err(error) if error.1 == "container_missing" => {
            let container = recreation::follow(state, &s.id, &s.container).await?;
            api_evidence(state, &container, s.port, &s.kind).await?
        }
        evidence => evidence?,
    };
    if media_source != s.media_source {
        return Err(ApiError::conflict(format!(
            "{}'s storage mounts changed since it was connected. Save it again in Edit connection to accept the new layout",
            failures::label(&s.kind)
        )));
    }
    Ok(Connection {
        state,
        base,
        url_base: s.url_base.clone(),
        key: s.credentials.secret.clone(),
        kind: s.kind.clone(),
        grant: ownership::Grant {
            scope: s.scope.clone(),
            service: s.id.clone(),
            generation: s.generation.clone(),
            container: s.container.clone(),
        },
    })
}
pub(super) async fn api_evidence(
    state: &AppState,
    container: &str,
    port: u16,
    kind: &str,
) -> Result<(String, String)> {
    evidence_for(
        state,
        container,
        port,
        !matches!(kind, "prowlarr" | "seerr"),
        thelxinoe_core::service_gateway_auth(kind),
    )
    .await
}
pub(super) fn docker_host(observed: &Value) -> Result<&str> {
    let name = observed["name"]
        .as_str()
        .ok_or_else(unavailable)?
        .trim_start_matches('/');
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(unavailable());
    }
    Ok(name)
}
pub(super) async fn rpc(
    c: &Connection<'_>,
    credentials: &Credentials,
    method: &str,
    params: Value,
) -> Result<Value> {
    // JSON-RPC uses POST for reads too; only the method decides whether NZBGet changes.
    if !ownership::nzbget_read_only(method) {
        c.authorize().await?;
    }
    let response = c
        .state
        .managers
        .http
        .post(format!("{}/jsonrpc", c.base))
        .basic_auth(&credentials.username, Some(&credentials.secret))
        .json(&json!({"method":method,"params":params,"id":1}))
        .send()
        .await
        .map_err(|error| failures::unreachable("nzbget", &c.base, &error))?;
    if !response.status().is_success() {
        return Err(failures::rejected("nzbget", "/jsonrpc", response.status()));
    }
    let result = read(response).await?;
    if !result["error"].is_null() {
        return Err(ApiError::conflict(format!(
            "NZBGet rejected the {method} command"
        )));
    }
    result.get("result").cloned().ok_or_else(unavailable)
}
pub(super) async fn version(c: &Connection<'_>, credentials: &Credentials) -> Result<String> {
    let value = match c.kind.as_str() {
        "nzbget" => rpc(c, credentials, "version", json!([])).await?,
        "bazarr" => c.get("system/status").await?["data"]["bazarr_version"].clone(),
        "seerr" => c.get("status").await?["version"].clone(),
        _ => {
            let value = c.get("system/status").await?;
            if value["appName"] != "Prowlarr" {
                return Err(failures::identity(&c.kind, &value["appName"]));
            }
            value["version"].clone()
        }
    };
    value
        .as_str()
        .filter(|v| !v.is_empty() && v.len() < 100)
        .map(str::to_owned)
        .ok_or_else(|| failures::identity(&c.kind, &Value::Null))
}
/// Owned NZBGet always has a control password, as fresh installations do. A missing one is
/// generated and recorded before NZBGet applies it: NZBGet without a password accepts any.
pub(super) async fn ensure_password(state: &AppState, key: &str) -> Result<Value> {
    let mut s = load(state, key).await?;
    if s.kind != "nzbget" || !super::owned_here(state, key).await? {
        return Ok(json!({"changed":false}));
    }
    let c = connect(state, &s).await?;
    let mutex = super::connections::lock(state, &format!("nzbget-config:{key}"));
    let _guard = mutex.lock().await;
    let mut config = rpc(&c, &s.credentials, "loadconfig", json!([])).await?;
    let rows = config
        .as_array_mut()
        .ok_or_else(|| ApiError::conflict("NZBGet configuration is unavailable"))?;
    let value = |rows: &[Value], name: &str| {
        rows.iter()
            .rev()
            .find(|r| {
                r["Name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|r| r["Value"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    if !value(rows, "ControlPassword").is_empty() {
        return Ok(json!({"changed":false}));
    }
    if s.credentials.secret.is_empty() {
        let username = value(rows, "ControlUsername");
        s.credentials = Credentials {
            username: if username.is_empty() {
                "thelxinoe".into()
            } else {
                username
            },
            secret: thelxinoe_auth::token(),
        };
        let encrypted = state.secrets.encrypt(
            &format!("support:{key}"),
            &serde_json::to_vec(&s.credentials).map_err(|_| unavailable())?,
        )?;
        if !storage::replace_credential(
            &state.db,
            key.to_owned(),
            s.container.clone(),
            s.access_revision.clone(),
            encrypted,
        )
        .await?
        {
            return Err(ApiError::conflict(
                "NZBGet's connection changed while Thelxinoe secured it; it will retry",
            ));
        }
    }
    for (name, setting) in [
        ("ControlUsername", s.credentials.username.clone()),
        ("ControlPassword", s.credentials.secret.clone()),
    ] {
        rows.retain(|r| {
            !r["Name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        });
        rows.push(json!({"Name":name,"Value":setting}));
    }
    if rpc(&c, &s.credentials, "saveconfig", json!([config])).await? != true {
        return Err(ApiError::conflict(
            "NZBGet did not save its control password",
        ));
    }
    rpc(&c, &s.credentials, "reload", json!([])).await?;
    let ready = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if version(&c, &s.credentials).await.is_ok() {
                return;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(40), ready)
        .await
        .map_err(|_| ApiError::conflict("NZBGet is restarting with its new control password"))?;
    // Linked Radarr, Sonarr and Lidarr clients receive the new credentials.
    state.managers.connection_wake.notify_one();
    Ok(json!({"changed":true,"username":s.credentials.username}))
}
pub(super) fn native_url(value: &str) -> bool {
    value.is_empty()
        || url::Url::parse(value).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
        })
}
#[derive(Deserialize)]
struct Register {
    name: String,
    kind: String,
    container_id: String,
    port: u16,
    credentials: Credentials,
    #[serde(default)]
    native_url: String,
    #[serde(default)]
    url_base: String,
    /// A stopped container can only be registered after the administrator accepts that.
    #[serde(default)]
    allow_unverified: bool,
    #[serde(skip)]
    container_name: String,
    /// Set only by an installation registering its own new container under its setup hold.
    #[serde(skip)]
    setup: Option<String>,
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
    if !matches!(
        input.kind.as_str(),
        "bazarr" | "prowlarr" | "nzbget" | "seerr"
    ) {
        return Err(ApiError::bad("Choose Bazarr, Prowlarr, NZBGet or Seerr"));
    }
    let service = failures::label(&input.kind);
    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(ApiError::bad("Enter a name of up to 100 characters"));
    }
    if input.kind != "nzbget" && input.credentials.secret.is_empty() {
        return Err(ApiError::bad(format!(
            "Enter the API key from {service}'s Settings → General"
        )));
    }
    if input.credentials.secret.len() > 1024 || input.credentials.username.len() > 100 {
        return Err(ApiError::bad(
            "Credentials are limited to a 100-character username and a 1024-character secret",
        ));
    }
    if input.native_url.len() > 2000 || !native_url(&input.native_url) {
        return Err(ApiError::bad(
            "Enter the UI address as an http(s) URL without a username, password, query or fragment",
        ));
    }
    let _guard = state.managers.guard.service(&input.kind).await;
    let endpoint = inspect_endpoint(
        &state,
        &input.kind,
        &input.container_id,
        input.port,
        &input.url_base,
        &input.credentials,
        input.allow_unverified,
    )
    .await?;
    input.container_name = endpoint.container_name.clone();
    let kind = input.kind.clone();
    let key = storage::register_with_actor_read_support_services(kind, &state.db)
        .await?
        .unwrap_or_else(id);
    let returned = key.clone();
    let secret = state.secrets.encrypt(
        &format!("support:{key}"),
        &serde_json::to_vec(&input.credentials).map_err(|_| unavailable())?,
    )?;
    let registration = endpoint.registration(key, secret, input.setup.clone());
    let allowed = storage::register_with_actor_write_stack_provisions(
        &state.db,
        registration,
        input,
        actor_id,
    )
    .await?;
    if !allowed {
        return Err(ApiError::conflict(format!(
            "Thelxinoe is installing, transferring or managing another {service}; finish or remove that installation first"
        )));
    }
    Ok(Json(json!({"id":returned})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
/// Read-only health and identity check of a saved connection, in any stage.
pub(super) async fn test(state: &AppState, key: &str) -> Result<Value> {
    let s = load_connected(state, key).await?;
    let result = async {
        let c = connect(state, &s).await?;
        access::verify_endpoint(&c, Some(&s.credentials)).await
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    storage::test(
        &state.db,
        key.to_owned(),
        s.container,
        result.as_ref().ok().cloned(),
        error,
    )
    .await?;
    state.notify_attention();
    Ok(json!({"healthy":true,"version":result?}))
}
fn text(value: &Value, secret: &str) -> String {
    let value = value.as_str().unwrap_or("");
    let value = if secret.is_empty() {
        value.to_owned()
    } else {
        value.replace(secret, "[redacted]")
    };
    value
        .split_whitespace()
        .map(|w| {
            if w.contains("://") || w.contains("apikey=") || w.contains("token=") {
                "[service URL]"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}
async fn wanted(c: &Connection<'_>, domain: &str) -> Result<Value> {
    c.call(
        reqwest::Method::GET,
        &format!("{domain}/wanted"),
        &[("start", "0".into()), ("length", "100".into())],
        None,
    )
    .await
}
/// The service's live state; `history` adds NZBGet's history, which health checks don't use.
async fn snapshot(c: &Connection<'_>, s: &Support, history: bool) -> Result<Value> {
    match s.kind.as_str() {
        "seerr" => {
            Ok(json!({"initialized":c.get("settings/public").await?["initialized"],"health":[]}))
        }
        "nzbget" => {
            let status = rpc(c, &s.credentials, "status", json!([])).await?;
            let queue = rpc(c, &s.credentials, "listgroups", json!([0])).await?;
            let history = if history {
                rpc(c, &s.credentials, "history", json!([true])).await?
            } else {
                json!([])
            };
            let summarize = |rows: &Value| {
                rows.as_array().into_iter().flatten().take(200).map(|r|json!({"id":r["NZBID"],"title":text(r.get("NZBName").unwrap_or(&r["Name"]),&c.key),"status":r["Status"],"category":r["Category"],"size_mb":r["FileSizeMB"],"remaining_mb":r["RemainingSizeMB"],"paused_mb":r["PausedSizeMB"],"downloaded_mb":r["DownloadedSizeMB"],"history_time":r["HistoryTime"]})).collect::<Vec<_>>()
            };
            Ok(
                json!({"rate":status["DownloadRate"],"limit":status["DownloadLimit"],"paused":status["DownloadPaused"],"remaining_mb":status["RemainingSizeMB"],"free_mb":status["FreeDiskSpaceMB"],"queue":summarize(&queue),"history":summarize(&history)}),
            )
        }
        "prowlarr" => {
            let health = c.get("health").await?;
            let indexers = c.get("indexer").await?;
            let status = c.get("indexerstatus").await?;
            let health=health.as_array().into_iter().flatten().map(|r|json!({"source":r["source"],"type":r["type"],"message":text(&r["message"],&c.key)})).collect::<Vec<_>>();
            let indexers=indexers.as_array().ok_or_else(unavailable)?.iter().map(|r|{let status=status.as_array().into_iter().flatten().find(|v|v["indexerId"]==r["id"]);json!({"id":r["id"],"name":r["name"],"enabled":r["enable"],"protocol":r["protocol"],"disabled_until":status.map(|s|&s["disabledTill"])})}).collect::<Vec<_>>();
            Ok(json!({"health":health,"indexers":indexers}))
        }
        _ => {
            let health = c.get("system/health").await?;
            let movies = wanted(c, "movies").await?;
            let episodes = wanted(c, "episodes").await?;
            let summarize = |rows: &Value| {
                rows["data"].as_array().into_iter().flatten().map(|r|json!({"movie_id":r["radarrId"],"series_id":r["sonarrSeriesId"],"episode_id":r["sonarrEpisodeId"],"title":r.get("title").cloned().unwrap_or_else(||json!(format!("{} - {}",r["seriesTitle"].as_str().unwrap_or("Series"),r["episodeTitle"].as_str().unwrap_or("Episode")))),"missing":r["missing_subtitles"]})).collect::<Vec<_>>()
            };
            let health = health["data"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| text(&r["issue"], &c.key))
                .collect::<Vec<_>>();
            Ok(json!({"health":health,"movies":summarize(&movies),"episodes":summarize(&episodes)}))
        }
    }
}
pub(crate) async fn operational_health(state: &AppState) -> anyhow::Result<()> {
    let rows = storage::operational_health_read_support_services(&state.db).await?;
    let mut items = Vec::new();
    for (key, kind) in rows {
        let result = async {
            let mut service = load(state, &key).await?;
            if kind == "nzbget" && service.credentials.secret.is_empty() {
                match ensure_password(state, &key).await {
                    Ok(_) => service = load(state, &key).await?,
                    Err(error) => {
                        tracing::warn!(error = %error.2, "NZBGet's control password is not set yet");
                    }
                }
            }
            let c = connect(state, &service).await?;
            snapshot(&c, &service, false).await
        }
        .await;
        let value = match result {
            Ok(data) => {
                let problem = kind == "prowlarr"
                    && (data["health"].as_array().is_some_and(|r| !r.is_empty())
                        || data["indexers"].as_array().is_some_and(|rows| {
                            rows.iter().any(|r| {
                                r["disabled_until"]
                                    .as_str()
                                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                    .is_some_and(|date| date.timestamp() > now())
                            })
                        }));
                json!({"id":key,"kind":kind,"problem":problem,"rate":data["rate"],"paused":data["paused"],"remaining_mb":data["remaining_mb"],"queue_count":data["queue"].as_array().map(Vec::len),"health_count":data["health"].as_array().map(Vec::len)})
            }
            Err(_) => json!({"id":key,"kind":kind,"problem":true,"unavailable":true}),
        };
        items.push(value);
    }
    storage::operational_health_write_settings(items, &state.db).await
}
async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let s = load(&state, &key).await?;
    if s.kind == "prowlarr" {
        super::indexers::ensure_hosts(&state, &key).await?;
    }
    let result = async {
        let c = connect(&state, &s).await?;
        snapshot(&c, &s, true).await
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    storage::inspect(&state.db, key, error).await?;
    state.notify_attention();
    Ok(Json(result?))
}
#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    item_id: i64,
    #[serde(default)]
    value: u32,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    forced: bool,
    #[serde(default)]
    hearing_impaired: bool,
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let kind = load(&state, &key).await?.kind;
    let _lease = state.managers.maintenance(&state).await;
    let _guard = state.managers.guard.service(&kind).await;
    let s = load(&state, &key).await?;
    let c = connect(&state, &s).await?;
    match s.kind.as_str() {
        "seerr" => return Err(ApiError::bad("Use the Seerr request controls")),
        "nzbget" => {
            let (method, params) = match input.action.as_str() {
                "pause_all" => ("pausedownload", json!([])),
                "resume_all" => ("resumedownload", json!([])),
                "rate" if input.value <= 1_000_000 => ("rate", json!([input.value])),
                "pause" | "resume" | "remove" if input.item_id > 0 => {
                    let queue = rpc(&c, &s.credentials, "listgroups", json!([0])).await?;
                    if !queue.as_array().is_some_and(|a| {
                        a.iter().any(|r| r["NZBID"].as_i64() == Some(input.item_id))
                    }) {
                        return Err(ApiError::conflict("Download is no longer in the queue"));
                    }
                    (
                        "editqueue",
                        json!([
                            match input.action.as_str() {
                                "pause" => "GroupPause",
                                "resume" => "GroupResume",
                                _ => "GroupDelete",
                            },
                            "",
                            [input.item_id]
                        ]),
                    )
                }
                _ => return Err(ApiError::bad("Unknown download action")),
            };
            if rpc(&c, &s.credentials, method, params).await? != true {
                return Err(ApiError::conflict("Download command was not accepted"));
            }
        }
        "prowlarr" => {
            if input.item_id <= 0 || !matches!(input.action.as_str(), "test" | "enable" | "disable")
            {
                return Err(ApiError::bad("Choose an indexer action"));
            }
            let mut indexer = c.get(&format!("indexer/{}", input.item_id)).await?;
            if input.action == "test" {
                c.call(reqwest::Method::POST, "indexer/test", &[], Some(indexer))
                    .await?;
            } else {
                indexer["enable"] = json!(input.action == "enable");
                c.call(
                    reqwest::Method::PUT,
                    &format!("indexer/{}", input.item_id),
                    &[],
                    Some(indexer),
                )
                .await?;
            }
        }
        _ => {
            if input.action != "subtitles"
                || !matches!(input.domain.as_str(), "movies" | "episodes")
                || input.item_id <= 0
                || !(2..=3).contains(&input.language.len())
                || !input.language.bytes().all(|b| b.is_ascii_lowercase())
            {
                return Err(ApiError::bad(
                    "Select missing subtitles and a language code",
                ));
            }
            let rows = wanted(&c, &input.domain).await?;
            let field = if input.domain == "movies" {
                "radarrId"
            } else {
                "sonarrEpisodeId"
            };
            let row = rows["data"]
                .as_array()
                .ok_or_else(unavailable)?
                .iter()
                .find(|r| r[field].as_i64() == Some(input.item_id))
                .ok_or_else(|| {
                    ApiError::conflict("Media no longer appears in the wanted subtitle list")
                })?;
            let mut query = vec![
                (
                    if input.domain == "movies" {
                        "radarrid"
                    } else {
                        "episodeid"
                    },
                    input.item_id.to_string(),
                ),
                ("language", input.language),
                ("forced", input.forced.to_string()),
                ("hi", input.hearing_impaired.to_string()),
            ];
            if input.domain == "episodes" {
                query.push((
                    "seriesid",
                    row["sonarrSeriesId"]
                        .as_i64()
                        .filter(|v| *v > 0)
                        .ok_or_else(unavailable)?
                        .to_string(),
                ));
            }
            c.call(
                reqwest::Method::PATCH,
                &format!("{}/subtitles", input.domain),
                &query,
                None,
            )
            .await?;
        }
    }
    storage::action(&state.db, input.action, p, s).await?;
    Ok(Json(json!({"accepted":true})))
}

/// Register an installation's own new container under its setup hold.
pub(super) async fn provision(
    state: AppState,
    actor: String,
    setup: &str,
    input: Value,
) -> Result<Json<Value>> {
    let mut input: Register = serde_json::from_value(input)
        .map_err(|_| ApiError::bad("Invalid managed service configuration"))?;
    input.setup = Some(setup.to_owned());
    register_with_actor(state, input, actor).await
}
