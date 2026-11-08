#[path = "../storage/managers/requests.rs"]
mod storage;
#[cfg(test)]
#[path = "requests_tests.rs"]
mod tests;

use super::domain::{
    CreateAcquisition, DecideAcquisition, RequestAction, RequestState, RequestTarget,
};
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
pub(super) fn identifier(kind: &str, value: &str) -> bool {
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
/// The names and MusicBrainz IDs of the library's artists, by name.
pub(super) async fn library_artists(state: &AppState) -> Result<Vec<(String, String)>> {
    Ok(storage::library_artists(&state.db).await?)
}
/// The library artist with this MusicBrainz ID.
pub(super) async fn library_artist(state: &AppState, id: &str) -> Result<Option<String>> {
    Ok(storage::library_artist(&state.db, id.to_owned()).await?)
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
    #[serde(default)]
    target: RequestTarget,
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
    let (title, artist) = if input.target == RequestTarget::Artist {
        if s.kind != "lidarr" {
            return Err(ApiError::bad("Only Lidarr monitors artists"));
        }
        if !identifier(&s.kind, &input.external_id) {
            return Err(ApiError::bad("Invalid provider identifier"));
        }
        let row = super::music::artist_lookup(&c, &input.external_id)
            .await?
            .ok_or_else(ApiError::not_found)?;
        let name = text(&row["artistName"])?;
        (name.clone(), Some((name, input.external_id.clone())))
    } else {
        let row = lookup(&c, &input.external_id).await?;
        let artist = if s.kind == "lidarr" {
            super::music::check_profile(&c, &s, &row).await?;
            Some((
                text(&row["artist"]["artistName"])?,
                text(&row["artist"]["foreignArtistId"])?,
            ))
        } else {
            None
        };
        (text(&row["title"])?, artist)
    };
    let result = storage::request(
        &state.db,
        CreateAcquisition {
            user_id: p.user.id,
            administrator: p.user.role == Role::Admin,
            service_id: s.id,
            service_generation: s.generation,
            external_id: input.external_id,
            title,
            target: input.target,
            artist,
        },
    )
    .await?;
    state.notify_attention();
    Ok(Json(json!({"id":result.id,"state":result.state})))
}
fn text(value: &Value) -> Result<String> {
    Ok(value
        .as_str()
        .ok_or_else(unavailable)?
        .chars()
        .take(500)
        .collect())
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
/// Seconds between checks for an album Lidarr is still loading.
const ALBUM_WAIT: i64 = 15;
/// Runs after which an album is searched for even without a known length, about ten minutes.
const ALBUM_WAIT_RUNS: i64 = 40;

/// Returns false when the job queued itself again to wait for Lidarr to load an album.
pub(crate) async fn acquire(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<bool> {
    let key = job.payload["request_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid acquisition job"))?
        .to_owned();
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    let result = perform(state, &key, job.attempts < ALBUM_WAIT_RUNS).await;
    // An added title appears in browse lists as soon as its manager reports it.
    super::status::refresh_soon(state);
    match result {
        Ok(true) => Ok(true),
        Ok(false) => {
            storage::wait(job.id.clone(), ALBUM_WAIT, &state.db).await?;
            Ok(false)
        }
        Err(error) => {
            let message = error.2;
            storage::acquire(key, message, &state.db).await?;
            anyhow::bail!("Acquisition needs attention; review its request status")
        }
    }
}
/// Whether Lidarr knows how long the album runs. Its size check rejects every release of an
/// album without a length, and a newly added album only gets its tracks in the refresh
/// Lidarr runs after the add.
fn album_ready(album: &Value) -> bool {
    let any = album["anyReleaseOk"].as_bool().unwrap_or(false);
    album["releases"].as_array().is_some_and(|releases| {
        releases.iter().any(|release| {
            (any || release["monitored"] == true) && release["duration"].as_i64() > Some(0)
        })
    })
}
/// Monitors an artist for one requested album without taking on its future albums.
async fn monitor_artist(c: &Connection<'_>, mut artist: Value) -> Result<Value> {
    if artist["monitored"] == true {
        return Ok(artist);
    }
    let id = artist["id"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)?;
    artist["monitored"] = json!(true);
    artist["monitorNewItems"] = json!("none");
    c.call(
        reqwest::Method::PUT,
        &format!("artist/{id}"),
        &[],
        Some(artist),
    )
    .await
}
/// Monitors an artist's future albums, adding the artist to Lidarr when it lacks it.
async fn monitor_releases(
    state: &AppState,
    key: &str,
    c: &Connection<'_>,
    defaults: &Defaults,
    artist_id: &str,
) -> Result<bool> {
    let existing = super::music::lidarr_artist(c, artist_id).await?;
    update(state, key, RequestState::Adding, None).await?;
    let artist = match existing {
        Some(artist) if artist["monitored"] == true && artist["monitorNewItems"] == "all" => artist,
        Some(mut artist) => {
            let id = artist["id"]
                .as_i64()
                .filter(|v| *v > 0)
                .ok_or_else(unavailable)?;
            artist["monitored"] = json!(true);
            artist["monitorNewItems"] = json!("all");
            c.call(
                reqwest::Method::PUT,
                &format!("artist/{id}"),
                &[],
                Some(artist),
            )
            .await?
        }
        None => {
            let mut artist = super::music::artist_lookup(c, artist_id)
                .await?
                .ok_or_else(ApiError::not_found)?;
            artist["rootFolderPath"] = json!(defaults.root_folder);
            artist["qualityProfileId"] = json!(defaults.quality_profile);
            artist["metadataProfileId"] = json!(defaults.metadata_profile);
            artist["monitored"] = json!(true);
            artist["monitorNewItems"] = json!("all");
            // Albums already out stay unmonitored; each later one is searched for on release.
            artist["addOptions"] = json!({"monitor":"future","searchForMissingAlbums":false});
            c.call(reqwest::Method::POST, "artist", &[], Some(artist))
                .await?
        }
    };
    let manager = artist["id"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)?;
    update(state, key, RequestState::Requested, Some(manager)).await?;
    Ok(true)
}
/// Returns false when `wait` allows it and Lidarr is still loading the album.
async fn perform(state: &AppState, key: &str, wait: bool) -> Result<bool> {
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
        return Ok(true);
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
    if row.target == RequestTarget::Artist {
        return monitor_releases(state, key, &c, &defaults, &row.external_id).await;
    }
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
            let existing = super::music::lidarr_artist(&c, &artist_id).await?;
            let artist = match existing {
                // Before the add: the refresh the add starts writes the artist back as it
                // read it.
                Some(a) if defaults.monitored => monitor_artist(&c, a).await?,
                Some(a) => a,
                None => {
                    let mut artist = item["artist"].clone();
                    artist["rootFolderPath"] = json!(defaults.root_folder);
                    artist["qualityProfileId"] = json!(defaults.quality_profile);
                    artist["metadataProfileId"] = json!(defaults.metadata_profile);
                    artist["monitored"] = json!(defaults.monitored);
                    artist["monitorNewItems"] = json!("none");
                    // Lidarr applies these after its first scan of the artist. "none" also
                    // unmonitors the artist and every album, the requested one included.
                    artist["addOptions"] = if defaults.monitored {
                        json!({"albumsToMonitor":[row.external_id],"searchForMissingAlbums":false})
                    } else {
                        json!({"monitor":"none","searchForMissingAlbums":false})
                    };
                    artist
                }
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
    let ready = s.kind != "lidarr" || album_ready(&item);
    if s.kind == "lidarr" && defaults.monitored {
        let artist_id = item["artistId"]
            .as_i64()
            .or_else(|| item["artist"]["id"].as_i64())
            .filter(|v| *v > 0)
            .ok_or_else(unavailable)?;
        monitor_artist(&c, c.get(&format!("artist/{artist_id}")).await?).await?;
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
        return Ok(true);
    }
    if !ready && wait {
        // The next run finds the album already added and checks it again.
        update(state, key, RequestState::Adding, Some(manager)).await?;
        return Ok(false);
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
    Ok(true)
}
