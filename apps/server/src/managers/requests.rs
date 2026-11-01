#[path = "../storage/managers/requests.rs"]
mod storage;

use super::domain::{CreateAcquisition, DecideAcquisition, RequestAction, RequestState};
use super::*;
use axum::extract::Query;
use thelxinoe_core::Role;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/acquisition/services", get(services))
        .route("/api/v1/acquisition/search", get(search))
        .route("/api/v1/acquisition/requests", get(list).post(request))
        .route("/api/v1/acquisition/requests/{id}", post(decide))
        .route("/api/v1/admin/acquisition/users", get(approval_users))
        .route(
            "/api/v1/admin/acquisition/users/{id}",
            axum::routing::put(auto_approve),
        )
}
async fn services(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let rows = storage::services(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
pub(super) fn endpoint(kind: &str) -> &'static str {
    match kind {
        "radarr" => "movie",
        "sonarr" => "series",
        _ => "album",
    }
}
pub(super) fn external(kind: &str, row: &Value) -> Option<String> {
    match kind {
        "radarr" => row["tmdbId"].as_u64().map(|v| v.to_string()),
        "sonarr" => row["tvdbId"].as_u64().map(|v| v.to_string()),
        _ => row["foreignAlbumId"].as_str().map(str::to_owned),
    }
}
fn identifier(kind: &str, value: &str) -> bool {
    if kind == "lidarr" {
        uuid::Uuid::parse_str(value).is_ok()
    } else {
        value.len() <= 12 && value.parse::<u64>().is_ok_and(|v| v > 0)
    }
}
pub(super) async fn lookup(c: &Connection<'_>, external_id: &str) -> Result<Value> {
    if !identifier(&c.kind, external_id) {
        return Err(ApiError::bad("Invalid provider identifier"));
    }
    let prefix = match c.kind.as_str() {
        "radarr" => "tmdb",
        "sonarr" => "tvdb",
        _ => "mbid",
    };
    let rows = c
        .call(
            reqwest::Method::GET,
            &format!("{}/lookup", endpoint(&c.kind)),
            &[("term", format!("{prefix}:{external_id}"))],
            None,
        )
        .await?;
    rows.as_array()
        .and_then(|a| {
            a.iter()
                .find(|r| external(&c.kind, r).as_deref() == Some(external_id))
        })
        .cloned()
        .ok_or_else(ApiError::not_found)
}
#[derive(Deserialize)]
struct Search {
    service_id: String,
    term: String,
}
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Search>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if input.term.trim().len() < 2 || input.term.len() > 200 {
        return Err(ApiError::bad("Enter between 2 and 200 characters"));
    }
    let s = service(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let remote = c
        .call(
            reqwest::Method::GET,
            &format!("{}/lookup", endpoint(&s.kind)),
            &[("term", input.term.trim().into())],
            None,
        )
        .await?;
    let domain = match s.kind.as_str() {
        "radarr" => "movie",
        "sonarr" => "show",
        _ => "album",
    };
    let term = input.term.trim().to_owned();
    let kind = s.kind.clone();
    let local = storage::search(&state.db, domain, term).await?;
    let remote = remote.as_array().ok_or_else(unavailable)?.iter().take(50);
    let rows = if kind == "lidarr" {
        let mut rows = remote
            .filter_map(|r| {
                Some(super::music::album(
                    &external(&kind, r).filter(|id| identifier(&kind, id))?,
                    r["title"].as_str()?,
                    r["artist"]["artistName"].as_str(),
                    r["releaseDate"].as_str(),
                ))
            })
            .collect::<Vec<_>>();
        decorate(&state, &p.user.id, &s.id, &mut rows).await?;
        rows
    } else {
        remote.filter_map(|r|{
            let external=external(&kind,r)?;let title=r["title"].as_str()?;
            Some(json!({"external_id":external,"title":title,"year":r["year"],"overview":r["overview"].as_str().unwrap_or("").chars().take(1000).collect::<String>()}))
        }).collect::<Vec<_>>()
    };
    Ok(Json(json!({"service":s.name,"local":local,"items":rows})))
}
/// Marks the albums the library holds and the viewer's latest request for each.
pub(super) async fn decorate(
    state: &AppState,
    user: &str,
    service: &str,
    items: &mut [Value],
) -> Result<()> {
    let ids = items
        .iter()
        .filter_map(|item| item["external_id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let (held, requests) =
        storage::albums(&state.db, user.to_owned(), service.to_owned(), ids).await?;
    for item in items {
        let id = item["external_id"].as_str().unwrap_or_default();
        let request = requests
            .get(id)
            .map_or(Value::Null, |(id, state)| json!({"id":id,"state":state}));
        item["in_library"] = json!(held.contains(id));
        item["request"] = request;
    }
    Ok(())
}
#[derive(Default, Deserialize)]
struct HistoryPage {
    #[serde(default)]
    page: u32,
}
async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<HistoryPage>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let admin = p.user.role == Role::Admin;
    Ok(Json(storage::list(&state.db, p, admin, input.page).await?))
}
#[derive(Deserialize)]
struct Request {
    service_id: String,
    external_id: String,
}

async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let s = service(&state, &input.service_id).await?;
    let _: Defaults = serde_json::from_value(s.defaults.clone())
        .map_err(|_| ApiError::bad("Administrator must choose acquisition defaults first"))?;
    let c = Connection::open(&state, &s).await?;
    let row = lookup(&c, &input.external_id).await?;
    let title = row["title"]
        .as_str()
        .ok_or_else(unavailable)?
        .chars()
        .take(500)
        .collect::<String>();
    let result = storage::request(
        &state.db,
        CreateAcquisition {
            user_id: p.user.id,
            administrator: p.user.role == Role::Admin,
            service_id: s.id,
            service_generation: s.generation,
            external_id: input.external_id,
            title,
        },
    )
    .await?;
    state.notify_attention();
    Ok(Json(json!({"id":result.id,"state":result.state})))
}
#[derive(Deserialize)]
struct Decision {
    action: String,
}
async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Decision>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let action: RequestAction = serde_json::from_value(json!(input.action))
        .map_err(|_| ApiError::bad("Unknown request action"))?;
    if matches!(action, RequestAction::Approve | RequestAction::Deny) && p.user.role != Role::Admin
    {
        return Err(ApiError::forbidden());
    }
    let changed = storage::decide(
        &state.db,
        DecideAcquisition {
            id: key,
            actor: p.user.id,
            administrator: p.user.role == Role::Admin,
            action,
        },
    )
    .await?;
    if !changed {
        return Err(ApiError::conflict(
            "This request cannot be changed in its current state",
        ));
    }
    state.notify_attention();
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Auto {
    enabled: bool,
}
async fn approval_users(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let rows = storage::approval_users(&state.db).await?;
    Ok(Json(json!({"items":rows})))
}
async fn auto_approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user): Path<String>,
    Json(input): Json<Auto>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    storage::auto_approve(&state.db, user, input.enabled, p.user.id).await?;
    Ok(Json(json!({"saved":true})))
}
async fn update(
    state: &AppState,
    key: &str,
    status: RequestState,
    manager: Option<i64>,
) -> Result<()> {
    let key = key.to_owned();
    storage::update(key, status, &state.db, manager).await?;
    Ok(())
}
pub(crate) async fn acquire(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let key = job.payload["request_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid acquisition job"))?
        .to_owned();
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    let result = perform(state, &key).await;
    // An added title appears in browse lists as soon as its manager reports it.
    super::status::refresh_soon(state);
    if let Err(error) = result {
        let message = error.2;
        storage::acquire(key, message, &state.db).await?;
        anyhow::bail!("Acquisition needs attention; review its request status")
    }
    Ok(())
}
async fn perform(state: &AppState, key: &str) -> Result<()> {
    let request_id = key.to_owned();
    let row = storage::perform(request_id, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if matches!(
        row.state,
        RequestState::Requested
            | RequestState::Available
            | RequestState::Denied
            | RequestState::Cancelled
    ) {
        return Ok(());
    }
    if row.state == RequestState::Searching {
        return Err(ApiError::conflict(
            "The previous search may have reached the manager. Review its queue before retrying.",
        ));
    }
    if !matches!(row.state, RequestState::Approved | RequestState::Adding) {
        return Err(ApiError::conflict("Request is not approved"));
    }
    let s = service(state, &row.service_id).await?;
    if s.generation != row.service_generation {
        return Err(ApiError::conflict(
            "Manager configuration changed; approve this request again",
        ));
    }
    let defaults: Defaults = serde_json::from_value(s.defaults.clone())
        .map_err(|_| ApiError::bad("Acquisition defaults are missing"))?;
    if !clean_path(&defaults.root_folder) {
        return Err(ApiError::conflict(
            "Save acquisition defaults using the canonical /media library folder",
        ));
    }
    let c = Connection::open(state, &s).await?;
    let kind = endpoint(&s.kind);
    // Re-read manager metadata and ownership instead of accepting a client-supplied object.
    let mut item = lookup(&c, &row.external_id).await?;
    // Ask for this one title; a manager that ignores the filter lists it among the rest.
    let filter = match s.kind.as_str() {
        "radarr" => "tmdbId",
        "sonarr" => "tvdbId",
        _ => "foreignAlbumId",
    };
    let existing = c
        .fetch::<Vec<Value>>(kind, &[(filter, row.external_id.clone())])
        .await?
        .into_iter()
        .find(|r| external(&s.kind, r).as_deref() == Some(&row.external_id));
    update(state, key, RequestState::Adding, None).await?;
    let mut item = if let Some(existing) = existing {
        existing
    } else {
        if s.kind == "lidarr" {
            let artist_id = item["artist"]["foreignArtistId"]
                .as_str()
                .ok_or_else(unavailable)?
                .to_owned();
            let existing = c
                .fetch::<Vec<Value>>("artist", &[("mbId", artist_id.clone())])
                .await?
                .into_iter()
                .find(|a| a["foreignArtistId"] == artist_id);
            let artist = if let Some(a) = existing {
                a
            } else {
                let mut artist = item["artist"].clone();
                artist["rootFolderPath"] = json!(defaults.root_folder);
                artist["qualityProfileId"] = json!(defaults.quality_profile);
                artist["metadataProfileId"] = json!(defaults.metadata_profile);
                artist["monitored"] = json!(true);
                artist["addOptions"] = json!({"monitor":"none","searchForMissingAlbums":false});
                artist
            };
            item["artistId"] = json!(artist["id"].as_i64().unwrap_or(0));
            item["artist"] = artist;
        } else {
            item["rootFolderPath"] = json!(defaults.root_folder);
            item["qualityProfileId"] = json!(defaults.quality_profile);
        }
        item["monitored"] = json!(defaults.monitored);
        if s.kind == "sonarr" {
            item["seasonFolder"] = json!(true);
            item["addOptions"] = json!({"searchForMissingEpisodes":false,"monitor":"all"});
            if let Some(seasons) = item["seasons"].as_array_mut() {
                for season in seasons {
                    season["monitored"] = json!(defaults.monitored && season["seasonNumber"] != 0);
                }
            }
        }
        if s.kind == "radarr" {
            item["minimumAvailability"] = json!("released");
            item["addOptions"] = json!({"searchForMovie":false});
        }
        c.call(reqwest::Method::POST, kind, &[], Some(item)).await?
    };
    let manager = item["id"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)?;
    if s.kind == "lidarr" && defaults.monitored {
        let artist_id = item["artistId"]
            .as_i64()
            .or_else(|| item["artist"]["id"].as_i64())
            .filter(|v| *v > 0)
            .ok_or_else(unavailable)?;
        let mut artist = c.get(&format!("artist/{artist_id}")).await?;
        if artist["monitored"] != true {
            artist["monitored"] = json!(true);
            c.call(
                reqwest::Method::PUT,
                &format!("artist/{artist_id}"),
                &[],
                Some(artist),
            )
            .await?;
        }
    }
    if defaults.monitored && item["monitored"] != true {
        item["monitored"] = json!(true);
        c.call(
            reqwest::Method::PUT,
            &format!("{kind}/{manager}"),
            &[],
            Some(item),
        )
        .await?;
    }
    if !defaults.monitored {
        update(state, key, RequestState::Requested, Some(manager)).await?;
        return Ok(());
    }
    super::retention::reacquire(state, &s, &c, &row.external_id, manager).await?;
    update(state, key, RequestState::Searching, Some(manager)).await?;
    let command = match s.kind.as_str() {
        "radarr" => json!({"name":"MoviesSearch","movieIds":[manager]}),
        "sonarr" => json!({"name":"SeriesSearch","seriesId":manager}),
        _ => json!({"name":"AlbumSearch","albumIds":[manager]}),
    };
    c.call(reqwest::Method::POST, "command", &[], Some(command))
        .await?;
    update(state, key, RequestState::Requested, Some(manager)).await?;
    Ok(())
}
