//! Administrator actions on the Sonarr episodes of a series: searches, monitoring and
//! interactive release grabs. The series is addressed like its seasons, by library id
//! or `tmdb:`/`tvdb:` reference, and its episodes by Sonarr id.

#[path = "../storage/managers/episodes.rs"]
mod storage;

use super::status::Series;
use super::*;
use axum::extract::Query;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/series/{id}/search", post(search))
        .route(
            "/api/v1/admin/series/{id}/monitor",
            axum::routing::put(monitor),
        )
        .route("/api/v1/admin/series/{id}/episodes/{episode}", get(episode))
        .route(
            "/api/v1/admin/series/{id}/releases",
            get(releases).post(grab),
        )
}

/// The most episodes one request may name; a whole season of a long-running show fits.
const EPISODES: usize = 500;

fn episodes(series: &Series, ids: &[i64]) -> Result<Vec<i64>> {
    let distinct = ids.iter().collect::<std::collections::BTreeSet<_>>();
    if ids.is_empty() || ids.len() > EPISODES || distinct.len() != ids.len() {
        return Err(ApiError::bad(format!(
            "Choose between 1 and {EPISODES} distinct episodes"
        )));
    }
    if !ids
        .iter()
        .all(|id| series.episodes.iter().any(|e| e.id == *id))
    {
        return Err(ApiError::conflict(
            "Sonarr no longer lists an episode of this series",
        ));
    }
    Ok(ids.to_vec())
}
fn season(series: &Series, number: i64) -> Result<i64> {
    if series.seasons.contains(&number) {
        Ok(number)
    } else {
        Err(ApiError::conflict("Season no longer exists in Sonarr"))
    }
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    episodes: Vec<i64>,
    season: Option<i64>,
    /// With a season: only its aired, wanted episodes that are neither here nor
    /// downloading.
    #[serde(default)]
    missing: bool,
    /// Monitor the episodes first, so Sonarr keeps what it finds and upgrades it.
    #[serde(default)]
    monitor: bool,
}
#[derive(Debug, PartialEq)]
enum Command {
    Episodes(Vec<i64>),
    Season(i64),
}
fn command(series: &Series, input: &Search) -> Result<Command> {
    match (input.episodes.is_empty(), input.season) {
        (false, None) if !input.missing => {
            Ok(Command::Episodes(episodes(series, &input.episodes)?))
        }
        (true, Some(number)) if !input.monitor => {
            let number = season(series, number)?;
            if !input.missing {
                return Ok(Command::Season(number));
            }
            let missing = series
                .episodes
                .iter()
                .filter(|e| e.season == number && e.missing)
                .map(|e| e.id)
                .collect::<Vec<_>>();
            if missing.is_empty() {
                return Err(ApiError::conflict("No episode of this season is missing"));
            }
            Ok(Command::Episodes(missing))
        }
        _ => Err(ApiError::bad(
            "Search episodes, a season, or a season's missing episodes",
        )),
    }
}
async fn run(c: &Connection<'_>, series: &Series, command: &Command, monitor: bool) -> Result<()> {
    let body = match command {
        Command::Episodes(ids) => {
            if monitor {
                c.call(
                    reqwest::Method::PUT,
                    "episode/monitor",
                    &[],
                    Some(json!({"episodeIds":ids,"monitored":true})),
                )
                .await?;
            }
            json!({"name":"EpisodeSearch","episodeIds":ids})
        }
        Command::Season(number) => {
            json!({"name":"SeasonSearch","seriesId":series.id,"seasonNumber":number})
        }
    };
    c.call(reqwest::Method::POST, "command", &[], Some(body))
        .await
        .map(drop)
}
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Search>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let series = super::status::series(&state, &id).await?;
    let command = command(&series, &input)?;
    let _guard = state.managers.guard.service("sonarr").await;
    let s = service(&state, &series.service).await?;
    run(
        &Connection::open(&state, &s).await?,
        &series,
        &command,
        input.monitor,
    )
    .await?;
    storage::audit(&state.db, p, "series.search", id).await?;
    super::status::refresh_soon(&state);
    Ok(Json(json!({"episodes":match command {
        Command::Episodes(ids) => Some(ids.len()),
        Command::Season(_) => None,
    }})))
}

#[derive(Deserialize)]
struct Monitor {
    episodes: Vec<i64>,
    monitored: bool,
}
async fn monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Monitor>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let series = super::status::series(&state, &id).await?;
    let ids = episodes(&series, &input.episodes)?;
    let _guard = state.managers.guard.service("sonarr").await;
    let s = service(&state, &series.service).await?;
    Connection::open(&state, &s)
        .await?
        .call(
            reqwest::Method::PUT,
            "episode/monitor",
            &[],
            Some(json!({"episodeIds":ids,"monitored":input.monitored})),
        )
        .await?;
    storage::audit(&state.db, p, "series.monitor", id).await?;
    super::status::refresh_soon(&state);
    Ok(Json(json!({"saved":true})))
}

fn time(value: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|at| at.timestamp())
}

/// What Sonarr knows of an episode right now; its cached list is only reread when the
/// series changes, while searches change when it was last searched.
async fn episode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, episode)): Path<(String, i64)>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let series = super::status::series(&state, &id).await?;
    episodes(&series, &[episode])?;
    let s = service(&state, &series.service).await?;
    let current = Connection::open(&state, &s)
        .await?
        .get(&format!("episode/{episode}"))
        .await?;
    if current["seriesId"].as_i64() != Some(series.id) {
        return Err(ApiError::conflict("Sonarr episode identity changed"));
    }
    Ok(Json(json!({
        "monitored":current["monitored"].as_bool().unwrap_or(false),
        "last_search":time(&current["lastSearchTime"])
    })))
}

#[derive(Deserialize)]
struct ReleaseTarget {
    episode: Option<i64>,
    season: Option<i64>,
}
fn release_query(series: &Series, target: &ReleaseTarget) -> Result<Vec<(&'static str, String)>> {
    match (target.episode, target.season) {
        (Some(episode), None) => {
            episodes(series, &[episode])?;
            Ok(vec![("episodeId", episode.to_string())])
        }
        (None, Some(number)) => Ok(vec![
            ("seriesId", series.id.to_string()),
            ("seasonNumber", season(series, number)?.to_string()),
        ]),
        _ => Err(ApiError::bad("Search releases for an episode or a season")),
    }
}
/// Sonarr lists rejections as text, or as objects in some versions.
fn rejections(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            r.as_str()
                .or_else(|| r["reason"].as_str())
                .or_else(|| r["message"].as_str())
        })
        .map(str::to_owned)
        .collect()
}
fn release(row: &Value) -> Value {
    let rejected = rejections(&row["rejections"]);
    json!({
        "guid":row["guid"],"indexer_id":row["indexerId"],"indexer":row["indexer"],
        "title":row["title"],"size":row["size"],"protocol":row["protocol"],
        "quality":row.pointer("/quality/quality/name"),"score":row["customFormatScore"],
        "seeders":row["seeders"],"leechers":row["leechers"],
        "published":time(&row["publishDate"]),"full_season":row["fullSeason"].as_bool().unwrap_or(false),
        "approved":row["approved"].as_bool().unwrap_or(false) && rejected.is_empty(),
        "rejections":rejected
    })
}
/// Every release Sonarr's indexers return, in the order Sonarr ranks them.
async fn releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(target): Query<ReleaseTarget>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let series = super::status::series(&state, &id).await?;
    let query = release_query(&series, &target)?;
    let s = service(&state, &series.service).await?;
    // Indexer searches can outlast one ordinary request, so the list is read like a
    // whole library.
    let rows = Connection::open(&state, &s)
        .await?
        .fetch::<Vec<Value>>("release", &query)
        .await?;
    Ok(Json(json!({
        "items":rows.iter().take(EPISODES).map(release).collect::<Vec<_>>()
    })))
}

#[derive(Deserialize)]
struct Grab {
    guid: String,
    indexer_id: i64,
}
/// Send a release from the latest search to the download client. Sonarr only accepts
/// releases still in its search cache, and decides itself which episodes they hold.
async fn grab(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Grab>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if input.guid.is_empty() || input.guid.len() > 2000 || input.indexer_id <= 0 {
        return Err(ApiError::bad("Select a release returned by Sonarr"));
    }
    let series = super::status::series(&state, &id).await?;
    let _guard = state.managers.guard.service("sonarr").await;
    let s = service(&state, &series.service).await?;
    // No automatic retry: an interrupted grab can have reached the download client.
    Connection::open(&state, &s)
        .await?
        .call(
            reqwest::Method::POST,
            "release",
            &[],
            Some(json!({"guid":input.guid,"indexerId":input.indexer_id})),
        )
        .await?;
    storage::audit(&state.db, p, "series.grab", id).await?;
    super::status::refresh_soon(&state);
    Ok(Json(json!({"submitted":true})))
}

#[cfg(test)]
#[path = "episodes_tests.rs"]
mod tests;
