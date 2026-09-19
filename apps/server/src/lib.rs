mod accounts;
pub mod config;
pub mod error;
mod grants;
mod history;
mod jellyfin;
pub mod library;
mod managers;
pub mod metadata;
mod online;
pub mod playback;
mod playlists;
mod realtime;
pub mod security;
pub mod segments;
pub mod user_media;
pub use jellyfin::discovery::run as run_discovery;
pub use online::downloads::run as run_downloads;
pub use online::run as run_online;
pub async fn run_service_updates(state: AppState) -> anyhow::Result<()> {
    managers::run_updates(state).await
}
pub async fn run_retention(state: AppState) -> anyhow::Result<()> {
    managers::run_retention(state).await
}

use crate::{config::Config, error::Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    middleware,
    routing::{delete, get, post},
};
use serde_json::json;
use std::sync::Arc;
use thelxinoe_auth::SecretStore;
use thelxinoe_database::Database;
use thelxinoe_jobs::Queue;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct AppState {
    pub server_id: Arc<String>,
    pub db: Database,
    pub secrets: SecretStore,
    pub config: Arc<Config>,
    pub events: tokio::sync::broadcast::Sender<()>,
    pub password_slots: Arc<tokio::sync::Semaphore>,
    pub dummy_hash: Arc<String>,
    pub playback: Arc<thelxinoe_playback::Pipelines>,
    pub subtitle_slots: Arc<tokio::sync::Semaphore>,
    pub compatibility_audio: Arc<tokio::sync::Mutex<()>>,
    pub online: Arc<online::Runtime>,
    pub(crate) managers: Arc<managers::Runtime>,
    pub(crate) media_operations: Arc<tokio::sync::RwLock<()>>,
}
impl AppState {
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.state)?;
        std::fs::create_dir_all(&config.cache)?;
        let db = Database::open(config.state.join("thelxinoe.sqlite3"))?;
        db.call(|db| {db.execute("UPDATE media_operations SET state='uncertain',error='Server stopped during execution; reconcile before preparing another operation' WHERE state='executing'",[])?;Ok(())}).await?;
        let server_id=db.call(|db|{
            db.execute("INSERT INTO settings(key,value) VALUES ('server_id',?1) ON CONFLICT(key) DO NOTHING",[thelxinoe_core::id()])?;
            Ok(db.query_row("SELECT value FROM settings WHERE key='server_id'",[],|r|r.get::<_,String>(0))?)
        }).await?;
        if !config.state.join("secrets/master.key").exists()
            && db
                .call(|c| {
                    Ok(c.query_row("SELECT (SELECT COUNT(*) FROM secrets)+(SELECT COUNT(*) FROM manager_services)+(SELECT COUNT(*) FROM support_services)+(SELECT COUNT(*) FROM stack_provisions)+(SELECT COUNT(*) FROM online_accounts WHERE credential IS NOT NULL)", [], |r| r.get::<_, i64>(0))?)
                })
                .await?
                > 0
        {
            anyhow::bail!(
                "The credential master key is missing. Restore the original key before starting the server."
            );
        }
        let secrets = SecretStore::open(&config.state.join("secrets"))?;
        let setup_path = config.state.join("secrets/setup-token");
        if !setup_path.exists() {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(setup_path)?;
            file.write_all(thelxinoe_auth::token().as_bytes())?;
            file.sync_all()?;
        }
        Ok(Self {
            server_id: Arc::new(server_id),
            playback: Arc::new(thelxinoe_playback::Pipelines::open(&config.cache).await?),
            subtitle_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            db,
            secrets,
            config: Arc::new(config),
            events: tokio::sync::broadcast::channel(128).0,
            password_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            compatibility_audio: Arc::new(tokio::sync::Mutex::new(())),
            online: Arc::new(online::Runtime::new()?),
            managers: Arc::new(managers::Runtime::new()?),
            media_operations: Arc::new(tokio::sync::RwLock::new(())),
            dummy_hash: Arc::new(thelxinoe_auth::password_hash(thelxinoe_auth::token()).await?),
        })
    }
    pub async fn emit(
        &self,
        user_id: Option<String>,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        let kind = kind.to_owned();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO events(user_id,kind,payload,created_at) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![user_id, kind, payload.to_string(), thelxinoe_core::now()],
                )?;
                Ok(())
            })
            .await?;
        let _ = self.events.send(());
        Ok(())
    }
}
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(jellyfin::router())
        .merge(online::router())
        .merge(managers::router())
        .merge(segments::router())
        .route("/api/v1/{*path}", axum::routing::any(not_found))
        .route("/api/v1/health", get(health))
        .route(
            "/api/v1/setup",
            get(accounts::setup_status).post(accounts::setup),
        )
        .route("/api/v1/auth/login", post(accounts::login))
        .route("/api/v1/auth/logout", post(accounts::logout))
        .route("/api/v1/auth/me", get(accounts::me))
        .route("/api/v1/auth/event-ticket", post(grants::event_ticket))
        .route(
            "/api/v1/auth/quick-connect/inspect",
            post(jellyfin::quick_connect::inspect),
        )
        .route(
            "/api/v1/auth/quick-connect/approve",
            post(jellyfin::quick_connect::approve),
        )
        .route("/api/v1/auth/sessions", get(accounts::sessions))
        .route("/api/v1/auth/sessions/{id}", delete(accounts::revoke))
        .route(
            "/api/v1/users",
            get(accounts::users).post(accounts::create_user),
        )
        .route("/api/v1/events", get(realtime::events))
        .route("/api/v1/me/home", get(user_media::home))
        .route(
            "/api/v1/me/preferences",
            axum::routing::put(user_media::preferences),
        )
        .route(
            "/api/v1/me/queue/{client}",
            get(user_media::queue_get).put(user_media::queue_put),
        )
        .route("/api/v1/me/history", get(history::mine))
        .route("/api/v1/admin/history", get(history::admin))
        .route("/api/v1/admin/audit", get(history::audit))
        .route(
            "/api/v1/catalog/{id}/state",
            get(user_media::get).put(user_media::set),
        )
        .route(
            "/api/v1/playlists",
            get(playlists::list).post(playlists::create),
        )
        .route(
            "/api/v1/playlists/{id}",
            get(playlists::detail)
                .put(playlists::update)
                .delete(playlists::remove),
        )
        .route(
            "/api/v1/playlists/{id}/favorite",
            axum::routing::put(playlists::favorite),
        )
        .route(
            "/api/v1/catalog/roots",
            get(library::roots).post(library::add_root),
        )
        .route("/api/v1/catalog/roots/{id}/scan", post(library::scan))
        .route("/api/v1/catalog", get(library::browse))
        .route("/api/v1/catalog/collections", get(metadata::collections))
        .route(
            "/api/v1/catalog/{id}/provider-episodes",
            get(metadata::provider_episodes),
        )
        .route(
            "/api/v1/catalog/{id}/episode-mapping",
            axum::routing::put(metadata::map_episode),
        )
        .route("/api/v1/metadata/search", get(metadata::search))
        .route(
            "/api/v1/admin/metadata",
            get(metadata::configuration).put(metadata::configure),
        )
        .route("/api/v1/catalog/{id}/match", post(metadata::match_item))
        .route("/api/v1/catalog/{id}/refresh", post(metadata::refresh))
        .route("/api/v1/catalog/{id}/artwork", get(metadata::artwork))
        .route("/api/v1/catalog/{id}", get(library::detail))
        .route("/api/v1/playback", post(playback::create))
        .route("/api/v1/playback/{id}", delete(playback::cancel))
        .route("/api/v1/playback/{id}/keepalive", post(playback::keepalive))
        .route(
            "/api/v1/playback/preferences",
            get(playback::preferences).put(playback::save_preferences),
        )
        .route("/api/v1/playback/{id}/progress", post(playback::progress))
        .route("/api/v1/playback/{id}/seek", post(playback::seek))
        .route("/api/v1/playback/{id}/stream", get(playback::stream))
        .route(
            "/api/v1/playback/{id}/hls/{revision}/{name}",
            get(playback::hls),
        )
        .route(
            "/api/v1/playback/{id}/subtitles/{track}",
            get(playback::subtitle),
        )
        .route("/api/v1/catalog/{id}/playback", get(playback::media_info))
        .route(
            "/api/v1/catalog/{id}/overrides",
            axum::routing::put(library::overrides),
        )
        .route(
            "/api/v1/admin/jobs",
            get(realtime::jobs).post(realtime::enqueue),
        )
        .route(
            "/api/v1/admin/settings",
            get(accounts::settings).put(accounts::save_settings),
        )
        .route("/api/v1/admin/health", get(admin_health))
        .fallback_service(
            ServeDir::new(&state.config.web)
                .not_found_service(ServeFile::new(state.config.web.join("index.html"))),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}
async fn health() -> Json<serde_json::Value> {
    Json(
        json!({"status":"ok","version":thelxinoe_core::VERSION,"api_version":thelxinoe_core::API_VERSION}),
    )
}
async fn admin_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    security::require(&state, &headers, thelxinoe_core::Capability::ManageServer).await?;
    #[cfg(unix)]
    let controller = reqwest::Client::builder()
        .unix_socket(state.config.controller_socket.clone())
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(anyhow::Error::from)?
        .get("http://localhost/health")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    #[cfg(not(unix))]
    let controller = false;
    let free = fs2::available_space(&state.config.cache).map_err(anyhow::Error::from)?;
    Ok(Json(
        json!({"version":thelxinoe_core::VERSION,"database":"ok","controller":controller,"cache_free_bytes":free}),
    ))
}
pub async fn run_jobs(state: AppState) -> anyhow::Result<()> {
    let queue = Queue(state.db.clone());
    queue.recover().await?;
    loop {
        if let Some(job) = queue.claim().await? {
            match job.kind.as_str() {
                "checkpoint" => queue.checkpoint(&job).await?,
                "stack.install" => {
                    let result = managers::provision(&state, &job).await;
                    queue
                        .finish(&job, result.err().map(|e| e.to_string()))
                        .await?;
                }
                "service.update" => {
                    let result = managers::update_service(&state, &job).await;
                    if !matches!(result, Ok(false)) {
                        queue
                            .finish(&job, result.err().map(|e| e.to_string()))
                            .await?;
                    }
                }
                "manager.request" => {
                    let result = managers::acquire(&state, &job).await;
                    queue
                        .finish(&job, result.err().map(|e| e.to_string()))
                        .await?;
                }
                "online.tools.install" => {
                    let result = online::tools::install(&state, &job).await;
                    queue
                        .finish(&job, result.err().map(|e| e.to_string()))
                        .await?;
                }
                "metadata.match" | "metadata.refresh" => {
                    let result = metadata::run(&state, &job.payload).await;
                    queue
                        .finish(&job, result.err().map(|e| e.to_string()))
                        .await?;
                }
                "library.scan" => {
                    let _lease = state.media_operations.read().await;
                    let roots = thelxinoe_catalog::roots(&state.db).await?;
                    if let Some(root) = roots
                        .into_iter()
                        .find(|r| Some(r.id.as_str()) == job.payload["root_id"].as_str())
                    {
                        state
                            .emit(None, "catalog.scan.started", json!({"root_id":root.id}))
                            .await?;
                        let result = thelxinoe_catalog::scan_with_progress(&state.db, root.clone(), |completed, total| {
                            let state = state.clone();
                            let root_id = root.id.clone();
                            async move { state.emit(None, "catalog.scan.progress", json!({"root_id":root_id,"completed":completed,"total":total})).await.map(|_| ()) }
                        }).await;
                        let error = result.err().map(|e| e.to_string());
                        if let Some(error) = error.clone() {
                            state
                                .db
                                .call(move |db| {
                                    db.execute(
                                        "UPDATE library_roots SET scan_error=?1 WHERE id=?2",
                                        rusqlite::params![error, root.id],
                                    )?;
                                    Ok(())
                                })
                                .await?;
                        }
                        queue.finish(&job, error).await?;
                        state
                            .emit(
                                None,
                                "catalog.changed",
                                json!({"root_id":job.payload["root_id"]}),
                            )
                            .await?;
                    } else {
                        queue
                            .finish(&job, Some("Library no longer exists".into()))
                            .await?;
                    }
                }
                _ => queue.finish(&job, Some("Unknown job type".into())).await?,
            }
            state
                .emit(None, "jobs.changed", json!({"id":job.id}))
                .await?;
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
pub async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error":{"code":"not_found","message":"Unknown API endpoint"}})),
    )
}
