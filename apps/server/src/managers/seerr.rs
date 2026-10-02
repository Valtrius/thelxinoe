//! Seerr owns discovery, requests and availability. Radarr/Sonarr own the media.
#[path = "../storage/managers/seerr.rs"]
mod storage;
use super::*;
use axum::{
    extract::{Query, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thelxinoe_core::{Principal, Role};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/seerr/status", get(status))
        .route("/api/v1/seerr/discover/{feed}", get(discover))
        .route("/api/v1/seerr/search", get(search))
        .route("/api/v1/seerr/profiles/{kind}", get(request_profiles))
        .route("/api/v1/seerr/{kind}/{id}", get(details))
        .route(
            "/api/v1/seerr/{kind}/{id}/recommendations",
            get(recommendations),
        )
        .route("/api/v1/seerr/requests", get(requests).post(request))
        .route("/api/v1/seerr/requests/{id}/{action}", post(decide))
        .route("/api/v1/admin/seerr/sync", post(sync))
        .route(
            "/seerr-bootstrap/{*path}",
            axum::routing::any(bootstrap_endpoint),
        )
}

async fn connection(state: &AppState) -> Result<(String, Connection<'_>)> {
    let id = storage::service(&state.db).await?.ok_or_else(|| {
        ApiError::conflict(
            "Seerr is not connected. An administrator can install it in Settings → Media services.",
        )
    })?;
    let service = support::load(state, &id).await?;
    Ok((id, support::connect(state, &service).await?))
}

async fn call(
    c: &Connection<'_>,
    method: reqwest::Method,
    path: &str,
    query: &[(&str, String)],
    body: Option<Value>,
    user: Option<i64>,
) -> Result<Value> {
    let mut request = c
        .state
        .managers
        .http
        .request(method, format!("{}/api/v1/{path}", c.base))
        .header("X-Api-Key", &c.key)
        .query(query);
    if let Some(user) = user {
        request = request.header("X-Api-User", user.to_string());
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|_| {
        ApiError::conflict("Seerr is unavailable. Check the service and try again.")
    })?;
    if !response.status().is_success() {
        let status = response.status();
        let message = match status.as_u16() {
            403 => "Seerr did not permit this action",
            409 => "This media has already been requested",
            429 => "Seerr's request limit has been reached",
            _ => "Seerr could not complete the request",
        };
        return Err(ApiError::conflict(format!(
            "{message} (HTTP {})",
            status.as_u16()
        )));
    }
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    read(response).await
}

fn public(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for key in [
                "email",
                "password",
                "apiKey",
                "plexToken",
                "jellyfinAuthToken",
                "jellyfinDeviceId",
                "settings",
                "permissions",
            ] {
                map.remove(key);
            }
            for item in map.values_mut() {
                *item = public(item.take());
            }
            if let Some(uhd) = map
                .get("status4k")
                .and_then(Value::as_u64)
                .filter(|status| (1..=5).contains(status))
            {
                let hd = map.get("status").and_then(Value::as_u64).unwrap_or(1);
                map.insert("status".into(), json!(hd.max(uhd)));
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = public(item.take());
            }
        }
        _ => {}
    }
    value
}

async fn user(c: &Connection<'_>, service: &str, principal: &Principal) -> Result<i64> {
    let _guard = c
        .state
        .managers
        .guard
        .service(&format!("seerr-user:{}", principal.user.id))
        .await;
    user_locked(c, service, principal).await
}

async fn user_locked(c: &Connection<'_>, service: &str, principal: &Principal) -> Result<i64> {
    let stored =
        storage::mapped_user(&c.state.db, service.into(), principal.user.id.clone()).await?;
    let remote = if let Some(id) = stored {
        id
    } else {
        let email = format!("{}@thelxinoe.invalid", principal.user.id);
        // Recover a completed create whose response or local write was interrupted.
        let found = call(
            c,
            reqwest::Method::GET,
            "user",
            &[("take", "100".into()), ("q", email.clone())],
            None,
            None,
        )
        .await?;
        let existing = found["results"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|u| u["email"] == email);
        let created = if let Some(value) = existing {
            value.clone()
        } else {
            call(c,reqwest::Method::POST,"user",&[],Some(json!({"email":email,"username":principal.user.username,"password":thelxinoe_auth::token()})),None).await?
        };
        let id = created["id"]
            .as_i64()
            .filter(|id| *id > 1)
            .ok_or_else(unavailable)?;
        storage::save_user(&c.state.db, service.into(), principal.user.id.clone(), id).await?;
        id
    };
    let admin = principal.user.role == Role::Admin;
    let automatic = admin || storage::auto_approve(&c.state.db, principal.user.id.clone()).await?;
    let permissions =
        32 | 1024 | 8192 | if automatic { 128 | 32768 } else { 0 } | if admin { 16 } else { 0 };
    call(
        c,
        reqwest::Method::POST,
        &format!("user/{remote}/settings/permissions"),
        &[],
        Some(json!({"permissions":permissions})),
        None,
    )
    .await?;
    Ok(remote)
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    if storage::service(&state.db).await?.is_none() {
        return Ok(Json(json!({"configured":false})));
    }
    let (_, c) = connection(&state).await?;
    let settings = c.get("settings/public").await?;
    Ok(Json(
        json!({"configured":true,"ready":settings["initialized"]==true}),
    ))
}
#[derive(Default, Deserialize)]
struct Browse {
    #[serde(default)]
    page: u32,
    #[serde(default)]
    q: String,
}
fn page(input: &Browse) -> String {
    input.page.clamp(1, 500).to_string()
}
async fn discover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(feed): Path<String>,
    Query(input): Query<Browse>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let path = match feed.as_str() {
        "trending" => "discover/trending",
        "movies" => "discover/movies",
        "tv" => "discover/tv",
        "upcoming" => "discover/movies/upcoming",
        "upcoming-tv" => "discover/tv/upcoming",
        _ => return Err(ApiError::not_found()),
    };
    let (_, c) = connection(&state).await?;
    Ok(Json(public(
        call(
            &c,
            reqwest::Method::GET,
            path,
            &[("page", page(&input))],
            None,
            None,
        )
        .await?,
    )))
}
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Browse>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    if input.q.trim().len() < 2 || input.q.len() > 200 {
        return Err(ApiError::bad("Enter between 2 and 200 characters"));
    }
    let (_, c) = connection(&state).await?;
    Ok(Json(public(
        call(
            &c,
            reqwest::Method::GET,
            "search",
            &[("query", input.q.trim().into()), ("page", page(&input))],
            None,
            None,
        )
        .await?,
    )))
}
fn media_path(kind: &str, id: i64) -> Result<String> {
    if !matches!(kind, "movie" | "tv") || id <= 0 {
        return Err(ApiError::bad("Choose a movie or show"));
    }
    Ok(format!("{kind}/{id}"))
}
async fn details(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let path = media_path(&kind, id)?;
    let (_, c) = connection(&state).await?;
    Ok(Json(public(c.get(&path).await?)))
}
async fn recommendations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, i64)>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let path = format!("{}/recommendations", media_path(&kind, id)?);
    let (_, c) = connection(&state).await?;
    Ok(Json(public(c.get(&path).await?)))
}
async fn requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Browse>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    let (service, c) = connection(&state).await?;
    let user = user(&c, &service, &principal).await?;
    let mut result = call(
        &c,
        reqwest::Method::GET,
        "request",
        &[
            ("take", "20".into()),
            ("skip", ((input.page.max(1) - 1).min(499) * 20).to_string()),
        ],
        None,
        Some(user),
    )
    .await?;
    if let Some(rows) = result["results"].as_array_mut() {
        for row in rows {
            row["attention"] = if row["requestedBy"]["id"] == user {
                json!(crate::operations::seerr_request_attention(&service, row))
            } else {
                Value::Null
            };
        }
    }
    Ok(Json(public(result)))
}

pub(crate) async fn observe_requests(state: &AppState) -> Result<()> {
    let _guard = state.managers.guard.service("seerr-attention").await;
    if storage::service(&state.db).await?.is_none() {
        return Ok(());
    }
    let (service, c) = connection(state).await?;
    let mut rows = Vec::new();
    for page in 0..100 {
        let result = call(
            &c,
            reqwest::Method::GET,
            "request",
            &[("take", "100".into()), ("skip", (page * 100).to_string())],
            None,
            None,
        )
        .await?;
        let batch = result["results"].as_array().ok_or_else(unavailable)?;
        let done = batch.len() < 100
            || result["pageInfo"]["results"]
                .as_u64()
                .is_some_and(|total| rows.len() as u64 + batch.len() as u64 >= total);
        rows.extend(batch.iter().cloned());
        if done {
            crate::operations::cache_seerr_requests(state, service, rows).await?;
            return Ok(());
        }
    }
    Err(ApiError::conflict(
        "The request list is too large to observe safely",
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestMedia {
    media_type: String,
    media_id: i64,
    #[serde(default)]
    seasons: Vec<u32>,
    profile: Option<ProfileRef>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
enum ProfileRef {
    Guide {
        service_id: String,
        trash_id: String,
    },
    Custom {
        service_id: String,
        profile_id: i64,
    },
}
impl ProfileRef {
    fn service_id(&self) -> &str {
        match self {
            Self::Guide { service_id, .. } | Self::Custom { service_id, .. } => service_id,
        }
    }
}
#[derive(Deserialize)]
struct ProfileQuery {
    media_id: i64,
}
async fn request_manager(state: &AppState, kind: &str) -> Result<Service> {
    for key in storage::managers(&state.db).await? {
        let service = super::service(state, &key).await?;
        if service.kind == kind {
            return Ok(service);
        }
    }
    Err(ApiError::conflict(format!(
        "Connect {kind} in Media services before requesting this media"
    )))
}
async fn existing_series_profile(
    c: &Connection<'_>,
    seerr: &Connection<'_>,
    media_id: i64,
) -> Result<Option<i64>> {
    let media = seerr.get(&format!("tv/{media_id}")).await?;
    let tvdb = media["externalIds"]["tvdbId"]
        .as_i64()
        .or_else(|| media["mediaInfo"]["tvdbId"].as_i64());
    let Some(tvdb) = tvdb.filter(|id| *id > 0) else {
        return Ok(None);
    };
    let series = c
        .call(
            reqwest::Method::GET,
            "series",
            &[("tvdbId", tvdb.to_string())],
            None,
        )
        .await?;
    Ok(series
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["tvdbId"] == tvdb)
        .and_then(|s| s["qualityProfileId"].as_i64()))
}
fn is_uhd(items: &Value) -> bool {
    items.as_array().into_iter().flatten().any(|item| {
        item["allowed"] == true && (item["quality"]["resolution"] == 2160 || is_uhd(&item["items"]))
    })
}
async fn request_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(input): Query<ProfileQuery>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    media_path(&kind, input.media_id)?;
    let manager_kind = if kind == "movie" { "radarr" } else { "sonarr" };
    let service = request_manager(&state, manager_kind).await?;
    let c = Connection::open(&state, &service).await?;
    let profiles = c.get("qualityprofile").await?;
    let mappings = recyclarr::profiles(&state, &service.id).await?;
    let mut items = Vec::new();
    for profile in profiles.as_array().ok_or_else(unavailable)? {
        let id = profile["id"].as_i64().ok_or_else(unavailable)?;
        let mapping = mappings.iter().find(|m| m.profile_id == id);
        let reference = if let Some(mapping) = mapping {
            ProfileRef::Guide {
                service_id: service.id.clone(),
                trash_id: mapping.trash_id.clone(),
            }
        } else {
            ProfileRef::Custom {
                service_id: service.id.clone(),
                profile_id: id,
            }
        };
        let default = if let Some(trash) = service.defaults["quality_profile_trash_id"].as_str() {
            mapping.is_some_and(|m| m.trash_id == trash)
        } else {
            service.defaults["quality_profile"] == id
        };
        items.push(json!({"name":profile["name"],"default":default,"profile":reference}));
    }
    items.sort_by(|a, b| {
        b["default"]
            .as_bool()
            .cmp(&a["default"].as_bool())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let locked = if kind == "tv" {
        let (_, seerr) = connection(&state).await?;
        existing_series_profile(&c, &seerr, input.media_id)
            .await?
            .is_some()
    } else {
        false
    };
    Ok(Json(json!({"items":items,"locked":locked})))
}
async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RequestMedia>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    media_path(&input.media_type, input.media_id)?;
    if input.seasons.len() > 100
        || input.seasons.iter().any(|s| *s > 1000)
        || (input.media_type == "tv" && input.seasons.is_empty())
    {
        return Err(ApiError::bad("Select the seasons to request"));
    }
    let kind = if input.media_type == "movie" {
        "radarr"
    } else {
        "sonarr"
    };
    let user_lock = format!("seerr-user:{}", principal.user.id);
    let guard = state
        .managers
        .guard
        .services(&[kind, "seerr", &user_lock])
        .await;
    let (service, c) = connection(&state).await?;
    let manager = request_manager(&state, kind).await?;
    let native = Connection::open(&state, &manager).await?;
    let profiles = native.get("qualityprofile").await?;
    let mappings = recyclarr::profiles(&state, &manager.id).await?;
    let requested = if let Some(reference) = &input.profile {
        if reference.service_id() != manager.id {
            return Err(ApiError::bad("Choose a profile from this media's manager"));
        }
        match reference {
            ProfileRef::Guide { trash_id, .. } => mappings
                .iter()
                .find(|m| m.trash_id == *trash_id)
                .map(|m| m.profile_id),
            ProfileRef::Custom { profile_id, .. } => {
                if mappings.iter().any(|m| m.profile_id == *profile_id) {
                    None
                } else {
                    Some(*profile_id)
                }
            }
        }
    } else if let Some(trash) = manager.defaults["quality_profile_trash_id"].as_str() {
        mappings
            .iter()
            .find(|m| m.trash_id == trash)
            .map(|m| m.profile_id)
    } else {
        manager.defaults["quality_profile"].as_i64()
    };
    let existing = if kind == "sonarr" {
        existing_series_profile(&native, &c, input.media_id).await?
    } else {
        None
    };
    let selected = if let Some(existing) = existing {
        if input.profile.is_some() && requested != Some(existing) {
            return Err(ApiError::conflict(
                "This series already has a profile; change it in Sonarr",
            ));
        }
        existing
    } else {
        requested.ok_or_else(|| {
            ApiError::conflict("The request profile is unavailable; select an existing profile")
        })?
    };
    let profile = profiles
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == selected)
        .ok_or_else(|| {
            ApiError::conflict("The request profile was removed; select an existing profile")
        })?;
    let mut body = json!({"mediaType":input.media_type,"mediaId":input.media_id,"is4k":is_uhd(&profile["items"]),"profileId":selected});
    sync_connections_locked(&state, Some(kind), false).await?;
    let configured = c.get(&format!("settings/{kind}")).await?;
    body["serverId"] = configured
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["name"] == format!("Thelxinoe {kind}"))
        .and_then(|entry| entry["id"].as_i64())
        .map(Value::from)
        .ok_or_else(unavailable)?;
    if input.media_type == "tv" {
        body["seasons"] = json!(input.seasons);
    }
    let user = user_locked(&c, &service, &principal).await?;
    let result = public(
        call(
            &c,
            reqwest::Method::POST,
            "request",
            &[],
            Some(body),
            Some(user),
        )
        .await?,
    );
    // Observation acquires its own shared deployment gate. Release mutation
    // locks first so an exclusive operation queued during the POST can finish.
    drop(guard);
    let _ = observe_requests(&state).await;
    crate::operations::observe(&state).await?;
    Ok(Json(result))
}
async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, action)): Path<(i64, String)>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    if id <= 0 || !matches!(action.as_str(), "approve" | "decline" | "cancel" | "retry") {
        return Err(ApiError::bad("Unknown request action"));
    }
    if action != "cancel" && principal.user.role != Role::Admin {
        return Err(ApiError::forbidden());
    }
    let (service, c) = connection(&state).await?;
    let user = user(&c, &service, &principal).await?;
    let (method, path) = if action == "cancel" {
        (reqwest::Method::DELETE, format!("request/{id}"))
    } else {
        (reqwest::Method::POST, format!("request/{id}/{action}"))
    };
    let result = public(call(&c, method, &path, &[], None, Some(user)).await?);
    let _ = observe_requests(&state).await;
    crate::operations::observe(&state).await?;
    Ok(Json(result))
}

/// Seerr requires a first owner before its API can create local users. Perform
/// its supported setup handshake against a two-minute, empty bootstrap server.
/// This endpoint cannot read the catalog, authenticate app users, or issue app tokens.
async fn bootstrap_endpoint(
    State(state): State<AppState>,
    Path(path): Path<String>,
    request: Request,
) -> Result<Response> {
    let method = request.method().clone();
    if path == "System/Info/Public" && method == "GET" {
        return Ok(Json(json!({"Id":state.server_id.as_str(),"ServerName":"Thelxinoe","Version":"10.10.7","ProductName":"Jellyfin Server"})).into_response());
    }
    let fields = crate::jellyfin::seerr_authorization(request.headers())?;
    let login = path == "Users/AuthenticateByName" && method == "POST";
    let token = if login {
        let bytes = axum::body::to_bytes(request.into_body(), 4096)
            .await
            .map_err(|_| ApiError::bad("Invalid setup request"))?;
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid setup request"))?;
        if body["Username"] != "thelxinoe-seerr" {
            return Err(ApiError::unauthorized());
        }
        body["Pw"].as_str().unwrap_or_default().to_owned()
    } else {
        fields.get("token").cloned().unwrap_or_default()
    };
    if token.len() != 64 {
        return Err(ApiError::unauthorized());
    }
    let id = storage::resolve_bootstrap(&state.db, thelxinoe_auth::digest(&token))
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let user = json!({"Id":id,"Name":"thelxinoe-seerr","ServerId":state.server_id.as_str(),"Policy":{"IsAdministrator":true},"Configuration":{"GroupedFolders":[]}});
    if login {
        return Ok(Json(
            json!({"User":user,"AccessToken":token,"ServerId":state.server_id.as_str()}),
        )
        .into_response());
    }
    match (method.as_str(), path.as_str()) {
        ("GET", "System/Info") => Ok(Json(
            json!({"Id":state.server_id.as_str(),"ServerName":"Thelxinoe","Version":"10.10.7"}),
        )
        .into_response()),
        ("POST", "Auth/Keys") => Ok(StatusCode::NO_CONTENT.into_response()),
        ("GET", "Auth/Keys") => {
            Ok(Json(json!({"Items":[{"AppName":"Seerr","AccessToken":token}]})).into_response())
        }
        ("GET", "Users/Me") => Ok(Json(user).into_response()),
        _ => Err(ApiError::not_found()),
    }
}

pub(super) async fn initialize(state: &AppState) -> Result<()> {
    let (service, c) = connection(state).await?;
    let _guard = state.managers.guard.service("seerr").await;
    if c.get("auth/me").await.is_err() {
        let token = thelxinoe_auth::token();
        storage::begin_bootstrap(&state.db, service.clone(), thelxinoe_auth::digest(&token))
            .await?;
        let own = std::env::var("HOSTNAME").map_err(|_| unavailable())?;
        let server = docker(state, &format!("containers/{own}")).await?;
        let remote = support::load(state, &service).await?;
        let remote = docker(state, &format!("containers/{}", remote.container)).await?;
        let hostname = server["networks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|network| {
                remote["networks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|other| other["id"] == network["id"])
            })
            .and_then(|network| network["address"].as_str())
            .ok_or_else(unavailable)?;
        let result=call(&c,reqwest::Method::POST,"auth/jellyfin",&[],Some(json!({"hostname":hostname,"port":state.config.bind.port(),"urlBase":"/seerr-bootstrap","useSsl":false,"serverType":2,"username":"thelxinoe-seerr","password":token})),None).await;
        storage::end_bootstrap(&state.db, service).await?;
        result?;
    }
    // Availability comes exclusively from Arr; media-server scans and sign-in are disabled.
    call(&c,reqwest::Method::POST,"settings/main",&[],Some(json!({"applicationTitle":"Thelxinoe","mediaServerType":4,"mediaServerLogin":false,"localLogin":false,"newPlexLogin":false,"defaultPermissions":32})),None).await?;
    call(
        &c,
        reqwest::Method::POST,
        "settings/initialize",
        &[],
        Some(json!({})),
        None,
    )
    .await?;
    drop(_guard);
    sync_managers(state).await
}

pub(super) async fn sync_managers(state: &AppState) -> Result<()> {
    sync_connections(state, None, false).await
}

async fn sync_connections(state: &AppState, only: Option<&str>, refresh: bool) -> Result<()> {
    let _guard = state.managers.guard.service("seerr").await;
    sync_connections_locked(state, only, refresh).await
}

async fn sync_connections_locked(
    state: &AppState,
    only: Option<&str>,
    refresh: bool,
) -> Result<()> {
    if storage::service(&state.db).await?.is_none() {
        return Ok(());
    }
    let (_, c) = connection(state).await?;
    if c.get("settings/public").await?["initialized"] != true {
        return Ok(());
    }
    let mut services = Vec::new();
    for key in storage::managers(&state.db).await? {
        services.push(super::service(state, &key).await?);
    }
    let mut failure = None;
    for kind in ["radarr", "sonarr"] {
        if only.is_some_and(|selected| selected != kind) {
            continue;
        }
        let result = async {
            if let Some(s) = services.iter().find(|s| s.kind == kind) {
                let defaults =
                    serde_json::from_value::<Defaults>(s.defaults.clone()).map_err(|_| {
                        ApiError::conflict(format!(
                            "Configure {kind}'s quality profile in Media services"
                        ))
                    })?;
                let changed = sync_manager(&c, s, &defaults).await?;
                if changed || refresh {
                    call(
                        &c,
                        reqwest::Method::POST,
                        &format!("settings/jobs/{kind}-scan/run"),
                        &[],
                        None,
                        None,
                    )
                    .await?;
                }
            } else {
                // Retire only the connections this integration creates.
                let existing = c.get(&format!("settings/{kind}")).await?;
                for entry in existing
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|entry| entry["name"] == format!("Thelxinoe {kind}"))
                {
                    let id = entry["id"].as_i64().ok_or_else(unavailable)?;
                    call(
                        &c,
                        reqwest::Method::DELETE,
                        &format!("settings/{kind}/{id}"),
                        &[],
                        None,
                        None,
                    )
                    .await?;
                }
                if only.is_some() {
                    return Err(ApiError::conflict(format!(
                        "Connect {kind} in Media services before requesting this media"
                    )));
                }
            }
            Ok::<_, ApiError>(())
        }
        .await;
        if let Err(error) = result {
            failure = Some(error);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn sync_manager(c: &Connection<'_>, s: &Service, defaults: &Defaults) -> Result<bool> {
    let state = c.state;
    let manager = Connection::open(state, s).await?;
    let url = url::Url::parse(&manager.base).map_err(|_| unavailable())?;
    let profiles = manager.get("qualityprofile").await?;
    let profile = profiles
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == defaults.quality_profile)
        .ok_or_else(unavailable)?;
    let name = profile["name"].as_str().ok_or_else(unavailable)?;
    let is4k = is_uhd(&profile["items"]);
    let path = format!("settings/{}", s.kind);
    let existing = c.get(&path).await?;
    let owned_name = format!("Thelxinoe {}", s.kind);
    let previous = existing
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["name"] == owned_name);
    let body = json!({"name":owned_name,"hostname":url.host_str(),"port":s.port,"apiKey":manager.key,"useSsl":false,"baseUrl":manager.url_base,"activeProfileId":defaults.quality_profile,"activeProfileName":name,"activeDirectory":defaults.root_folder,"is4k":is4k,"isDefault":true,"syncEnabled":true,"preventSearch":!defaults.monitored,"minimumAvailability":"released","enableSeasonFolders":true});
    if previous.is_some_and(|previous| {
        body.as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| previous[key] == *value)
    }) {
        return Ok(false);
    }
    let (method, path) = if let Some(previous) = previous {
        (reqwest::Method::PUT, format!("{path}/{}", previous["id"]))
    } else {
        (reqwest::Method::POST, path)
    };
    call(c, method, &path, &[], Some(body), None).await?;
    Ok(true)
}
async fn sync(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    sync_connections(&state, None, true).await?;
    Ok(Json(json!({"saved":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{call as api, fixture};
    use std::{future::Future, sync::Arc, task::Poll, time::Duration};

    // Browser qualification cannot deterministically queue an exclusive operation
    // between two awaits inside the request handler. Exercise the actual HTTP route.
    #[tokio::test]
    async fn requesting_media_finishes_with_a_queued_exclusive_operation() {
        for pause_at in ["request", "profiles"] {
            let (_temp, mut state, alice) = fixture().await;
            Arc::get_mut(&mut state.config).unwrap().media = "/media".into();
            let reached = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let reached_upstream = reached.clone();
            let release_upstream = release.clone();
            let remote = json!({"id":7,"type":"movie","status":2,"updatedAt":"2026-10-02T10:00:00Z","requestedBy":{"id":2},"media":{"tmdbId":603,"status":3},"seasons":[]});
            let remote_upstream = remote.clone();
            let stub = Router::new().fallback(move |request: Request| {
                let reached = reached_upstream.clone();
                let release = release_upstream.clone();
                let remote = remote_upstream.clone();
                async move {
                    let path = request.uri().path();
                    if (pause_at == "request"
                        && path == "/api/v1/request"
                        && request.method() == "POST")
                        || (pause_at == "profiles" && path == "/api/v3/qualityprofile")
                    {
                        reached.notify_one();
                        release.notified().await;
                    }
                    Json(match path {
                        "/api/v3/qualityprofile" => json!([{"id":1,"name":"HD","items":[]}]),
                        "/api/v1/settings/public" => json!({"initialized":false}),
                        "/api/v1/settings/radarr" => json!([{"id":1,"name":"Thelxinoe radarr"}]),
                        "/api/v1/user/2/settings/permissions" => json!({}),
                        "/api/v1/request" if request.method() == "POST" => remote,
                        "/api/v1/request" => {
                            json!({"results":[remote],"pageInfo":{"pages":1,"results":1}})
                        }
                        _ => panic!("Unexpected upstream request: {path}"),
                    })
                }
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let upstream = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
            let container = "a".repeat(64);
            let inspection = json!({"id":container,"running":true,"mounts":[{"kind":"bind","source":"/media","destination":"/media","writable":true}],"networks":[{"id":"shared","address":"127.0.0.1"}]});
            state
                .managers
                .docker
                .lock()
                .unwrap()
                .insert(format!("containers/{container}"), inspection.clone());
            state
                .managers
                .docker
                .lock()
                .unwrap()
                .insert("containers/self".into(), inspection);
            let manager_key = state
                .secrets
                .encrypt("manager:radarr", b"fixture-key")
                .unwrap();
            let seerr_key = state
                .secrets
                .encrypt("support:seerr", br#"{"secret":"fixture-key"}"#)
                .unwrap();
            state.db.write("test.seerr", move |db| {
                db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at,defaults) VALUES ('radarr','Radarr','radarr',?1,?2,'g',?3,'/media','1',1,'{\"quality_profile\":1}')",params![container,port,manager_key])?;
                db.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('seerr','Seerr','seerr',?1,?2,'g',?3,'','1',1)",params![container,port,seerr_key])?;
                db.execute("INSERT INTO seerr_users VALUES ('seerr','alice',2)",[])?;
                Ok(())
            }).await.unwrap();
            let request_state = state.clone();
            let cookie = alice.clone();
            let mut request = tokio::spawn(async move {
                api(
                    &request_state,
                    "/api/v1/seerr/requests",
                    "POST",
                    json!({"media_type":"movie","media_id":603}),
                    &cookie,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    _ = reached.notified() => (),
                    response = &mut request => panic!("Request ended before reaching {pause_at}: {response:?}"),
                }
            }).await.expect("Upstream request did not reach the controlled pause");
            let mut exclusive = Box::pin(state.managers.guard.lock());
            std::future::poll_fn(|cx| {
                assert!(exclusive.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            release.notify_one();
            let (response, ()) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(request, async {
                    drop(exclusive.await);
                })
            })
            .await
            .expect("The request and exclusive operation deadlocked");
            assert_eq!(response.unwrap().0, StatusCode::OK);
            let snapshot = api(&state, "/api/v1/seerr/requests", "GET", json!({}), &alice)
                .await
                .2;
            let reference = &snapshot["results"][0]["attention"];
            assert_eq!(
                api(
                    &state,
                    &format!("/api/v1/me/attention/{}", reference["id"].as_str().unwrap()),
                    "PUT",
                    json!({"revision":reference["revision"]}),
                    &alice
                )
                .await
                .0,
                StatusCode::OK
            );
            assert_eq!(
                api(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
                    .await
                    .2["items"],
                json!([])
            );
            upstream.abort();
        }
    }
    #[test]
    fn media_routes_and_request_identity_are_closed() {
        assert!(media_path("settings", 1).is_err());
        assert!(media_path("movie", 0).is_err());
        assert!(
            serde_json::from_value::<RequestMedia>(
                json!({"media_type":"movie","media_id":1,"requestedBy":1})
            )
            .is_err()
        );
    }
    #[test]
    fn removes_nested_user_secrets() {
        assert_eq!(
            public(
                json!({"mediaInfo":{"requests":[{"requestedBy":{"id":2,"email":"private","jellyfinAuthToken":"secret","username":"Alice"}}]}})
            )["mediaInfo"]["requests"][0]["requestedBy"],
            json!({"id":2,"username":"Alice"})
        );
    }
}
