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
#[derive(Deserialize, Serialize)]
pub(super) struct Credentials {
    #[serde(default)]
    pub(super) username: String,
    pub(super) secret: String,
}
pub(super) struct Support {
    id: String,
    kind: String,
    container: String,
    port: u16,
    media_source: String,
    pub(super) credentials: Credentials,
}
pub(super) async fn load(state: &AppState, key: &str) -> Result<Support> {
    let key = key.to_owned();
    let row = storage::load(key, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let credentials = serde_json::from_slice(
        &state
            .secrets
            .decrypt(&format!("support:{}", row.0), &row.5)?,
    )
    .map_err(|_| unavailable())?;
    Ok(Support {
        id: row.0,
        kind: row.1,
        container: row.2,
        port: row.3,
        media_source: row.4,
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
pub(super) async fn ensure_idle(state: &AppState, key: &str) -> Result<()> {
    let s = load(state, key).await?;
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
    let (base, media_source) =
        evidence_for(state, &s.container, s.port, s.kind != "prowlarr").await?;
    if media_source != s.media_source {
        return Err(ApiError::conflict(
            "Service mounts changed; reconnect the service",
        ));
    }
    Ok(Connection {
        state,
        base,
        key: s.credentials.secret.clone(),
        kind: s.kind.clone(),
    })
}
pub(super) async fn rpc(
    c: &Connection<'_>,
    credentials: &Credentials,
    method: &str,
    params: Value,
) -> Result<Value> {
    let response = c
        .state
        .managers
        .http
        .post(format!("{}/jsonrpc", c.base))
        .basic_auth(&credentials.username, Some(&credentials.secret))
        .json(&json!({"method":method,"params":params,"id":1}))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(ApiError::conflict(
            "Download service rejected the API credentials or request",
        ));
    }
    let result = read(response).await?;
    if !result["error"].is_null() {
        return Err(ApiError::conflict("Download service rejected the command"));
    }
    result.get("result").cloned().ok_or_else(unavailable)
}
pub(super) async fn version(c: &Connection<'_>, credentials: &Credentials) -> Result<String> {
    let value = match c.kind.as_str() {
        "nzbget" => rpc(c, credentials, "version", json!([])).await?,
        "bazarr" => c.get("system/status").await?["data"]["bazarr_version"].clone(),
        _ => {
            let value = c.get("system/status").await?;
            if value["appName"] != "Prowlarr" {
                return Err(unavailable());
            }
            value["version"].clone()
        }
    };
    value
        .as_str()
        .filter(|v| !v.is_empty() && v.len() < 100)
        .map(str::to_owned)
        .ok_or_else(unavailable)
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
    if !matches!(input.kind.as_str(), "bazarr" | "prowlarr" | "nzbget")
        || input.name.trim().is_empty()
        || input.name.len() > 100
        || input.credentials.secret.is_empty()
        || input.credentials.secret.len() > 1024
        || input.credentials.username.len() > 100
        || input.native_url.len() > 2000
        || !native_url(&input.native_url)
    {
        return Err(ApiError::bad(
            "Choose a supported service, credentials and an HTTP(S) UI address without credentials or query parameters",
        ));
    }
    let _guard = state.managers.guard.service(&input.kind).await;
    let (base, media_source) = evidence_for(
        &state,
        &input.container_id,
        input.port,
        input.kind != "prowlarr",
    )
    .await?;
    let c = Connection {
        state: &state,
        base,
        key: input.credentials.secret.clone(),
        kind: input.kind.clone(),
    };
    let version = version(&c, &input.credentials).await?;
    let kind = input.kind.clone();
    let key = storage::register_with_actor_read_support_services(kind, &state.db)
        .await?
        .unwrap_or_else(id);
    let returned = key.clone();
    let secret = state.secrets.encrypt(
        &format!("support:{key}"),
        &serde_json::to_vec(&input.credentials).map_err(|_| unavailable())?,
    )?;
    let allowed = storage::register_with_actor_write_stack_provisions(
        media_source,
        version,
        key,
        secret,
        &state.db,
        input,
        actor_id,
    )
    .await?;
    if !allowed {
        return Err(ApiError::conflict(
            "A managed service operation already owns this integration type",
        ));
    }
    Ok(Json(json!({"id":returned})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows = storage::list(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
fn text(value: &Value, secret: &str) -> String {
    value
        .as_str()
        .unwrap_or("")
        .replace(secret, "[redacted]")
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
async fn snapshot(c: &Connection<'_>, s: &Support) -> Result<Value> {
    match s.kind.as_str() {
        "nzbget" => {
            let status = rpc(c, &s.credentials, "status", json!([])).await?;
            let queue = rpc(c, &s.credentials, "listgroups", json!([0])).await?;
            let history = rpc(c, &s.credentials, "history", json!([true])).await?;
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
            let service = load(state, &key).await?;
            let c = connect(state, &service).await?;
            snapshot(&c, &service).await
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
    let result = async {
        let c = connect(&state, &s).await?;
        snapshot(&c, &s).await
    }
    .await;
    let error = result.as_ref().err().map(|e| e.2.clone());
    storage::inspect(&state.db, key, error).await?;
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

pub(super) async fn provision(state: AppState, actor: String, input: Value) -> Result<Json<Value>> {
    register_with_actor(
        state,
        serde_json::from_value(input)
            .map_err(|_| ApiError::bad("Invalid managed service configuration"))?,
        actor,
    )
    .await
}
