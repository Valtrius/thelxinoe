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
    let permissions = 32 | if automatic { 128 } else { 0 } | if admin { 16 } else { 0 };
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
    Ok(Json(public(
        call(
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
        .await?,
    )))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestMedia {
    media_type: String,
    media_id: i64,
    #[serde(default)]
    seasons: Vec<u32>,
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
    let (service, c) = connection(&state).await?;
    let user = user(&c, &service, &principal).await?;
    let mut body = json!({"mediaType":input.media_type,"mediaId":input.media_id,"is4k":false});
    sync_connections(
        &state,
        Some(if input.media_type == "movie" {
            "radarr"
        } else {
            "sonarr"
        }),
        false,
    )
    .await?;
    if input.media_type == "tv" {
        body["seasons"] = json!(input.seasons);
    }
    Ok(Json(public(
        call(
            &c,
            reqwest::Method::POST,
            "request",
            &[],
            Some(body),
            Some(user),
        )
        .await?,
    )))
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
    Ok(Json(public(
        call(&c, method, &path, &[], None, Some(user)).await?,
    )))
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
    if storage::service(&state.db).await?.is_none() {
        return Ok(());
    }
    let (_, c) = connection(state).await?;
    let _guard = state.managers.guard.service("seerr").await;
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
    let name = profiles
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == defaults.quality_profile)
        .and_then(|p| p["name"].as_str())
        .ok_or_else(unavailable)?;
    let path = format!("settings/{}", s.kind);
    let existing = c.get(&path).await?;
    let owned_name = format!("Thelxinoe {}", s.kind);
    let previous = existing
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["name"] == owned_name);
    let body = json!({"name":owned_name,"hostname":url.host_str(),"port":s.port,"apiKey":manager.key,"useSsl":false,"baseUrl":"","activeProfileId":defaults.quality_profile,"activeProfileName":name,"activeDirectory":defaults.root_folder,"is4k":false,"isDefault":true,"syncEnabled":true,"preventSearch":!defaults.monitored,"minimumAvailability":"released","enableSeasonFolders":true});
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
