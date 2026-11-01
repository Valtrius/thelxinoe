//! Album discovery for music requests: recent albums of Lidarr's artists,
//! ListenBrainz charts, and Cover Art Archive covers relayed same-origin.
use super::*;
use axum::{
    extract::Query,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "music_tests.rs"]
mod tests;

const LISTENBRAINZ: &str = "https://api.listenbrainz.org/1";
const COVERS: &str = "https://coverartarchive.org/release-group";
const FEED_LENGTH: usize = 30;
/// Charts change daily at most; one fetch serves every viewer for an hour.
const CHART_TTL: Duration = Duration::from_secs(3600);

pub(super) struct Runtime {
    http: reqwest::Client,
    charts: tokio::sync::Mutex<HashMap<&'static str, (Instant, Vec<Value>)>>,
    covers: tokio::sync::Semaphore,
}
impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .user_agent(concat!("Thelxinoe/", env!("CARGO_PKG_VERSION")))
                // The archive serves covers from its own hosts.
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let archive = attempt.url().host_str().is_some_and(|host| {
                        host == "archive.org" || host.ends_with(".archive.org")
                    });
                    if archive && attempt.previous().len() < 5 {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()?,
            charts: Default::default(),
            covers: tokio::sync::Semaphore::new(8),
        })
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/acquisition/music/covers/{id}", get(cover))
        .route("/api/v1/acquisition/music/{feed}", get(feed))
}

/// An album as the music tab shows it; `id` is its MusicBrainz release group.
pub(super) fn album(id: &str, title: &str, artist: Option<&str>, date: Option<&str>) -> Value {
    json!({
        "external_id": id,
        "title": title,
        "artist": artist,
        "year": date.and_then(|date| date.get(..4)?.parse::<i64>().ok()),
        "cover": format!("/api/v1/acquisition/music/covers/{id}"),
    })
}

fn release_group(id: Option<&str>) -> Option<&str> {
    id.filter(|id| uuid::Uuid::parse_str(id).is_ok())
}

/// Lidarr's calendar rows, newest first.
fn released(rows: &[Value]) -> Vec<Value> {
    let mut rows = rows.iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b["releaseDate"].as_str().cmp(&a["releaseDate"].as_str()));
    rows.into_iter()
        .filter_map(|row| {
            Some(album(
                release_group(row["foreignAlbumId"].as_str())?,
                row["title"].as_str()?,
                row["artist"]["artistName"].as_str(),
                row["releaseDate"].as_str(),
            ))
        })
        .take(FEED_LENGTH)
        .collect()
}

fn distinct(albums: impl Iterator<Item = Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    albums
        .filter(|album| seen.insert(album["external_id"].to_string()))
        .take(FEED_LENGTH)
        .collect()
}

/// The week's most listened release groups. A group appears once per release.
fn trending(body: &Value) -> Vec<Value> {
    distinct(
        body["payload"]["release_groups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some(album(
                    release_group(row["release_group_mbid"].as_str())?,
                    row["release_group_name"].as_str()?,
                    row["artist_name"].as_str(),
                    None,
                ))
            }),
    )
}

/// Recent albums, most listened first; singles and EPs are left out.
fn fresh(body: &Value) -> Vec<Value> {
    let mut rows = body["payload"]["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["release_group_primary_type"] == "Album")
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b["listen_count"]
            .as_i64()
            .cmp(&a["listen_count"].as_i64())
            .then_with(|| b["release_date"].as_str().cmp(&a["release_date"].as_str()))
    });
    distinct(rows.into_iter().filter_map(|row| {
        Some(album(
            release_group(row["release_group_mbid"].as_str())?,
            row["release_name"].as_str()?,
            row["artist_credit_name"].as_str(),
            row["release_date"].as_str(),
        ))
    }))
}

fn listenbrainz_unavailable() -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "dependency_unavailable",
        "ListenBrainz is unavailable. Try again later.".into(),
    )
}

async fn chart(state: &AppState, name: &'static str) -> Result<Vec<Value>> {
    let music = &state.managers.music;
    // Held across the fetch so concurrent viewers share one request.
    let mut charts = music.charts.lock().await;
    if let Some((at, items)) = charts.get(name)
        && at.elapsed() < CHART_TTL
    {
        return Ok(items.clone());
    }
    let (url, parse): (String, fn(&Value) -> Vec<Value>) = if name == "trending" {
        (
            format!("{LISTENBRAINZ}/stats/sitewide/release-groups?count=100&range=week"),
            trending,
        )
    } else {
        (
            format!("{LISTENBRAINZ}/explore/fresh-releases/?days=14&past=true&future=false"),
            fresh,
        )
    };
    let fetched = async {
        let response = music
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| listenbrainz_unavailable())?;
        // Statistics that are still being computed come back empty.
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(Vec::new());
        }
        if !response.status().is_success() {
            return Err(listenbrainz_unavailable());
        }
        let body = bounded(response, 32 * 1024 * 1024).await?;
        let body = serde_json::from_slice(&body).map_err(|_| listenbrainz_unavailable())?;
        Ok(parse(&body))
    }
    .await;
    match fetched {
        Ok(items) => {
            charts.insert(name, (Instant::now(), items.clone()));
            Ok(items)
        }
        // A stale chart beats none while ListenBrainz is down.
        Err(error) => charts
            .get(name)
            .map(|(_, items)| items.clone())
            .ok_or(error),
    }
}

#[derive(Deserialize)]
struct Feed {
    service_id: String,
}

async fn feed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(input): Query<Feed>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let s = service(&state, &input.service_id).await?;
    if s.kind != "lidarr" {
        return Err(ApiError::bad("Choose a Lidarr service"));
    }
    let mut items = match name.as_str() {
        "artists" => {
            let c = Connection::open(&state, &s).await?;
            let now = chrono::Utc::now();
            let date =
                |at: chrono::DateTime<chrono::Utc>| at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let rows = c
                .fetch::<Vec<Value>>(
                    "calendar",
                    &[
                        ("start", date(now - chrono::Duration::days(90))),
                        ("end", date(now)),
                        ("unmonitored", "true".into()),
                        ("includeArtist", "true".into()),
                    ],
                )
                .await?;
            released(&rows)
        }
        "trending" => chart(&state, "trending").await?,
        "fresh" => chart(&state, "fresh").await?,
        _ => return Err(ApiError::not_found()),
    };
    super::requests::decorate(&state, &p.user.id, &s.id, &mut items).await?;
    Ok(Json(json!({ "items": items })))
}

fn cover_unavailable() -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "dependency_unavailable",
        "The Cover Art Archive is unavailable".into(),
    )
}

async fn cover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    security::principal(&state, &headers).await?;
    let id = release_group(Some(&id)).ok_or_else(|| ApiError::bad("Invalid album identifier"))?;
    let music = &state.managers.music;
    let _slot = music
        .covers
        .acquire()
        .await
        .map_err(|_| cover_unavailable())?;
    let response = music
        .http
        .get(format!("{COVERS}/{id}/front-250"))
        .send()
        .await
        .map_err(|_| cover_unavailable())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found());
    }
    if !response.status().is_success() {
        return Err(cover_unavailable());
    }
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|kind| ["image/jpeg", "image/png", "image/gif", "image/webp"].contains(kind))
        .ok_or_else(cover_unavailable)?
        .to_owned();
    let bytes = bounded(response, 4 * 1024 * 1024).await?;
    Ok((
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "private, max-age=604800".to_owned()),
        ],
        bytes,
    )
        .into_response())
}
