mod auth;
mod catalog;
mod playback;
pub(crate) mod quick_connect;
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::SocketAddr};

pub type Query = BTreeMap<String, String>;
pub fn canonical(value: &str) -> String {
    uuid::Uuid::parse_str(value)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| value.to_string())
}
pub fn router() -> Router<AppState> {
    let mut router = Router::new();
    for path in [
        "/System/{*path}",
        "/Users",
        "/Users/{*path}",
        "/UserViews",
        "/UserItems/{*path}",
        "/Items",
        "/Items/{*path}",
        "/Shows/{*path}",
        "/Artists",
        "/Artists/{*path}",
        "/Genres",
        "/MusicGenres",
        "/Sessions",
        "/Sessions/{*path}",
        "/Audio/{*path}",
        "/Videos/{*path}",
        "/DisplayPreferences/{*path}",
        "/QuickConnect/{*path}",
        "/Branding/{*path}",
        "/MediaSegments/{*path}",
        "/Playlists/{*path}",
    ] {
        router = router.route(path, any(dispatch));
    }
    router
}
async fn dispatch(State(state): State<AppState>, request: Request) -> Response {
    let route = safe_route(request.uri().path());
    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        _ => "OTHER",
    };
    let mut response = handle(state, request)
        .await
        .unwrap_or_else(IntoResponse::into_response);
    tracing::debug!(
        route,
        method,
        status = response.status().as_u16(),
        "Compatibility request"
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}
// Only fixed protocol names enter logs. IDs, filenames, query values, headers,
// bodies, and unrecognized path components never enter this representation.
fn safe_route(path: &str) -> String {
    const NAMES: &[&str] = &[
        "System",
        "Info",
        "Public",
        "Users",
        "AuthenticateByName",
        "AuthenticateWithQuickConnect",
        "Me",
        "UserViews",
        "UserItems",
        "Items",
        "Latest",
        "Resume",
        "Images",
        "Primary",
        "Backdrop",
        "Shows",
        "NextUp",
        "Episodes",
        "Seasons",
        "Artists",
        "AlbumArtists",
        "Genres",
        "MusicGenres",
        "Sessions",
        "Playing",
        "Progress",
        "Stopped",
        "Logout",
        "Capabilities",
        "Full",
        "Audio",
        "Videos",
        "stream",
        "stream.mp4",
        "stream.mkv",
        "PlaybackInfo",
        "Intros",
        "DisplayPreferences",
        "QuickConnect",
        "Enabled",
        "Initiate",
        "Connect",
        "Branding",
        "Configuration",
        "MediaSegments",
        "Playlists",
    ];
    path.split('/')
        .filter(|s| !s.is_empty())
        .take(8)
        .map(|s| if NAMES.contains(&s) { s } else { ":value" })
        .collect::<Vec<_>>()
        .join("/")
}
async fn handle(state: AppState, request: Request) -> Result<Response> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0)
        .unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
    let context = security::request_context(&state.config, request.headers(), peer)?;
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && origin != context.origin
    {
        return Err(ApiError::forbidden());
    }
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let lower = path.to_ascii_lowercase();
    let mut query = Query::new();
    if request.uri().query().is_some_and(|q| q.len() > 32 * 1024) {
        return Err(ApiError::bad("Query exceeds size limit"));
    }
    for (key, value) in url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
    {
        let key = key.to_ascii_lowercase();
        let value = value.into_owned();
        if let Some(old) = query.get_mut(&key) {
            if [
                "fields",
                "includeitemtypes",
                "excludeitemtypes",
                "enableimagetypes",
                "imagetypes",
                "ids",
                "filters",
                "sortby",
                "genreids",
                "genres",
                "artistids",
                "studioids",
                "persontypes",
                "excludelocationtypes",
            ]
            .contains(&key.as_str())
            {
                old.push(',');
                old.push_str(&value);
            } else if old != &value {
                return Err(ApiError::bad("Conflicting query parameters"));
            }
        } else {
            query.insert(key, value);
        }
    }
    // The SDK's discovery checks ProductName literally. Version names the emulated
    // API contract; the actual product and release remain visible in ServerName.
    let public_info = || json!({"Id":state.server_id.as_str(),"ServerName":format!("Thelxinoe {}",thelxinoe_core::VERSION),"Version":"10.10.7","ProductName":"Jellyfin Server","ThelxinoeVersion":thelxinoe_core::VERSION,"OperatingSystem":std::env::consts::OS,"LocalAddress":context.origin,"StartupWizardCompleted":true});
    if method == "GET" && lower == "/system/info/public" {
        return Ok(axum::Json(public_info()).into_response());
    }
    if method == "GET" && lower == "/system/ping" {
        return Ok("Jellyfin Server".into_response());
    }
    if method == "GET" && lower == "/users/public" {
        return Ok(axum::Json(json!([])).into_response());
    }
    if method == "GET" && lower == "/quickconnect/enabled" {
        return Ok(axum::Json(true).into_response());
    }
    if method == "POST" && lower == "/quickconnect/initiate" {
        return Ok(axum::Json(
            quick_connect::initiate(&state, auth::device(request.headers())?, context.address)
                .await?,
        )
        .into_response());
    }
    if method == "GET" && lower == "/quickconnect/connect" {
        return Ok(axum::Json(
            quick_connect::status(
                &state,
                query.get("secret").map(String::as_str).unwrap_or_default(),
            )
            .await?,
        )
        .into_response());
    }
    if method == "POST" && lower == "/users/authenticatewithquickconnect" {
        let input = body(request).await?;
        return Ok(axum::Json(
            quick_connect::exchange(&state, input["Secret"].as_str().unwrap_or_default()).await?,
        )
        .into_response());
    }
    if method == "GET" && lower.starts_with("/branding/") {
        return Ok(axum::Json(json!({})).into_response());
    }
    if method == "POST" && lower == "/users/authenticatebyname" {
        let headers = request.headers().clone();
        let body = body(request).await?;
        return Ok(
            axum::Json(auth::login(&state, &headers, context.address, body).await?).into_response(),
        );
    }
    let image_request = parts.len() == 4
        && parts[0] == "Items"
        && parts[2] == "Images"
        && (method == "GET" || method == "HEAD");
    let stream_request = parts.len() == 3
        && ["Videos", "Audio"].contains(&parts[0])
        && parts[2].starts_with("stream")
        && (method == "GET" || method == "HEAD");
    let grant_principal = if let Some(tag) = query.get("tag").filter(|s| s.len() == 64) {
        if image_request {
            crate::grants::resolve(&state, tag, "artwork", false).await?
        } else if stream_request {
            if let Some(id) = query.get("playsessionid") {
                crate::grants::resolve(&state, tag, &format!("playback:{}", canonical(id)), false)
                    .await?
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let p = if let Some(p) = grant_principal {
        p
    } else {
        auth::principal(&state, request.headers(), &query).await?
    };
    if parts.len() > 1
        && parts[0].eq_ignore_ascii_case("Users")
        && !["me", "public"].contains(&parts[1].to_ascii_lowercase().as_str())
        && canonical(parts[1]) != p.user.id
    {
        return Err(ApiError::forbidden());
    }
    if method == "GET" && (lower == "/users/me" || (parts.len() == 2 && parts[0] == "Users")) {
        return Ok(axum::Json(auth::user_dto(&state, &p).await?).into_response());
    }
    if method == "GET" && lower == "/system/info" {
        return Ok(axum::Json(public_info()).into_response());
    }
    if method == "POST" && lower == "/sessions/logout" {
        state
            .db
            .call(move |db| {
                db.execute("DELETE FROM sessions WHERE id=?1", [p.session_id])?;
                Ok(())
            })
            .await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if method == "POST" && lower.starts_with("/sessions/capabilities") {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if parts.len() == 3 && parts[0] == "Items" && parts[2] == "PlaybackInfo" {
        let media = canonical(parts[1]);
        if method == "POST" {
            let input = body(request).await?;
            return Ok(
                axum::Json(playback::info(&state, &p, &media, &query, input).await?)
                    .into_response(),
            );
        }
        if method == "GET" {
            return Ok(
                axum::Json(json!({"MediaSources":playback::sources(&state,&media).await?}))
                    .into_response(),
            );
        }
    }
    if method == "POST"
        && [
            "/sessions/playing",
            "/sessions/playing/progress",
            "/sessions/playing/stopped",
        ]
        .contains(&lower.as_str())
    {
        playback::report(
            &state,
            &p,
            body(request).await?,
            lower.ends_with("/stopped"),
        )
        .await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if parts.len() == 3
        && ["Videos", "Audio"].contains(&parts[0])
        && parts[2].starts_with("stream")
        && (method == "GET" || method == "HEAD")
    {
        let media = canonical(parts[1]);
        return playback::stream(state, &p, &media, &query, request).await;
    }
    if method == "GET" && (lower == "/userviews" || lower.ends_with("/views")) {
        return Ok(axum::Json(catalog::views(&state).await?).into_response());
    }
    if parts.len() == 4
        && parts[0] == "Items"
        && parts[2] == "Images"
        && (method == "GET" || method == "HEAD")
    {
        let id = parts[1].to_owned();
        let kind = parts[3].to_owned();
        return catalog::image(&state, &id, &kind, request).await;
    }
    if parts.len() == 2 && parts[0] == "DisplayPreferences" {
        let id = parts[1].to_owned();
        let value = if method == "POST" {
            Some(body(request).await?)
        } else if method == "GET" {
            None
        } else {
            return Err(ApiError::not_found());
        };
        return Ok(
            axum::Json(catalog::display_preferences(&state, &p, &id, &query, value).await?)
                .into_response(),
        );
    }
    if method == "GET" {
        if parts.len() == 3 && parts[0] == "Items" && parts[2] == "Intros" {
            catalog::browse(&state, &p, &Query::new(), Some(parts[1])).await?;
            return Ok(axum::Json(catalog::result(Vec::new(), 0, 0)).into_response());
        }
        if lower == "/shows/nextup" {
            let home = crate::user_media::home_for(&state, &p).await?;
            let ids = home["next_up"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["id"].as_str())
                .collect::<Vec<_>>()
                .join(",");
            query.insert("ids".into(), ids);
            return Ok(axum::Json(catalog::browse(&state, &p, &query, None).await?).into_response());
        }
        let item_at = if parts.first() == Some(&"Items") {
            Some(0)
        } else if parts.len() >= 3 && parts[0] == "Users" && parts[2] == "Items" {
            Some(2)
        } else {
            None
        };
        if let Some(at) = item_at {
            let tail = &parts[at + 1..];
            if tail.is_empty() {
                return Ok(
                    axum::Json(catalog::browse(&state, &p, &query, None).await?).into_response()
                );
            }
            if tail.len() == 1 {
                if tail[0].eq_ignore_ascii_case("Latest") {
                    query.insert("sortby".into(), "DateCreated".into());
                    query.insert("sortorder".into(), "Descending".into());
                    query.insert("recursive".into(), "true".into());
                    let value = catalog::browse(&state, &p, &query, None).await?;
                    return Ok(axum::Json(value["Items"].clone()).into_response());
                }
                if tail[0].eq_ignore_ascii_case("Resume") {
                    query.insert("filters".into(), "IsResumable".into());
                    query.insert("sortby".into(), "DatePlayed".into());
                    query.insert("sortorder".into(), "Descending".into());
                    return Ok(axum::Json(catalog::browse(&state, &p, &query, None).await?)
                        .into_response());
                }
                let mut item = catalog::browse(&state, &p, &query, Some(tail[0])).await?;
                if item["IsFolder"] == false {
                    item["MediaSources"] =
                        json!(playback::sources(&state, &canonical(tail[0])).await?);
                }
                return Ok(axum::Json(item).into_response());
            }
        }
        if lower == "/useritems/resume" {
            query.insert("filters".into(), "IsResumable".into());
            return Ok(axum::Json(catalog::browse(&state, &p, &query, None).await?).into_response());
        }
        if parts.len() == 3 && parts[0] == "Shows" {
            query.insert("parentid".into(), canonical(parts[1]));
            query.insert(
                "includeitemtypes".into(),
                if parts[2] == "Seasons" {
                    "Season"
                } else if parts[2] == "Episodes" {
                    "Episode"
                } else {
                    return Err(ApiError::not_found());
                }
                .into(),
            );
            query.insert("sortby".into(), "IndexNumber".into());
            if parts[2] == "Episodes" {
                query.insert("recursive".into(), "true".into());
            }
            if let Some(season) = query.get("seasonid").cloned() {
                query.insert("parentid".into(), season);
            }
            return Ok(axum::Json(catalog::browse(&state, &p, &query, None).await?).into_response());
        }
        if lower == "/artists" || lower == "/artists/albumartists" {
            query.insert("includeitemtypes".into(), "MusicArtist".into());
            return Ok(axum::Json(catalog::browse(&state, &p, &query, None).await?).into_response());
        }
    }
    Err(ApiError::not_found())
}
async fn body(request: Request) -> Result<Value> {
    let bytes = axum::body::to_bytes(request.into_body(), 512 * 1024)
        .await
        .map_err(|_| ApiError::bad("Request body exceeds size limit"))?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Invalid JSON request body"))
}
