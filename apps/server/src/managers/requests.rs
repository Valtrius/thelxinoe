#[path = "../storage/managers/requests.rs"]
mod storage;

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
    security::principal(&state, &headers).await?;
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
    let rows=remote.as_array().ok_or_else(unavailable)?.iter().take(50).filter_map(|r|{
        let external=external(&kind,r)?;let title=r["title"].as_str()?;
        Some(json!({"external_id":external,"title":title,"year":r["year"],"overview":r["overview"].as_str().unwrap_or("").chars().take(1000).collect::<String>(),"artist":r["artist"]["artistName"]}))
    }).collect::<Vec<_>>();
    Ok(Json(json!({"service":s.name,"local":local,"items":rows})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let admin = p.user.role == Role::Admin;
    let rows = storage::list(&state.db, p, admin).await?;
    Ok(Json(json!({"items":rows})))
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
    let result = storage::request(&state.db, input, p, s, title).await?;
    Ok(Json(json!({"id":result.0,"state":result.1})))
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
    if !matches!(
        input.action.as_str(),
        "approve" | "deny" | "cancel" | "reacquire"
    ) {
        return Err(ApiError::bad("Unknown request action"));
    }
    if matches!(input.action.as_str(), "approve" | "deny") && p.user.role != Role::Admin {
        return Err(ApiError::forbidden());
    }
    let changed = storage::decide(&state.db, key, input, p).await?;
    if !changed {
        return Err(ApiError::conflict(
            "This request cannot be changed in its current state",
        ));
    }
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
    storage::auto_approve(&state.db, user, input, p).await?;
    Ok(Json(json!({"saved":true})))
}
async fn update(state: &AppState, key: &str, status: &str, manager: Option<i64>) -> Result<()> {
    let key = key.to_owned();
    let status = status.to_owned();
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
    if let Err(error) = perform(state, &key).await {
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
        row.3.as_str(),
        "requested" | "available" | "denied" | "cancelled"
    ) {
        return Ok(());
    }
    if row.3 == "searching" {
        return Err(ApiError::conflict(
            "The previous search may have reached the manager. Review its queue before retrying.",
        ));
    }
    if !matches!(row.3.as_str(), "approved" | "adding") {
        return Err(ApiError::conflict("Request is not approved"));
    }
    let s = service(state, &row.0).await?;
    if s.generation != row.1 {
        return Err(ApiError::conflict(
            "Manager configuration changed; approve this request again",
        ));
    }
    let defaults: Defaults = serde_json::from_value(s.defaults.clone())
        .map_err(|_| ApiError::bad("Acquisition defaults are missing"))?;
    if defaults.root_folder != canonical_root(&s.kind) {
        return Err(ApiError::conflict(
            "Save acquisition defaults using the canonical /media library folder",
        ));
    }
    let c = Connection::open(state, &s).await?;
    let kind = endpoint(&s.kind);
    // Re-read manager metadata and ownership instead of accepting a client-supplied object.
    let mut item = lookup(&c, &row.2).await?;
    let existing = c.get(kind).await?;
    let existing = existing
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .find(|r| external(&s.kind, r).as_deref() == Some(&row.2))
        .cloned();
    update(state, key, "adding", None).await?;
    let mut item = if let Some(existing) = existing {
        existing
    } else {
        if s.kind == "lidarr" {
            let artist_id = item["artist"]["foreignArtistId"]
                .as_str()
                .ok_or_else(unavailable)?
                .to_owned();
            let artists = c.get("artist").await?;
            let existing = artists
                .as_array()
                .ok_or_else(unavailable)?
                .iter()
                .find(|a| a["foreignArtistId"] == artist_id)
                .cloned();
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
        update(state, key, "requested", Some(manager)).await?;
        return Ok(());
    }
    super::retention::reacquire(state, &s, &c, &row.2, manager).await?;
    update(state, key, "searching", Some(manager)).await?;
    let command = match s.kind.as_str() {
        "radarr" => json!({"name":"MoviesSearch","movieIds":[manager]}),
        "sonarr" => json!({"name":"SeriesSearch","seriesId":manager}),
        _ => json!({"name":"AlbumSearch","albumIds":[manager]}),
    };
    c.call(reqwest::Method::POST, "command", &[], Some(command))
        .await?;
    update(state, key, "requested", Some(manager)).await?;
    Ok(())
}
