#[path = "../storage/online.rs"]
mod storage;

pub(crate) mod availability;
mod browse;
pub(crate) mod downloads;
mod egress;
mod extract;
pub(crate) mod feed;
pub(crate) mod kick;
pub(crate) mod live;
pub(crate) mod network;
pub(crate) mod oauth;
mod presentation;
mod process;
mod quota;
pub(crate) mod relay;
mod streamlink_worker;
pub(crate) mod streams;
mod sync;
use crate::tools;
mod thumbnails;
mod twitch;
pub(crate) mod watchlists;
mod youtube;
mod youtube_worker;
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
/// YouTube's video codecs, which the administrator can exclude for players that cannot
/// decode them.
pub(super) const VIDEO_CODECS: [&str; 3] = ["av1", "vp9", "h264"];
async fn excluded_codecs(state: &AppState) -> anyhow::Result<Vec<&'static str>> {
    let allowed = storage::configuration(&state.db).await?.4;
    Ok(VIDEO_CODECS
        .into_iter()
        .filter(|codec| !allowed.iter().any(|allowed| allowed == codec))
        .collect())
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    tokio::try_join!(
        sync::run(state.clone()),
        twitch::run(state.clone()),
        kick::run(state.clone()),
        run_extraction(state.clone())
    )?;
    Ok(())
}
async fn run_extraction(state: AppState) -> anyhow::Result<()> {
    state.tools.wait_ready().await;
    state.online.streamlink.warm(&state).await;
    tokio::try_join!(
        sync::run_classifications(state.clone()),
        downloads::run_watchlists(state.clone()),
        state.online.youtube_worker.run(&state)
    )?;
    Ok(())
}
pub(crate) async fn refresh_tools(state: &AppState) {
    state.online.streamlink.warm(state).await;
}
use thelxinoe_core::{Capability, now};

// Provider images are loaded by the client, with no credentials or server fetch.
pub(super) fn public_image(value: Option<&str>) -> Option<String> {
    let value = value.filter(|s| s.len() <= 2048)?;
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host == "static-cdn.jtvnw.net"
                || host == "yt3.ggpht.com"
                || host == "yt3.googleusercontent.com"
                || host == "kick.com"
                || host.ends_with(".kick.com")
                || host.ends_with(".kickcdn.com")
        }))
    .then(|| value.to_owned())
}
pub(crate) fn artwork_url(value: &str) -> Option<String> {
    if let Some(value) = public_image(Some(value)) {
        return Some(value);
    }
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("i.ytimg.com")
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| value.to_owned())
}
pub(crate) async fn artwork_bytes(
    state: &AppState,
    address: &str,
) -> Result<(&'static str, Vec<u8>)> {
    let address = artwork_url(address).ok_or_else(ApiError::not_found)?;
    let _slot = state
        .online
        .artwork
        .acquire()
        .await
        .map_err(|_| ApiError::not_found())?;
    let response = state
        .online
        .http
        .get(address)
        .send()
        .await
        .map_err(|_| ApiError::not_found())?;
    let (status, _, bytes) = crate::bounded::response(response)
        .await
        .map_err(|_| ApiError::not_found())?;
    if !status.is_success() {
        return Err(ApiError::not_found());
    }
    let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(ApiError::not_found());
    };
    Ok((mime, bytes.to_vec()))
}
pub(crate) async fn youtube_artwork_bytes(
    state: &AppState,
    video: &str,
) -> Result<(&'static str, Vec<u8>)> {
    thumbnails::load(state, video, 1280).await
}

pub(super) fn channel_avatar(channel: &Value) -> Option<String> {
    ["medium", "high", "default"]
        .into_iter()
        .find_map(|size| public_image(channel["snippet"]["thumbnails"][size]["url"].as_str()))
}

pub struct Runtime {
    http: reqwest::Client,
    authorize: String,
    token: String,
    api: String,
    slots: tokio::sync::Semaphore,
    // Images have their own slots: a page of thumbnails neither waits behind
    // API calls nor holds them back.
    artwork: tokio::sync::Semaphore,
    thumbnails: thumbnails::Cache,
    refresh: tokio::sync::Mutex<()>,
    pub(crate) extraction: tokio::sync::Semaphore,
    streamlink: streamlink_worker::Pool,
    youtube_worker: youtube_worker::Pool,
    pub(crate) network: network::Runtime,
    pub(crate) streams: streams::Runtime,
    twitch: twitch::Runtime,
    kick: kick::Runtime,
}
impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            api: "https://www.googleapis.com/youtube/v3".into(),
            slots: tokio::sync::Semaphore::new(4),
            artwork: tokio::sync::Semaphore::new(16),
            thumbnails: thumbnails::Cache::default(),
            refresh: tokio::sync::Mutex::new(()),
            extraction: tokio::sync::Semaphore::new(2),
            streamlink: streamlink_worker::Pool::default(),
            youtube_worker: youtube_worker::Pool::default(),
            network: network::Runtime::default(),
            streams: streams::Runtime::default(),
            twitch: twitch::Runtime::default(),
            kick: kick::Runtime::default(),
        })
    }
}
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(twitch::router())
        .merge(kick::router())
        .route("/api/v1/admin/online", get(configuration).put(configure))
        .route("/api/v1/online/youtube", get(account).delete(disconnect))
        .route("/api/v1/online/youtube/connect", post(oauth::start))
        .route("/api/v1/online/youtube/callback", get(oauth::callback))
        .route("/api/v1/online/youtube/sync", post(sync::request))
        .route("/api/v1/online/youtube/feed", get(feed::list))
        .route("/api/v1/online/youtube/browse", post(browse::list))
        .route("/api/v1/online/youtube/home", get(browse::home))
        .route(
            "/api/v1/online/{provider}/authorization",
            axum::routing::delete(presentation::cancel),
        )
        .route(
            "/api/v1/online/{provider}/sync",
            post(presentation::refresh),
        )
        .route(
            "/api/v1/online/youtube/resolve",
            post(presentation::resolve),
        )
        .route(
            "/api/v1/online/youtube/watchlists",
            get(watchlists::list).post(watchlists::create),
        )
        .route(
            "/api/v1/online/youtube/watchlists/{id}",
            axum::routing::put(watchlists::update).delete(watchlists::delete),
        )
        .route(
            "/api/v1/online/youtube/watchlists/{id}/items",
            post(watchlists::add),
        )
        .route(
            "/api/v1/online/youtube/watchlists/{id}/items/{video}",
            axum::routing::delete(watchlists::remove),
        )
        .route(
            "/api/v1/online/youtube/watchlists/{id}/order",
            axum::routing::put(watchlists::reorder),
        )
        .route("/api/v1/online/youtube/watchlist", post(feed::add))
        .route(
            "/api/v1/admin/online/downloads",
            get(downloads::settings).put(downloads::configure),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}/download",
            get(downloads::status)
                .post(downloads::request)
                .delete(downloads::remove),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}",
            axum::routing::put(feed::edit),
        )
        .route(
            "/api/v1/online/youtube/videos/{id}/artwork",
            get(feed::artwork),
        )
        .route(
            "/api/v1/online/youtube/data",
            axum::routing::delete(feed::delete_data),
        )
        .route("/api/v1/online/providers", get(availability::list))
        .route(
            "/api/v1/admin/online/providers/{provider}",
            axum::routing::put(availability::save),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            availability::gate,
        ))
}
#[derive(Deserialize, Serialize)]
pub(crate) struct Google {
    pub client_id: String,
    pub client_secret: String,
}
impl Google {
    fn validate(&self) -> Result<()> {
        if !self.client_id.ends_with(".apps.googleusercontent.com")
            || self.client_id.len() > 256
            || self.client_secret.is_empty()
            || self.client_secret.len() > 512
            || self
                .client_id
                .chars()
                .chain(self.client_secret.chars())
                .any(char::is_control)
        {
            return Err(ApiError::bad(
                "Enter a Google Web application client ID and client secret",
            ));
        }
        Ok(())
    }
    fn hash(&self) -> String {
        thelxinoe_auth::digest(&format!("{}\0{}", self.client_id, self.client_secret))
    }
}
async fn google(state: &AppState) -> Result<Google> {
    let bytes = state
        .secrets
        .get(&state.db, "provider.google")
        .await?
        .ok_or_else(|| {
            ApiError::bad("An administrator must configure Google application credentials")
        })?;
    let client: Google = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad("Google application credentials are invalid"))?;
    client.validate()?;
    Ok(client)
}
fn redirect_uri(state: &AppState) -> Result<String> {
    let base =
        state.config.public_url.as_ref().ok_or_else(|| {
            ApiError::bad("Set THELXINOE_PUBLIC_URL before linking online accounts")
        })?;
    let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if base.scheme() != "https" && !loopback {
        return Err(ApiError::bad(
            "Online account linking requires an HTTPS public URL (HTTP localhost is allowed for development)",
        ));
    }
    Ok(format!(
        "{}/api/v1/online/youtube/callback",
        base.origin().ascii_serialization()
    ))
}
async fn configuration(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let settings = storage::configuration(&state.db).await?;
    Ok(Json(
        json!({"google_configured":google(&state).await.is_ok(),"redirect_uri":redirect_uri(&state).ok(),"youtube_downloads":settings.0,"youtube_daily_quota":settings.1,"youtube_network":settings.2,"youtube_network_status":network::status(&state,settings.2),"youtube_download_quality":settings.3,"youtube_video_codecs":settings.4,"quota":quota::status(&state).await?}),
    ))
}
#[derive(Deserialize)]
struct Settings {
    google: Option<Google>,
    youtube_downloads: bool,
    youtube_daily_quota: u32,
    youtube_network: network::Mode,
    /// Absent keeps the current quality, for clients that don't offer it.
    #[serde(default)]
    youtube_download_quality: Option<String>,
    /// Absent keeps the current codecs.
    #[serde(default)]
    youtube_video_codecs: Option<Vec<String>>,
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Settings>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !(1..=10_000_000).contains(&input.youtube_daily_quota) {
        return Err(ApiError::bad(
            "Daily quota must be between 1 and 10,000,000 units",
        ));
    }
    if input
        .youtube_download_quality
        .as_deref()
        .is_some_and(|quality| !downloads::QUALITIES.contains(&quality))
    {
        return Err(ApiError::bad("Unsupported download quality"));
    }
    if input.youtube_video_codecs.as_ref().is_some_and(|codecs| {
        codecs.is_empty()
            || codecs
                .iter()
                .any(|codec| !VIDEO_CODECS.contains(&codec.as_str()))
    }) {
        return Err(ApiError::bad("Choose at least one supported video codec"));
    }
    let changed = if let Some(client) = &input.google {
        client.validate()?;
        google(&state)
            .await
            .ok()
            .is_none_or(|old| old.hash() != client.hash())
    } else {
        false
    };
    let encrypted = input
        .google
        .as_ref()
        .map(|g| {
            state.secrets.encrypt(
                "provider.google",
                &serde_json::to_vec(g).expect("credential serialization"),
            )
        })
        .transpose()?;
    storage::configure(&state.db, input, p, changed, encrypted).await?;
    streams::forget(&state).await;
    state
        .emit(None, "online.configuration.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
async fn account(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    let pending = oauth::pending(&state, user.clone()).await?;
    let sync = storage::account_read_youtube_sync(&state.db, user).await?;
    let account = storage::account_read_online_accounts(&state.db, p)
        .await?
        .unwrap_or(
            json!({"status":"disconnected","display_name":"","external_id":"","avatar_url":null}),
        );
    Ok(Json(
        json!({"account":account,"pending":pending,"sync":sync,"downloads_enabled":downloads::enabled(&state).await?,"configured":google(&state).await.is_ok(),"linking_available":redirect_uri(&state).is_ok(),"linking_url":state.config.public_url.as_ref().map(|u|format!("{}/?section=YouTube",u.origin().ascii_serialization())),"quota":quota::status(&state).await?}),
    ))
}
async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    storage::disconnect(&state.db, user).await?;
    state
        .emit(
            Some(p.user.id),
            "online.account.changed",
            json!({"provider":"youtube"}),
        )
        .await?;
    Ok(Json(json!({"disconnected":true})))
}

#[cfg(test)]
mod home_tests;
#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
pub(crate) use twitch::tests::startup_fixture as twitch_startup_fixture;
