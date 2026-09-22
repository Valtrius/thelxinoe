#[path = "../storage/jellyfin/catalog.rs"]
mod storage;

use super::{Query, canonical};
use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::{
    extract::Request,
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::Principal;

pub const PLAYLIST_VIEW: &str = "275504fb-fdb1-49b3-bf8d-730b56915677";
pub fn item_type(kind: &str) -> &'static str {
    match kind {
        "movie" => "Movie",
        "show" => "Series",
        "season" => "Season",
        "episode" => "Episode",
        "artist" => "MusicArtist",
        "album" => "MusicAlbum",
        "track" => "Audio",
        _ => "Folder",
    }
}
fn kind(name: &str) -> &str {
    match name {
        "movie" => "movie",
        "series" => "show",
        "season" => "season",
        "episode" => "episode",
        "musicartist" => "artist",
        "musicalbum" => "album",
        "audio" => "track",
        _ => "unsupported",
    }
}
pub fn ticks(seconds: f64) -> i64 {
    (seconds.max(0.0) * 10_000_000.0).round() as i64
}
pub fn result(items: Vec<Value>, total: usize, start: usize) -> Value {
    json!({"Items":items,"TotalRecordCount":total,"StartIndex":start})
}
pub async fn views(state: &AppState) -> Result<Value> {
    let server = state.server_id.to_string();
    let mut items = storage::views(server, &state.db).await?;
    items.extend(super::online::views(&state.server_id));
    let total = items.len();
    Ok(result(items, total, 0))
}

pub async fn browse(
    state: &AppState,
    p: &Principal,
    query: &Query,
    id: Option<&str>,
) -> Result<Value> {
    if let Some(value) = super::online::browse(state, p, query, id).await? {
        return Ok(value);
    }
    if let Some(id) = id {
        if is_playlist(state, id).await? {
            return playlists(state, p, Some(id), query).await;
        }
    } else if let Some(parent) = query.get("parentid")
        && is_playlist(state, parent).await?
    {
        return playlist_items(state, p, parent, query).await;
    }
    browse_media(state, p, query, id).await
}
async fn browse_media(
    state: &AppState,
    p: &Principal,
    query: &Query,
    id: Option<&str>,
) -> Result<Value> {
    let q = query.clone();
    let uid = p.user.id.clone();
    let server = state.server_id.to_string();
    let id = id.map(canonical);
    let parent = q.get("parentid").map(|s| canonical(s));
    if parent.as_deref() == Some(PLAYLIST_VIEW)
        || q.get("includeitemtypes")
            .is_some_and(|s| s.eq_ignore_ascii_case("Playlist"))
    {
        return playlists(state, p, id.as_deref(), query).await;
    }
    if let Some(id) = id.as_deref() {
        let roots = views(state).await?;
        if let Some(root) = roots["Items"]
            .as_array()
            .and_then(|a| a.iter().find(|i| i["Id"] == id))
        {
            return Ok(root.clone());
        }
    }
    let start = q
        .get("startindex")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1_000_000);
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);
    let single = id.is_some();
    let (mut items, total) = storage::browse_media(
        &state.db,
        storage::MediaQuery {
            q,
            uid,
            server,
            id,
            parent,
            start,
            limit,
        },
    )
    .await?;
    if items
        .iter()
        .any(|item| item["ImageTags"]["Primary"].is_string())
    {
        let grant = crate::grants::issue(state, p, "artwork", 300).await?;
        for item in &mut items {
            if item["ImageTags"]["Primary"].is_string() {
                item["ImageTags"]["Primary"] = json!(grant);
            }
        }
    }
    if single {
        items.into_iter().next().ok_or_else(ApiError::not_found)
    } else {
        Ok(result(items, total, start))
    }
}

pub async fn is_playlist(state: &AppState, id: &str) -> Result<bool> {
    let id = canonical(id);
    Ok(storage::is_playlist(id, &state.db).await?)
}
pub async fn playlist_items(
    state: &AppState,
    p: &Principal,
    id: &str,
    query: &Query,
) -> Result<Value> {
    let id = canonical(id);
    let (revision, tracks) = storage::playlist_items(id, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let start = query
        .get("startindex")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);
    let page = tracks
        .iter()
        .enumerate()
        .skip(start)
        .take(limit)
        .collect::<Vec<_>>();
    let q = Query::from([
        (
            "ids".into(),
            page.iter()
                .map(|(_, id)| id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
        ("limit".into(), "500".into()),
    ]);
    let catalog = browse_media(state, p, &q, None).await?;
    let items = page
        .into_iter()
        .filter_map(|(position, id)| {
            let mut item = catalog["Items"]
                .as_array()?
                .iter()
                .find(|i| i["Id"] == *id)?
                .clone();
            item["PlaylistItemId"] = json!(format!("{revision}:{position}"));
            Some(item)
        })
        .collect();
    Ok(result(items, tracks.len(), start))
}
pub async fn playlists(
    state: &AppState,
    p: &Principal,
    id: Option<&str>,
    q: &Query,
) -> Result<Value> {
    super::online::ensure_lists(state, p).await?;
    let user = p.user.id.clone();
    let id = id.map(canonical);
    let single = id.is_some();
    let server = state.server_id.to_string();
    let favorite = q
        .get("isfavorite")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || q.get("filters")
            .is_some_and(|v| v.split(',').any(|f| f.eq_ignore_ascii_case("IsFavorite")));
    let start = if single {
        0
    } else {
        q.get("startindex")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let limit = if single {
        1
    } else {
        q.get("limit")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(100)
            .min(500)
    };
    let search = q.get("searchterm").cloned();
    let media = q.get("mediatypes").cloned();
    let order = if q
        .get("sortby")
        .is_some_and(|v| v.to_ascii_lowercase().contains("date"))
    {
        "updated_at"
    } else {
        "name COLLATE NOCASE"
    };
    let direction = if q
        .get("sortorder")
        .is_some_and(|v| v.eq_ignore_ascii_case("Descending"))
    {
        "DESC"
    } else {
        "ASC"
    };
    let (items, total) = storage::playlists(
        &state.db,
        storage::PlaylistQuery {
            user,
            id,
            server,
            favorite,
            start,
            limit,
            search,
            media,
            order,
            direction,
        },
    )
    .await?;
    if single {
        items.into_iter().next().ok_or_else(ApiError::not_found)
    } else {
        Ok(result(items, total, start as usize))
    }
}

pub async fn image(state: &AppState, id: &str, kind: &str, request: Request) -> Result<Response> {
    if kind != "Primary" {
        return Err(ApiError::not_found());
    }
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| ApiError::not_found())?
        .to_string();
    let path = state.config.cache.join("artwork").join(id);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::not_found())?;
    let content_type = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let mut response = tower_http::services::ServeFile::new(path)
        .try_call(request)
        .await
        .map_err(anyhow::Error::from)?
        .into_response();
    response
        .headers_mut()
        .insert("content-type", content_type.parse().unwrap());
    Ok(response)
}

pub async fn display_preferences(
    state: &AppState,
    p: &Principal,
    id: &str,
    q: &Query,
    body: Option<Value>,
) -> Result<Value> {
    let uid = p.user.id.clone();
    let id = id.to_owned();
    let client = q.get("client").cloned().unwrap_or_default();
    if id.len() > 128 || client.len() > 128 {
        return Err(ApiError::bad("Invalid display preference identifier"));
    }
    storage::display_preferences(uid, id, client, &state.db, body)
        .await?
        .ok_or_else(|| ApiError::conflict("Too many display preferences"))
}
