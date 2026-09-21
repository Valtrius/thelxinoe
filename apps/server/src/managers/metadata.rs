use super::*;
use crate::grants;
use anyhow::{Context, bail};
use axum::{
    body::Body,
    extract::Query,
    http::Request,
    response::{IntoResponse, Response},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tower_http::services::ServeFile;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/metadata/search", get(search))
        .route("/api/v1/catalog/{id}/match", post(match_item))
        .route("/api/v1/catalog/{id}/refresh", post(refresh))
        .route(
            "/api/v1/catalog/{id}/manager-episodes",
            get(manager_episodes),
        )
        .route(
            "/api/v1/catalog/{id}/episode-mapping",
            axum::routing::put(map_episode),
        )
        .route("/api/v1/catalog/collections", get(collections))
        .route("/api/v1/catalog/{id}/artwork", get(artwork))
}

fn manager_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "movie" => Some("radarr"),
        "show" | "season" | "episode" => Some("sonarr"),
        "artist" | "album" | "track" => Some("lidarr"),
        _ => None,
    }
}

fn search_kind(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "movie" => Some(("movie/lookup", "tmdbId")),
        "show" => Some(("series/lookup", "tvdbId")),
        "artist" => Some(("artist/lookup", "foreignArtistId")),
        "album" => Some(("album/lookup", "foreignAlbumId")),
        _ => None,
    }
}

fn entity_kind(kind: &str) -> &'static str {
    match kind {
        "movie" => "movie",
        "show" | "season" | "episode" => "series",
        "artist" => "artist",
        _ => "album",
    }
}

fn external_id(kind: &str, row: &Value) -> Option<String> {
    match kind {
        "movie" => row["tmdbId"].as_u64().map(|v| v.to_string()),
        "show" | "season" | "episode" => row["tvdbId"].as_u64().map(|v| v.to_string()),
        "artist" => row["foreignArtistId"].as_str().map(str::to_owned),
        _ => row["foreignAlbumId"].as_str().map(str::to_owned),
    }
}

fn valid_external_id(kind: &str, value: &str) -> bool {
    if matches!(kind, "artist" | "album" | "track") {
        uuid::Uuid::parse_str(value).is_ok()
    } else {
        value.len() <= 12 && value.parse::<u64>().is_ok_and(|v| v > 0)
    }
}

async fn service_for_kind(state: &AppState, kind: &str) -> Result<Option<Service>> {
    let kind = kind.to_owned();
    let id = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT id FROM manager_services WHERE enabled=1 AND kind=?1",
                    [kind],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?;
    match id {
        Some(id) => Ok(Some(service(state, &id).await?)),
        None => Ok(None),
    }
}

#[derive(Deserialize)]
struct Search {
    q: String,
    kind: String,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Search>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    if query.q.trim().len() < 2 || query.q.len() > 200 {
        return Err(ApiError::bad("Enter between 2 and 200 characters"));
    }
    let manager = manager_kind(&query.kind)
        .filter(|_| !matches!(query.kind.as_str(), "season" | "episode" | "track"))
        .ok_or_else(|| ApiError::bad("Match a movie, show, artist or album"))?;
    let (path, _) =
        search_kind(&query.kind).ok_or_else(|| ApiError::bad("Unsupported media type"))?;
    let mut items = Vec::new();
    if let Some(service) = service_for_kind(&state, manager).await? {
        let connection = Connection::open(&state, &service).await?;
        let rows = connection
            .call(
                reqwest::Method::GET,
                path,
                &[("term", query.q.trim().to_owned())],
                None,
            )
            .await?;
        for row in rows.as_array().ok_or_else(unavailable)?.iter().take(50) {
            let Some(id) = external_id(&query.kind, row) else {
                continue;
            };
            let title = if query.kind == "artist" {
                row["artistName"].as_str()
            } else {
                row["title"].as_str()
            };
            let Some(title) = title else { continue };
            let year = row["year"].as_i64().map(|v| v.to_string()).or_else(|| {
                row["releaseDate"]
                    .as_str()
                    .and_then(|v| v.get(..4))
                    .map(str::to_owned)
            });
            items.push(json!({
                "service_id":service.id,
                "service_generation":service.generation,
                "source":service.name,
                "external_id":id,
                "title":title,
                "year":year,
                "overview":row["overview"].as_str().unwrap_or("")
            }));
        }
    }
    Ok(Json(json!({"items":items})))
}

#[derive(Deserialize)]
struct Match {
    service_id: String,
    service_generation: String,
    external_id: String,
}

async fn match_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(input): Json<Match>,
) -> Result<Json<Value>> {
    let principal = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let target = media_id.clone();
    let kind = state
        .db
        .call(move |db| {
            Ok(db
                .query_row("SELECT kind FROM media WHERE id=?1", [target], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?)
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    if !matches!(kind.as_str(), "movie" | "show" | "artist" | "album") {
        return Err(ApiError::bad("Match a movie, show, artist or album"));
    }
    let service = service(&state, &input.service_id).await?;
    if input.service_generation != service.generation
        || manager_kind(&kind) != Some(service.kind.as_str())
        || !valid_external_id(&kind, &input.external_id)
    {
        return Err(ApiError::bad(
            "Manager result does not match this media type",
        ));
    }
    let job = thelxinoe_jobs::Queue(state.db.clone())
        .enqueue(
            "metadata.match".into(),
            json!({
                "media_id":media_id,
                "kind":kind,
                "service_id":input.service_id,
                "service_generation":service.generation,
                "external_id":input.external_id,
                "actor_id":principal.user.id
            }),
            format!("metadata:{media_id}:{}", id()),
        )
        .await?;
    Ok(Json(json!({"job_id":job})))
}

async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    let principal = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let target = media_id.clone();
    let kind = state
        .db
        .call(move |db| {
            Ok(db
                .query_row("SELECT kind FROM media WHERE id=?1", [target], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?)
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    if manager_kind(&kind).is_none() {
        return Err(ApiError::bad("Refresh local movie, show or music metadata"));
    }
    let job = thelxinoe_jobs::Queue(state.db.clone())
        .enqueue(
            "metadata.refresh".into(),
            json!({"media_id":media_id,"kind":kind,"actor_id":principal.user.id}),
            format!("refresh:{media_id}:{}", now() / 10),
        )
        .await?;
    Ok(Json(json!({"job_id":job})))
}

async fn exact_entity(connection: &Connection<'_>, kind: &str, external: &str) -> Result<Value> {
    let endpoint = entity_kind(kind);
    let existing = connection.get(endpoint).await?;
    if let Some(row) = existing
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| external_id(kind, row).as_deref() == Some(external))
    {
        return Ok(row.clone());
    }
    let (lookup, _) = search_kind(kind).ok_or_else(|| ApiError::bad("Unsupported media type"))?;
    let prefix = match kind {
        "movie" => "tmdb",
        "show" => "tvdb",
        _ => "mbid",
    };
    let rows = connection
        .call(
            reqwest::Method::GET,
            lookup,
            &[("term", format!("{prefix}:{external}"))],
            None,
        )
        .await?;
    rows.as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| external_id(kind, row).as_deref() == Some(external))
        })
        .cloned()
        .ok_or_else(ApiError::not_found)
}

#[derive(Clone)]
struct Binding {
    service_id: String,
    external_id: String,
    entity_id: Option<i64>,
}

async fn stored_binding(state: &AppState, media: &str) -> anyhow::Result<Option<Binding>> {
    let media = media.to_owned();
    state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT b.service_id,b.external_id,b.manager_entity_id FROM metadata_bindings b JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1",
                    [media],
                    |r| Ok(Binding { service_id:r.get(0)?, external_id:r.get(1)?, entity_id:r.get(2)? }),
                )
                .optional()?)
        })
        .await
}

async fn inferred_binding(
    state: &AppState,
    media: &str,
    manager: &str,
) -> anyhow::Result<Option<Binding>> {
    let media = media.to_owned();
    let manager = manager.to_owned();
    state.db.call(move |db| {
        let rows=db.prepare("WITH RECURSIVE tree(id) AS (SELECT ?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT DISTINCT b.service_id,b.external_id,b.entity_id FROM tree JOIN media_sources ms ON ms.media_id=tree.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind=?2 AND b.service_generation=s.generation WHERE b.checked_at>=?3 ORDER BY b.service_id,b.external_id,b.entity_id")?.query_map(params![media,manager,now()-60],|r|Ok(Binding{service_id:r.get(0)?,external_id:r.get(1)?,entity_id:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(if rows.len()==1 { rows.into_iter().next() } else { None })
    }).await
}

async fn ancestor(
    state: &AppState,
    media: &str,
    target_kind: &str,
) -> anyhow::Result<Option<String>> {
    let media = media.to_owned();
    let target_kind = target_kind.to_owned();
    state.db.call(move |db| {
        Ok(db.query_row("WITH RECURSIVE parents(id,parent_id,kind) AS (SELECT id,parent_id,kind FROM media WHERE id=?1 UNION ALL SELECT m.id,m.parent_id,m.kind FROM media m JOIN parents p ON p.parent_id=m.id) SELECT id FROM parents WHERE kind=?2 LIMIT 1",params![media,target_kind],|r|r.get::<_,String>(0)).optional()?)
    }).await
}

async fn root_target(
    state: &AppState,
    media: &str,
    kind: &str,
) -> anyhow::Result<(String, String)> {
    match kind {
        "season" | "episode" => Ok((
            ancestor(state, media, "show")
                .await?
                .context("Missing parent show")?,
            "show".into(),
        )),
        "track" => Ok((
            ancestor(state, media, "album")
                .await?
                .context("Missing parent album")?,
            "album".into(),
        )),
        _ => Ok((media.to_owned(), kind.to_owned())),
    }
}

fn normalized_genres(row: &Value) -> Value {
    json!(
        row["genres"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|name| json!({"name":name}))
            .collect::<Vec<_>>()
    )
}

fn normalize(kind: &str, row: &Value) -> Value {
    let title = if kind == "artist" {
        &row["artistName"]
    } else {
        &row["title"]
    };
    let mut value = json!({
        "title":title,
        "overview":row["overview"],
        "year":row["year"],
        "runtime":row["runtime"],
        "status":row["status"],
        "genres":normalized_genres(row)
    });
    if let Some(rating) = row
        .pointer("/ratings/tmdb/value")
        .or_else(|| row.pointer("/ratings/value"))
        .and_then(Value::as_f64)
    {
        value["vote_average"] = json!(rating);
    }
    match kind {
        "movie" => {
            value["release_date"] = row["releaseDate"].clone();
            value["tmdb_id"] = row["tmdbId"].clone();
            value["imdb_id"] = row["imdbId"].clone();
            if row["collection"]["tmdbId"].as_i64().is_some() {
                value["belongs_to_collection"] = json!({
                    "id":row["collection"]["tmdbId"],
                    "name":row["collection"]["title"]
                });
            }
        }
        "show" => {
            value["first_air_date"] = row["firstAired"].clone();
            value["tvdb_id"] = row["tvdbId"].clone();
            value["tmdb_id"] = row["tmdbId"].clone();
            value["imdb_id"] = row["imdbId"].clone();
        }
        "artist" => {
            value["name"] = row["artistName"].clone();
            value["musicbrainz_artist_id"] = row["foreignArtistId"].clone();
        }
        "album" => {
            value["release_date"] = row["releaseDate"].clone();
            value["musicbrainz_release_group_id"] = row["foreignAlbumId"].clone();
            if let Some(release_id) = row["releases"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|release| release["monitored"].as_bool() == Some(true))
                .and_then(|release| release["foreignReleaseId"].as_str())
            {
                value["musicbrainz_release_id"] = json!(release_id);
            }
        }
        "track" => {
            value["duration_ms"] = row["duration"].clone();
            value["track_number"] = row["trackNumber"].clone();
            value["absolute_track_number"] = row["absoluteTrackNumber"].clone();
            value["medium_number"] = row["mediumNumber"].clone();
            value["explicit"] = row["explicit"].clone();
            value["musicbrainz_recording_id"] = row["foreignRecordingId"].clone();
            value["musicbrainz_release_track_id"] = row["foreignTrackId"].clone();
        }
        _ => {}
    }
    value
}

async fn cache_manager_artwork(
    state: &AppState,
    connection: &Connection<'_>,
    media_id: &str,
    row: &Value,
) -> anyhow::Result<bool> {
    if !media_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        bail!("Invalid media ID");
    }
    let local = ["poster", "cover", "headshot", "fanart"]
        .into_iter()
        .find_map(|kind| {
            row["images"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|image| {
                    (image["coverType"].as_str() == Some(kind))
                        .then(|| image["url"].as_str())
                        .flatten()
                })
        });
    let dir = state.config.cache.join("artwork");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(media_id);
    let Some(local) = local else {
        return Ok(path.is_file());
    };
    if !local.starts_with('/') || local.starts_with("//") || local.contains("..") {
        return Ok(path.is_file());
    }
    let response = state
        .managers
        .http
        .get(format!("{}{}", connection.base, local))
        .header("X-Api-Key", &connection.key)
        .send()
        .await?;
    if !response.status().is_success() {
        return Ok(path.is_file());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 10 * 1024 * 1024 {
            bail!("Artwork is too large");
        }
        bytes.extend(chunk);
    }
    if !bytes.starts_with(&[0xff, 0xd8, 0xff]) && !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(path.is_file());
    }
    if fs2::available_space(&dir)? < 512 * 1024 * 1024 {
        bail!("Insufficient artwork cache space");
    }
    let temp = dir.join(format!("{media_id}.tmp"));
    tokio::fs::write(&temp, bytes).await?;
    #[cfg(windows)]
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    tokio::fs::rename(temp, path).await?;
    Ok(true)
}

async fn update_metadata(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    media_id: &str,
    kind: &str,
    external: &str,
    row: &Value,
    refreshed_at: i64,
    actor: Option<&str>,
) -> anyhow::Result<()> {
    let identity_media = media_id.to_owned();
    let identity_service = service.id.clone();
    let identity_generation = service.generation.clone();
    let identity_external = external.to_owned();
    let same_identity = state.db.call(move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata_bindings WHERE media_id=?1 AND service_id=?2 AND service_generation=?3 AND external_id=?4)",
            params![identity_media,identity_service,identity_generation,identity_external],
            |r| r.get::<_,bool>(0),
        )?)
    }).await?;
    if !same_identity {
        let _ = tokio::fs::remove_file(state.config.cache.join("artwork").join(media_id)).await;
    }
    let mut metadata = normalize(kind, row);
    metadata["artwork_cached"] =
        json!(cache_manager_artwork(state, connection, media_id, row).await?);
    metadata["refreshed_at"] = json!(refreshed_at);
    let media = media_id.to_owned();
    let service_id = service.id.clone();
    let generation = service.generation.clone();
    let external = external.to_owned();
    let manager_entity_id = row["id"].as_i64().filter(|v| *v > 0);
    let stored = metadata.to_string();
    let actor = actor.map(str::to_owned);
    state.db.call(move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE media SET metadata=?1 WHERE id=?2",params![stored,media])?;
        tx.execute("INSERT INTO metadata_bindings(media_id,service_id,service_generation,external_id,manager_entity_id,refreshed_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(media_id) DO UPDATE SET service_id=excluded.service_id,service_generation=excluded.service_generation,external_id=excluded.external_id,manager_entity_id=excluded.manager_entity_id,refreshed_at=excluded.refreshed_at",params![media,service_id,generation,external,manager_entity_id,refreshed_at])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.refresh',?2,?3)",params![actor,media,now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

async fn update_show(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    media_id: &str,
    external: &str,
    mut row: Value,
    actor: Option<&str>,
) -> anyhow::Result<()> {
    let refreshed_at = now();
    let previous_media = media_id.to_owned();
    let current_service = service.id.clone();
    let current_generation = service.generation.clone();
    let current_external = external.to_owned();
    let binding_changed = state.db.call(move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata_bindings WHERE media_id=?1 AND (service_id<>?2 OR service_generation<>?3 OR external_id<>?4))",
            params![previous_media,current_service,current_generation,current_external],
            |r| r.get::<_,bool>(0),
        )?)
    }).await?;
    if binding_changed {
        let changed_show = media_id.to_owned();
        state.db.call(move |db| {
            let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM manager_episode_mappings WHERE media_id IN (SELECT ep.id FROM media ep JOIN media season ON season.id=ep.parent_id WHERE season.parent_id=?1)",[&changed_show])?;
            tx.commit()?;
            Ok(())
        }).await?;
    }
    let entity = row["id"].as_i64().filter(|v| *v > 0);
    let episodes = if let Some(series) = entity {
        connection
            .call(
                reqwest::Method::GET,
                "episode",
                &[("seriesId", series.to_string())],
                None,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.2))?
            .as_array()
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut seasons: BTreeMap<i64, (usize, Option<String>)> = BTreeMap::new();
    for episode in &episodes {
        let Some(season) = episode["seasonNumber"].as_i64() else {
            continue;
        };
        let entry = seasons.entry(season).or_default();
        entry.0 += 1;
        if let Some(date) = episode["airDate"].as_str() {
            if entry.1.as_deref().is_none_or(|current| date < current) {
                entry.1 = Some(date.to_owned());
            }
        }
    }
    row["seasons"] = json!(
        seasons
            .into_iter()
            .map(|(season, (count, air_date))| json!({
                "season_number":season,
                "episode_count":count,
                "air_date":air_date
            }))
            .collect::<Vec<_>>()
    );
    update_metadata(
        state,
        service,
        connection,
        media_id,
        "show",
        external,
        &row,
        refreshed_at,
        actor,
    )
    .await?;
    let service_id = service.id.clone();
    let service_generation = service.generation.clone();
    let series_external = external.to_owned();
    let show_id = media_id.to_owned();
    let saved = episodes
        .into_iter()
        .filter_map(|episode| {
            Some((
                episode["id"].as_i64()?,
                episode["seasonNumber"].as_i64()?,
                episode["episodeNumber"].as_i64()?,
                json!({
                    "name":episode["title"],
                    "overview":episode["overview"],
                    "air_date":episode["airDate"],
                    "air_date_utc":episode["airDateUtc"],
                    "runtime":episode["runtime"],
                    "tvdb_id":episode["tvdbId"],
                    "finale_type":episode["finaleType"]
                })
                .to_string(),
            ))
        })
        .collect::<Vec<_>>();
    state.db.call(move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (episode,season,number,metadata) in saved {
            tx.execute("INSERT INTO manager_episodes(service_id,service_generation,series_external_id,manager_episode_id,season_number,episode_number,metadata,refreshed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(service_id,service_generation,manager_episode_id) DO UPDATE SET series_external_id=excluded.series_external_id,season_number=excluded.season_number,episode_number=excluded.episode_number,metadata=excluded.metadata,refreshed_at=excluded.refreshed_at",params![service_id,service_generation,series_external,episode,season,number,metadata,refreshed_at])?;
        }
        tx.execute("UPDATE manager_episode_mappings SET state='unresolved' WHERE service_id=?1 AND service_generation=?2 AND manager_episode_id IN (SELECT manager_episode_id FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND series_external_id=?3 AND refreshed_at<>?4)",params![service_id,service_generation,series_external,refreshed_at])?;
        let rows=tx.prepare("SELECT ep.id,b.members FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media_sources ms ON ms.media_id=ep.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE ep.kind='episode' AND season.parent_id=?1 AND b.service_id=?2 AND b.service_generation=?3 AND b.checked_at>=?4")?.query_map(params![show_id,service_id,service_generation,now()-60],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut exact=BTreeMap::<String,BTreeSet<i64>>::new();
        for (episode,members) in rows {
            for member in serde_json::from_str::<Vec<i64>>(&members).unwrap_or_default() { exact.entry(episode.clone()).or_default().insert(member); }
        }
        for (episode,members) in exact {
            if members.len()!=1 || tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episode_mappings WHERE media_id=?1)",[&episode],|r|r.get::<_,bool>(0))? {continue;}
            let member=*members.first().unwrap();
            let current=tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND series_external_id=?3 AND manager_episode_id=?4 AND refreshed_at=?5)",params![service_id,service_generation,series_external,member,refreshed_at],|r|r.get::<_,bool>(0))?;
            if current {tx.execute("INSERT INTO manager_episode_mappings VALUES (?1,?2,?3,?4,'confirmed')",params![episode,service_id,service_generation,member])?;}
        }
        tx.execute("UPDATE manager_episode_mappings SET state=CASE WHEN media_id IN (SELECT media_id FROM manager_episode_mappings GROUP BY media_id HAVING COUNT(*)>1) OR (service_id,service_generation,manager_episode_id) IN (SELECT service_id,service_generation,manager_episode_id FROM manager_episode_mappings GROUP BY service_id,service_generation,manager_episode_id HAVING COUNT(*)>1) THEN 'complex' ELSE 'confirmed' END WHERE service_id=?1 AND service_generation=?2 AND state<>'unresolved'",params![service_id,service_generation])?;
        tx.execute("UPDATE media SET metadata=COALESCE((SELECT e.metadata FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id AND b.refreshed_at=e.refreshed_at WHERE m.media_id=media.id AND m.state='confirmed'),'{}') WHERE kind='episode' AND parent_id IN (SELECT id FROM media WHERE parent_id=?1)",[show_id])?;
        tx.commit()?;
        Ok(())
    }).await
}

async fn update_album_tracks(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    album_id: &str,
    row: &Value,
) -> anyhow::Result<()> {
    let Some(entity) = row["id"].as_i64().filter(|id| *id > 0) else {
        let album = album_id.to_owned();
        return state
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE media SET metadata='{}' WHERE kind='track' AND parent_id=?1",
                    [album],
                )?;
                Ok(())
            })
            .await;
    };
    let tracks = connection
        .call(
            reqwest::Method::GET,
            "track",
            &[("albumId", entity.to_string())],
            None,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    let by_id = tracks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|track| track["id"].as_i64().map(|id| (id, track.clone())))
        .collect::<HashMap<_, _>>();
    let album = album_id.to_owned();
    let service_id = service.id.clone();
    let generation = service.generation.clone();
    let assignments=state.db.call(move|db|{
        let rows=db.prepare("SELECT track.id,b.members FROM media track JOIN media_sources ms ON ms.media_id=track.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE track.kind='track' AND track.parent_id=?1 AND b.service_id=?2 AND b.service_generation=?3 AND b.checked_at>=?4")?.query_map(params![album,service_id,generation,now()-60],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut exact=BTreeMap::<String,BTreeSet<i64>>::new();
        for (track,members) in rows {for member in serde_json::from_str::<Vec<i64>>(&members).unwrap_or_default(){exact.entry(track.clone()).or_default().insert(member);}}
        Ok(exact.into_iter().filter_map(|(track,members)|(members.len()==1).then(||(track,*members.first().unwrap()))).collect::<Vec<_>>())
    }).await?;
    let refreshed = now();
    let updates = assignments
        .into_iter()
        .filter_map(|(media, manager)| {
            by_id.get(&manager).map(|row| {
                let mut metadata = normalize("track", row);
                metadata["refreshed_at"] = json!(refreshed);
                (media, metadata.to_string())
            })
        })
        .collect::<Vec<_>>();
    let album = album_id.to_owned();
    state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE media SET metadata='{}' WHERE kind='track' AND parent_id=?1",
                [&album],
            )?;
            for (media, metadata) in updates {
                tx.execute(
                    "UPDATE media SET metadata=?1 WHERE id=?2",
                    params![metadata, media],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
}

pub(super) async fn enqueue_managed_refreshes(state: &AppState) -> Result<()> {
    let cutoff = now() - 6 * 3600;
    let freshness = now() - 60;
    let targets=state.db.call(move|db|{
        let sql="WITH raw(media_id,kind,service_id,service_generation,external_id) AS (
          SELECT m.id,'movie',b.service_id,b.service_generation,b.external_id FROM media m JOIN media_sources ms ON ms.media_id=m.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='radarr' AND s.generation=b.service_generation WHERE m.kind='movie' AND b.checked_at>=?1
          UNION ALL SELECT show.id,'show',b.service_id,b.service_generation,b.external_id FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media show ON show.id=season.parent_id JOIN media_sources ms ON ms.media_id=ep.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='sonarr' AND s.generation=b.service_generation WHERE ep.kind='episode' AND b.checked_at>=?1
          UNION ALL SELECT album.id,'album',b.service_id,b.service_generation,b.external_id FROM media track JOIN media album ON album.id=track.parent_id JOIN media_sources ms ON ms.media_id=track.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='lidarr' AND s.generation=b.service_generation WHERE track.kind='track' AND b.checked_at>=?1
        ), targets AS (
          SELECT media_id,kind,min(service_id) service_id,min(service_generation) service_generation,min(external_id) external_id,count(DISTINCT service_id||char(31)||service_generation||char(31)||external_id) variants FROM raw GROUP BY media_id,kind
        ) SELECT t.media_id,t.kind,t.service_id,t.service_generation,t.external_id FROM targets t LEFT JOIN metadata_bindings m ON m.media_id=t.media_id WHERE t.variants=1 AND (m.media_id IS NULL OR m.service_id<>t.service_id OR m.service_generation<>t.service_generation OR m.external_id<>t.external_id OR m.refreshed_at<?2) ORDER BY t.kind,t.media_id";
        Ok(db.prepare(sql)?.query_map(params![freshness,cutoff],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    let queue = thelxinoe_jobs::Queue(state.db.clone());
    for (media, kind, service, generation, external) in targets {
        queue
            .enqueue(
                "metadata.refresh".into(),
                json!({"media_id":media,"kind":kind,"service_id":service,"service_generation":generation,"external_id":external}),
                format!("manager-metadata:{media}:{}", now() / (6 * 3600)),
            )
            .await?;
    }
    Ok(())
}

async fn update_album_parent(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    album_id: &str,
    row: &Value,
    actor: Option<&str>,
) -> anyhow::Result<()> {
    let parent = ancestor(state, album_id, "artist").await?;
    let Some(parent) = parent else { return Ok(()) };
    let artist_external = row["artist"]["foreignArtistId"]
        .as_str()
        .or_else(|| row["foreignArtistId"].as_str());
    let Some(artist_external) = artist_external else {
        return Ok(());
    };
    let artist = exact_entity(connection, "artist", artist_external)
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    update_metadata(
        state,
        service,
        connection,
        &parent,
        "artist",
        artist_external,
        &artist,
        now(),
        actor,
    )
    .await
}

pub(crate) async fn run(state: &AppState, payload: &Value) -> anyhow::Result<()> {
    let media = payload["media_id"]
        .as_str()
        .context("Missing media identity")?;
    let original_kind = payload["kind"].as_str().context("Missing media type")?;
    let actor = payload["actor_id"].as_str();
    let (target, kind) = root_target(state, media, original_kind).await?;
    let manager = manager_kind(&kind).context("Unsupported media type")?;
    let explicit = match (
        payload["service_id"].as_str(),
        payload["external_id"].as_str(),
    ) {
        (Some(service_id), Some(external)) => Some(Binding {
            service_id: service_id.to_owned(),
            external_id: external.to_owned(),
            entity_id: None,
        }),
        _ => None,
    };
    let (binding, inferred) = if let Some(binding) = explicit {
        (binding, false)
    } else if let Some(binding) = stored_binding(state, &target).await? {
        (binding, false)
    } else {
        (
            inferred_binding(state, &target, manager)
                .await?
                .context("No matching Radarr, Sonarr or Lidarr item is available")?,
            true,
        )
    };
    let service = service(state, &binding.service_id)
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    if let Some(expected) = payload["service_generation"].as_str() {
        anyhow::ensure!(
            expected == service.generation,
            "Metadata manager configuration changed; retry the operation"
        );
    }
    anyhow::ensure!(
        service.kind == manager,
        "Metadata manager does not match the media type"
    );
    let connection = Connection::open(state, &service)
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    let external = if inferred && kind == "artist" {
        let album = exact_entity(&connection, "album", &binding.external_id)
            .await
            .map_err(|error| anyhow::anyhow!(error.2))?;
        album["artist"]["foreignArtistId"]
            .as_str()
            .or_else(|| album["foreignArtistId"].as_str())
            .context("Lidarr album has no artist identity")?
            .to_owned()
    } else {
        binding.external_id.clone()
    };
    let mut row = exact_entity(&connection, &kind, &external)
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    if row["id"].as_i64().is_none() && binding.entity_id.is_some() {
        row["id"] = json!(binding.entity_id);
    }
    match kind.as_str() {
        "show" => update_show(state, &service, &connection, &target, &external, row, actor).await?,
        "album" => {
            update_metadata(
                state,
                &service,
                &connection,
                &target,
                "album",
                &external,
                &row,
                now(),
                actor,
            )
            .await?;
            update_album_parent(state, &service, &connection, &target, &row, actor).await?;
            update_album_tracks(state, &service, &connection, &target, &row).await?;
        }
        "artist" => {
            update_metadata(
                state,
                &service,
                &connection,
                &target,
                "artist",
                &external,
                &row,
                now(),
                actor,
            )
            .await?
        }
        "movie" => {
            update_metadata(
                state,
                &service,
                &connection,
                &target,
                "movie",
                &external,
                &row,
                now(),
                actor,
            )
            .await?
        }
        _ => bail!("Unsupported metadata target"),
    }
    state
        .emit(None, "catalog.changed", json!({"id":target}))
        .await?;
    Ok(())
}

async fn manager_episodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    let result=state.db.call(move|db|{
        let show:String=db.query_row("SELECT CASE WHEN m.kind='episode' THEN season.parent_id WHEN m.kind='season' THEN m.parent_id ELSE m.id END FROM media m LEFT JOIN media season ON season.id=m.parent_id WHERE m.id=?1",[&media_id],|r|r.get(0))?;
        let items=db.prepare("SELECT e.manager_episode_id,e.season_number,e.episode_number,e.metadata FROM manager_episodes e JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id AND b.refreshed_at=e.refreshed_at JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1 ORDER BY e.season_number,e.episode_number")?.query_map([&show],|r|Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"season":r.get::<_,i64>(1)?,"episode":r.get::<_,i64>(2)?,"metadata":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mappings=db.prepare("SELECT m.manager_episode_id,m.state FROM manager_episode_mappings m JOIN metadata_bindings b ON b.service_id=m.service_id AND b.service_generation=m.service_generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE m.media_id=?1 AND b.media_id=?2")?.query_map(params![media_id,show],|r|Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"state":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"items":items,"mappings":mappings}))
    }).await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct Mapping {
    episode_ids: Vec<String>,
}

async fn map_episode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(input): Json<Mapping>,
) -> Result<Json<Value>> {
    let principal = security::require(&state, &headers, Capability::ManageLibrary).await?;
    if input.episode_ids.len() > 20
        || input.episode_ids.iter().collect::<BTreeSet<_>>().len() != input.episode_ids.len()
    {
        return Err(ApiError::bad("Choose at most 20 distinct manager episodes"));
    }
    let ids = input
        .episode_ids
        .iter()
        .map(|id| {
            id.parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(|| ApiError::bad("Invalid manager episode"))
        })
        .collect::<Result<Vec<_>>>()?;
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let show:Option<String>=tx.query_row("SELECT show.id FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media show ON show.id=season.parent_id WHERE ep.id=?1 AND ep.kind='episode'",[&media_id],|r|r.get(0)).optional()?;
        let Some(show)=show else{return Ok(false);};
        let binding:Option<(String,String,String,i64)>=tx.query_row("SELECT b.service_id,b.service_generation,b.external_id,b.refreshed_at FROM metadata_bindings b JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1",[&show],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((service,generation,external,refreshed))=binding else{return Ok(false);};
        for id in &ids {let belongs:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND manager_episode_id=?3 AND series_external_id=?4 AND refreshed_at=?5)",params![service,generation,id,external,refreshed],|r|r.get(0))?;if !belongs{return Ok(false);}}
        tx.execute("DELETE FROM manager_episode_mappings WHERE media_id=?1",[&media_id])?;
        for id in &ids{tx.execute("INSERT INTO manager_episode_mappings VALUES (?1,?2,?3,?4,?5)",params![media_id,service,generation,id,if ids.len()==1{"confirmed"}else{"complex"}])?;}
        tx.execute("UPDATE manager_episode_mappings SET state=CASE WHEN media_id IN (SELECT media_id FROM manager_episode_mappings GROUP BY media_id HAVING COUNT(*)>1) OR (service_id,service_generation,manager_episode_id) IN (SELECT service_id,service_generation,manager_episode_id FROM manager_episode_mappings GROUP BY service_id,service_generation,manager_episode_id HAVING COUNT(*)>1) THEN 'complex' ELSE 'confirmed' END WHERE state<>'unresolved'",[])?;
        tx.execute("UPDATE media SET metadata=COALESCE((SELECT e.metadata FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id WHERE m.media_id=media.id AND m.state='confirmed'),'{}') WHERE id=?1",[&media_id])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'episode.map',?2,?3)",params![principal.user.id,media_id,now()])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !result {
        return Err(ApiError::bad(
            "Episode or manager identity does not belong to this show",
        ));
    }
    Ok(Json(json!({"ok":true})))
}

async fn collections(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::Browse).await?;
    let items=state.db.call(|db|Ok(db.prepare("SELECT json_extract(metadata,'$.belongs_to_collection.id'),json_extract(metadata,'$.belongs_to_collection.name'),count(*) FROM media WHERE kind='movie' AND json_extract(metadata,'$.belongs_to_collection.id') IS NOT NULL GROUP BY json_extract(metadata,'$.belongs_to_collection.id')")?.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"count":r.get::<_,i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":items})))
}

#[derive(Deserialize)]
struct ArtGrant {
    grant: String,
}

async fn artwork(
    State(state): State<AppState>,
    Path(media_id): Path<String>,
    Query(query): Query<ArtGrant>,
    request: Request<Body>,
) -> Result<Response> {
    grants::resolve(&state, &query.grant, "artwork", false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if media_id.len() != 36 || !media_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(ApiError::not_found());
    }
    let path = state.config.cache.join("artwork").join(media_id);
    let signature = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::not_found())?;
    let content_type = if signature.starts_with(b"\x89PNG") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let mut response = ServeFile::new(path)
        .try_call(request)
        .await
        .map_err(anyhow::Error::from)?
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "private, max-age=300".parse().unwrap());
    response
        .headers_mut()
        .insert("content-type", content_type.parse().unwrap());
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arr_metadata_is_normalized_for_catalog_consumers() {
        let movie = normalize(
            "movie",
            &json!({
                "title":"Fixture","overview":"Plot","year":1999,"runtime":136,"status":"released","genres":["Action"],
                "releaseDate":"1999-03-31","tmdbId":603,"imdbId":"tt0133093","ratings":{"tmdb":{"value":8.2}},
                "collection":{"tmdbId":2344,"title":"Collection"}
            }),
        );
        assert_eq!(movie["tmdb_id"], 603);
        assert_eq!(movie["vote_average"], 8.2);
        assert_eq!(movie["genres"][0]["name"], "Action");
        assert_eq!(movie["belongs_to_collection"]["id"], 2344);

        let album = normalize(
            "album",
            &json!({
                "title":"Album","foreignAlbumId":"11111111-1111-1111-1111-111111111111","releases":[{"monitored":true,"foreignReleaseId":"22222222-2222-2222-2222-222222222222"}]
            }),
        );
        assert_eq!(
            album["musicbrainz_release_group_id"],
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(
            album["musicbrainz_release_id"],
            "22222222-2222-2222-2222-222222222222"
        );

        let track = normalize(
            "track",
            &json!({"title":"Track","duration":123000,"foreignRecordingId":"33333333-3333-3333-3333-333333333333","foreignTrackId":"44444444-4444-4444-4444-444444444444"}),
        );
        assert_eq!(
            track["musicbrainz_recording_id"],
            "33333333-3333-3333-3333-333333333333"
        );
        assert_eq!(
            track["musicbrainz_release_track_id"],
            "44444444-4444-4444-4444-444444444444"
        );
    }
}
