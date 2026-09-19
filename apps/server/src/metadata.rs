use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use anyhow::{Context, bail};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use thelxinoe_core::{Capability, now};
use tokio::sync::Mutex;
use tower_http::services::ServeFile;

static MUSIC_RATE: LazyLock<Arc<Mutex<tokio::time::Instant>>> =
    LazyLock::new(|| Arc::new(Mutex::new(tokio::time::Instant::now())));

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
async fn json_response(response: reqwest::Response) -> anyhow::Result<Value> {
    if !response.status().is_success() {
        bail!(
            "Metadata provider returned HTTP {}",
            response.status().as_u16()
        );
    }
    if response
        .content_length()
        .is_some_and(|n| n > 8 * 1024 * 1024)
    {
        bail!("Metadata response is too large");
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            bail!("Metadata response is too large");
        }
        bytes.extend(chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
async fn tmdb(state: &AppState, path: &str, query: &[(&str, &str)]) -> anyhow::Result<Value> {
    let token = state
        .secrets
        .get(&state.db, "provider.tmdb")
        .await?
        .context("Configure the TMDB API Read Access Token in Settings")?;
    let token = String::from_utf8(token)?;
    json_response(
        client()?
            .get(format!("https://api.themoviedb.org/3/{path}"))
            .bearer_auth(token.trim())
            .query(query)
            .send()
            .await?,
    )
    .await
}
async fn musicbrainz(
    state: &AppState,
    path: &str,
    query: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let contact = state
        .db
        .call(|db| {
            Ok(db
                .query_row(
                    "SELECT value FROM settings WHERE key='musicbrainz_contact'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?
        .context("Configure a MusicBrainz contact email or URL in Settings")?;
    let mut next = MUSIC_RATE.lock().await;
    tokio::time::sleep_until(*next).await;
    *next = tokio::time::Instant::now() + std::time::Duration::from_millis(1100);
    json_response(
        client()?
            .get(format!("https://musicbrainz.org/ws/2/{path}"))
            .header(
                "User-Agent",
                format!("Thelxinoe/{} ({contact})", thelxinoe_core::VERSION),
            )
            .query(&[("fmt", "json")])
            .query(query)
            .send()
            .await?,
    )
    .await
}
pub async fn configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let contact = state
        .db
        .call(|db| {
            Ok(db
                .query_row(
                    "SELECT value FROM settings WHERE key='musicbrainz_contact'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?;
    Ok(Json(
        json!({"tmdb_configured":state.secrets.get(&state.db,"provider.tmdb").await?.is_some(),"musicbrainz_contact":contact}),
    ))
}
#[derive(Deserialize)]
pub struct Configuration {
    tmdb_token: Option<String>,
    musicbrainz_contact: Option<String>,
}
pub async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Configuration>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if let Some(token) = &input.tmdb_token
        && (token.trim().len() < 20 || token.len() > 4096)
    {
        return Err(ApiError::bad("Enter the TMDB API Read Access Token"));
    }
    if let Some(contact) = &input.musicbrainz_contact
        && (contact.is_empty()
            || contact.len() > 200
            || contact.contains(['\r', '\n'])
            || (!contact.contains('@') && !contact.starts_with("https://")))
    {
        return Err(ApiError::bad(
            "Enter a contact email or HTTPS URL for MusicBrainz",
        ));
    }
    if let Some(token) = input.tmdb_token {
        state
            .secrets
            .put(&state.db, "provider.tmdb".into(), token.trim().as_bytes())
            .await?;
    }
    if let Some(contact) = input.musicbrainz_contact {
        state.db.call(move|db|{db.execute("INSERT INTO settings VALUES ('musicbrainz_contact',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[contact])?;Ok(())}).await?;
    }
    state.db.call(move|db|{db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.configure','providers',?2)",params![p.user.id,now()])?;Ok(())}).await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Search {
    q: String,
    kind: String,
}
pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Search>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    if query.q.len() > 200 {
        return Err(ApiError::bad("Search is too long"));
    }
    let result = search_provider(&state, &query.kind, &query.q)
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?;
    Ok(Json(result))
}
async fn search_provider(state: &AppState, kind: &str, query: &str) -> anyhow::Result<Value> {
    match kind {
        "movie" | "show" => {
            let provider_kind = if kind == "movie" { "movie" } else { "tv" };
            let value = tmdb(
                state,
                &format!("search/{provider_kind}"),
                &[("query", query), ("include_adult", "false")],
            )
            .await?;
            Ok(
                json!({"items":value["results"].as_array().into_iter().flatten().map(|v|json!({"provider":"tmdb","id":v["id"].to_string(),"title":v.get("title").or_else(||v.get("name")),"year":v.get("release_date").or_else(||v.get("first_air_date")).and_then(Value::as_str).and_then(|s|s.get(..4)),"overview":v["overview"]})).collect::<Vec<_>>()}),
            )
        }
        "artist" | "album" | "track" => {
            let entity = music_entity(kind)?;
            let value = musicbrainz(state, entity, &[("query", query), ("limit", "15")]).await?;
            let plural = if entity == "release" {
                "releases"
            } else if entity == "artist" {
                "artists"
            } else {
                "recordings"
            };
            Ok(
                json!({"items":value[plural].as_array().into_iter().flatten().map(|v|json!({"provider":"musicbrainz","id":v["id"],"title":v.get("title").or_else(||v.get("name")),"year":v["date"],"score":v["score"]})).collect::<Vec<_>>()}),
            )
        }
        _ => bail!("Match a movie, show, artist, album or track"),
    }
}
fn music_entity(kind: &str) -> anyhow::Result<&'static str> {
    match kind {
        "artist" => Ok("artist"),
        "album" => Ok("release"),
        "track" => Ok("recording"),
        _ => bail!("Not a music entity"),
    }
}
#[derive(Deserialize)]
pub struct Match {
    provider: String,
    external_id: String,
}
pub async fn match_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(input): Json<Match>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    validate_id(&input.provider, &input.external_id).map_err(|e| ApiError::bad(e.to_string()))?;
    let target = media_id.clone();
    let record = state
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
    if (input.provider == "tmdb" && !matches!(record.as_str(), "movie" | "show"))
        || (input.provider == "musicbrainz"
            && !matches!(record.as_str(), "artist" | "album" | "track"))
    {
        return Err(ApiError::bad("Provider does not match this media type"));
    }
    let job=thelxinoe_jobs::Queue(state.db.clone()).enqueue("metadata.match".into(),json!({"media_id":media_id,"kind":record,"provider":input.provider,"external_id":input.external_id,"actor_id":p.user.id}),format!("metadata:{media_id}:{}",thelxinoe_core::id())).await?;
    Ok(Json(json!({"job_id":job})))
}
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let target = media_id.clone();
    let (kind,title,provider,external_id,year)=state.db.call(move|db|Ok(db.query_row("SELECT m.kind,m.title,p.provider,p.external_id,m.year FROM media m LEFT JOIN provider_ids p ON p.media_id=m.id AND p.provider IN ('tmdb','musicbrainz') WHERE m.id=?1 ORDER BY p.provider LIMIT 1",[target],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<i64>>(4)?))).optional()?)).await?.ok_or_else(ApiError::not_found)?;
    if !matches!(
        kind.as_str(),
        "movie" | "show" | "artist" | "album" | "track"
    ) {
        return Err(ApiError::bad(
            "Refresh a movie, show, artist, album or track",
        ));
    }
    let payload = json!({"media_id":media_id,"kind":kind,"title":title,"provider":provider,"external_id":external_id,"year":year,"actor_id":p.user.id});
    let job = thelxinoe_jobs::Queue(state.db.clone())
        .enqueue(
            "metadata.refresh".into(),
            payload,
            format!("refresh:{media_id}:{}", now() / 10),
        )
        .await?;
    Ok(Json(json!({"job_id":job})))
}
fn validate_id(provider: &str, id: &str) -> anyhow::Result<()> {
    match provider {
        "tmdb" => {
            id.parse::<u64>().context("TMDB ID must be a number")?;
        }
        "musicbrainz" => {
            if id.len() != 36 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                bail!("Invalid MusicBrainz identifier");
            }
        }
        _ => bail!("Unknown metadata provider"),
    };
    Ok(())
}
pub async fn run(state: &AppState, payload: &Value) -> anyhow::Result<()> {
    let media_id = payload["media_id"]
        .as_str()
        .context("Missing media identity")?;
    let kind = payload["kind"].as_str().context("Missing media type")?;
    let (provider, external_id) = if let (Some(provider), Some(id)) = (
        payload["provider"].as_str(),
        payload["external_id"].as_str(),
    ) {
        (provider.to_string(), id.to_string())
    } else {
        let title = payload["title"].as_str().context("No title for matching")?;
        let result = search_provider(state, kind, title).await?;
        let candidates = result["items"]
            .as_array()
            .context("Invalid metadata search")?;
        let exact: Vec<_> = candidates
            .iter()
            .filter(|v| {
                v["title"]
                    .as_str()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(title))
                    && payload["year"].as_i64().is_none_or(|year| {
                        v["year"]
                            .as_str()
                            .and_then(|s| s.get(..4))
                            .and_then(|s| s.parse::<i64>().ok())
                            == Some(year)
                    })
            })
            .collect();
        if exact.len() != 1 {
            bail!("Automatic match is uncertain; choose a provider result manually");
        }
        (
            exact[0]["provider"]
                .as_str()
                .context("Missing provider")?
                .into(),
            exact[0]["id"]
                .as_str()
                .context("Missing provider ID")?
                .into(),
        )
    };
    validate_id(&provider, &external_id)?;
    let mut metadata = if provider == "tmdb" {
        tmdb(
            state,
            &format!(
                "{}/{external_id}",
                if kind == "movie" { "movie" } else { "tv" }
            ),
            &[("append_to_response", "external_ids,videos")],
        )
        .await?
    } else {
        musicbrainz(
            state,
            &format!("{}/{external_id}", music_entity(kind)?),
            &[],
        )
        .await?
    };
    if provider == "tmdb" && kind == "show" {
        refresh_tv_orders(state, &external_id, &metadata).await?;
    }
    let mut artwork = None;
    if provider == "tmdb" {
        if let Some(path) = metadata["poster_path"]
            .as_str()
            .filter(|p| p.starts_with('/') && !p.contains("..") && !p.contains('?'))
        {
            artwork = Some(format!("https://image.tmdb.org/t/p/w500{path}"));
        }
    } else if kind == "album" {
        artwork = Some(format!(
            "https://coverartarchive.org/release/{external_id}/front-500"
        ));
    }
    let artwork = if let Some(url) = artwork {
        cache_artwork(state, media_id, &url).await.unwrap_or(false)
    } else {
        false
    };
    metadata["artwork_cached"] = json!(artwork);
    metadata["refreshed_at"] = json!(now());
    let actor = payload["actor_id"].as_str().map(str::to_string);
    let mid = media_id.to_string();
    let provider_clone = provider.clone();
    let eid = external_id.clone();
    let stored = metadata.clone();
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let rematched:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_ids WHERE media_id=?1 AND provider=?2 AND external_id<>?3)",params![mid,provider_clone,eid],|r|r.get(0))?;
        if rematched && provider_clone == "tmdb" {
            tx.execute("DELETE FROM episode_mappings WHERE media_id IN (SELECT ep.id FROM media ep JOIN media season ON ep.parent_id=season.id WHERE season.parent_id=?1) AND provider='tmdb'",[&mid])?;
            tx.execute("UPDATE media SET metadata='{}' WHERE kind='episode' AND parent_id IN (SELECT id FROM media WHERE parent_id=?1)",[&mid])?;
        }
        tx.execute("UPDATE media SET metadata=?1 WHERE id=?2",params![stored.to_string(),mid])?;
        // A rematch replaces this provider's binding without changing the opaque logical ID.
        tx.execute("DELETE FROM provider_ids WHERE media_id=?1 AND provider=?2",params![mid,provider_clone])?;
        tx.execute("INSERT INTO provider_ids(media_id,provider,external_id,mapping_state) VALUES (?1,?2,?3,'exact')",params![mid,provider_clone,eid])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.match',?2,?3)",params![actor,mid,now()])?;tx.commit()?;Ok(())
    }).await?;
    // Provider coordinates are evidence only. Episode mapping remains unresolved until an
    // explicit mapping is confirmed; numeric TMDB/Sonarr equality never implies ownership.
    state
        .emit(None, "catalog.changed", json!({"id":media_id}))
        .await?;
    Ok(())
}

async fn refresh_tv_orders(
    state: &AppState,
    series_id: &str,
    series: &Value,
) -> anyhow::Result<()> {
    let mut episodes = Vec::new();
    for season in series["seasons"].as_array().into_iter().flatten().take(100) {
        let number = season["season_number"]
            .as_u64()
            .context("Invalid provider season")?;
        let data = tmdb(state, &format!("tv/{series_id}/season/{number}"), &[]).await?;
        for episode in data["episodes"].as_array().into_iter().flatten() {
            episodes.push((
                episode["id"]
                    .as_u64()
                    .context("Invalid provider episode")?
                    .to_string(),
                number,
                episode["episode_number"]
                    .as_u64()
                    .context("Invalid provider episode coordinate")?,
                episode.clone(),
            ));
        }
    }
    let series_id = series_id.to_string();
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;for (eid,season,number,metadata) in episodes{tx.execute("INSERT INTO provider_episodes VALUES ('tmdb',?1,?2,?3,?4,?5) ON CONFLICT(provider,episode_id) DO UPDATE SET series_id=excluded.series_id,season_number=excluded.season_number,episode_number=excluded.episode_number,metadata=excluded.metadata",params![series_id,eid,i64::try_from(season)?,i64::try_from(number)?,metadata.to_string()])?;}tx.commit()?;Ok(())}).await
}
pub async fn provider_episodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    let result=state.db.call(move|db|{
        let show:String=db.query_row("SELECT CASE WHEN m.kind='episode' THEN season.parent_id WHEN m.kind='season' THEN m.parent_id ELSE m.id END FROM media m LEFT JOIN media season ON season.id=m.parent_id WHERE m.id=?1",[&media_id],|r|r.get(0))?;
        let items=db.prepare("SELECT e.episode_id,e.season_number,e.episode_number,e.metadata FROM provider_episodes e JOIN provider_ids p ON p.provider=e.provider AND p.external_id=e.series_id WHERE p.media_id=?1 ORDER BY e.season_number,e.episode_number")?.query_map([show],|r|Ok(json!({"id":r.get::<_,String>(0)?,"season":r.get::<_,i64>(1)?,"episode":r.get::<_,i64>(2)?,"metadata":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or_default()})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mappings=db.prepare("SELECT episode_id,state FROM episode_mappings WHERE media_id=?1 AND provider='tmdb'")?.query_map([media_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"items":items,"mappings":mappings}))
    }).await?;
    Ok(Json(result))
}
#[derive(Deserialize)]
pub struct Mapping {
    episode_ids: Vec<String>,
}
pub async fn map_episode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(input): Json<Mapping>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    if input.episode_ids.len() > 20
        || input
            .episode_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.episode_ids.len()
    {
        return Err(ApiError::bad(
            "Choose at most 20 distinct provider episodes",
        ));
    }
    for id in &input.episode_ids {
        validate_id("tmdb", id).map_err(|e| ApiError::bad(e.to_string()))?;
    }
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let show:Option<String>=tx.query_row("SELECT show.id FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media show ON show.id=season.parent_id WHERE ep.id=?1 AND ep.kind='episode'",[&media_id],|r|r.get(0)).optional()?;
        let Some(show)=show else{return Ok(false);};
        for id in &input.episode_ids{let belongs:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_episodes e JOIN provider_ids p ON p.provider=e.provider AND p.external_id=e.series_id WHERE e.provider='tmdb' AND e.episode_id=?1 AND p.media_id=?2)",params![id,show],|r|r.get(0))?;if !belongs{return Ok(false);}}
        tx.execute("DELETE FROM episode_mappings WHERE media_id=?1 AND provider='tmdb'",[&media_id])?;
        for id in &input.episode_ids{tx.execute("INSERT INTO episode_mappings VALUES (?1,'tmdb',?2,?3)",params![media_id,id,if input.episode_ids.len()==1{"confirmed"}else{"complex"}])?;}
        // A provider episode mapped to multiple logical episodes is explicitly complex.
        tx.execute("UPDATE episode_mappings SET state=CASE WHEN media_id IN (SELECT media_id FROM episode_mappings WHERE provider='tmdb' GROUP BY media_id HAVING COUNT(*)>1) OR episode_id IN (SELECT episode_id FROM episode_mappings WHERE provider='tmdb' GROUP BY episode_id HAVING COUNT(*)>1) THEN 'complex' ELSE 'confirmed' END WHERE provider='tmdb' AND state<>'unresolved'",[])?;
        tx.execute("UPDATE media SET metadata=COALESCE((SELECT e.metadata FROM episode_mappings m JOIN provider_episodes e ON e.provider=m.provider AND e.episode_id=m.episode_id WHERE m.media_id=media.id AND m.provider='tmdb' AND m.state='confirmed'),'{}') WHERE kind='episode' AND parent_id IN (SELECT id FROM media WHERE parent_id=?1)",[&show])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'episode.map',?2,?3)",params![p.user.id,media_id,now()])?;tx.commit()?;Ok(true)
    }).await?;
    if !result {
        return Err(ApiError::bad(
            "Episode or provider identity does not belong to this show",
        ));
    }
    Ok(Json(json!({"ok":true})))
}
pub async fn collections(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::Browse).await?;
    let items=state.db.call(|db|Ok(db.prepare("SELECT json_extract(metadata,'$.belongs_to_collection.id'),json_extract(metadata,'$.belongs_to_collection.name'),count(*) FROM media WHERE kind='movie' AND json_extract(metadata,'$.belongs_to_collection.id') IS NOT NULL GROUP BY json_extract(metadata,'$.belongs_to_collection.id')")?.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"count":r.get::<_,i64>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?)).await?;
    Ok(Json(json!({"items":items})))
}
async fn cache_artwork(state: &AppState, media_id: &str, address: &str) -> anyhow::Result<bool> {
    if !media_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        bail!("Invalid media ID");
    }
    let mut url = url::Url::parse(address)?;
    let http = client()?;
    let mut response = None;
    for _ in 0..5 {
        let host = url.host_str().context("Missing artwork host")?;
        if url.scheme() != "https"
            || !matches!(
                host,
                "image.tmdb.org" | "coverartarchive.org" | "archive.org"
            ) && !host.ends_with(".archive.org")
        {
            bail!("Untrusted artwork host");
        }
        let result = http.get(url.clone()).send().await?;
        if result.status().is_redirection() {
            let location = result
                .headers()
                .get(reqwest::header::LOCATION)
                .context("Missing artwork redirect")?
                .to_str()?;
            url = url.join(location)?;
        } else {
            response = Some(result);
            break;
        }
    }
    let mut response = response.context("Too many artwork redirects")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    if !response.status().is_success() {
        bail!("Artwork download failed");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 10 * 1024 * 1024 {
            bail!("Artwork is too large");
        }
        bytes.extend(chunk);
    }
    if !bytes.starts_with(&[0xff, 0xd8, 0xff]) && !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("Unsupported artwork image");
    }
    let dir = state.config.cache.join("artwork");
    tokio::fs::create_dir_all(&dir).await?;
    if fs2::available_space(&dir)? < 512 * 1024 * 1024 {
        bail!("Insufficient artwork cache space");
    }
    let temp = dir.join(format!("{media_id}.tmp"));
    let path = dir.join(media_id);
    tokio::fs::write(&temp, bytes).await?;
    #[cfg(windows)]
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    tokio::fs::rename(temp, path).await?;
    Ok(true)
}
#[derive(Deserialize)]
pub struct ArtGrant {
    grant: String,
}
pub async fn artwork(
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
