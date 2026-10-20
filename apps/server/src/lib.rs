#[path = "storage/server.rs"]
mod storage;
#[cfg(test)]
mod test_support;

mod accounts;
#[cfg(test)]
mod accounts_tests;
mod appearance;
pub mod authentication;
mod auto_delete;
mod avatars;
pub mod backups;
pub mod config;
pub mod error;
mod grants;
mod history;
mod jellyfin;
pub mod library;
pub mod maintenance;
mod managers;
#[path = "storage/media_resources.rs"]
mod media_resources;
mod online;
pub mod operations;
#[cfg(test)]
mod operations_tests;
pub mod playback;
mod playlists;
pub mod product;
mod realtime;
pub mod security;
pub mod segments;
mod statistics;
mod timezones;
pub mod tools;
mod transcoding;
pub mod user_media;
pub mod validation;
pub use jellyfin::discovery::run as run_discovery;
pub use online::downloads::run as run_downloads;
pub use online::run as run_online;
pub async fn run_service_updates(state: AppState) -> anyhow::Result<()> {
    managers::run_updates(state).await
}
pub async fn run_service_connections(state: AppState) -> anyhow::Result<()> {
    managers::run_connections(state).await
}
pub async fn run_retention(state: AppState) -> anyhow::Result<()> {
    managers::run_retention(state).await
}
pub async fn run_title_status(state: AppState) -> anyhow::Result<()> {
    managers::status::run(state).await
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

#[derive(Clone)]
pub struct AppState {
    pub server_id: Arc<String>,
    pub event_epoch: Arc<String>,
    pub db: Database,
    pub secrets: SecretStore,
    pub config: Arc<Config>,
    pub events: tokio::sync::broadcast::Sender<()>,
    pub(crate) attention: Arc<tokio::sync::Notify>,
    /// What attention observation last saw, so only changes are pushed.
    pub(crate) attention_digests: Arc<tokio::sync::Mutex<operations::AttentionDigests>>,
    pub password_slots: Arc<tokio::sync::Semaphore>,
    pub dummy_hash: Arc<String>,
    pub playback: Arc<thelxinoe_playback::Pipelines>,
    pub(crate) transcoding: Arc<tokio::sync::RwLock<transcoding::Policy>>,
    pub tools: Arc<tools::Manager>,
    pub subtitle_slots: Arc<tokio::sync::Semaphore>,
    pub compatibility_audio: Arc<tokio::sync::Mutex<()>>,
    pub online: Arc<online::Runtime>,
    pub(crate) managers: Arc<managers::Runtime>,
    pub(crate) media_operations: Arc<tokio::sync::RwLock<()>>,
    pub(crate) playback_lifecycle: Arc<thelxinoe_core::resource_locks::ResourceLocks>,
    pub(crate) media_resources: Arc<thelxinoe_core::resource_locks::ResourceLocks>,
    pub(crate) release_gate: Arc<tokio::sync::RwLock<()>>,
    pub(crate) release_quiescing: Arc<std::sync::atomic::AtomicBool>,
    /// When the controller last reported it was validating a release under its lock.
    pub(crate) release_validating: Arc<std::sync::atomic::AtomicI64>,
    /// Held while a release command waits for the controller's answer.
    pub(crate) release_command: Arc<tokio::sync::Mutex<()>>,
    pub(crate) release_check: Arc<tokio::sync::Mutex<()>>,
    pub(crate) release_install: Arc<tokio::sync::Mutex<()>>,
    /// Library roots by ID, and whether filesystem changes reach them live.
    pub(crate) library_watch: Arc<std::sync::Mutex<std::collections::HashMap<String, bool>>>,
    /// The last measured size of the state and cache folders, and when.
    pub(crate) folder_sizes: Arc<tokio::sync::Mutex<operations::FolderSizes>>,
}
impl AppState {
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.state)?;
        std::fs::create_dir_all(&config.cache)?;
        let db = Database::open(config.state.join("thelxinoe.sqlite3"))?;
        storage::open_write_media_operations(&db).await?;
        let server_id = storage::open_write_settings(&db).await?;
        let transcoding = accounts::transcoding_policy(&db).await?;
        if !config.state.join("secrets/master.key").exists()
            && storage::open_read_secrets(&db).await? > 0
        {
            anyhow::bail!(
                "The credential master key is missing. Restore the original key before starting the server."
            );
        }
        let secrets = SecretStore::open(&config.state.join("secrets"))?;
        let release_pending = product::maintenance_pending(&db).await?;
        let tools = Arc::new(tools::Manager::new(&config.state));
        Ok(Self {
            server_id: Arc::new(server_id),
            event_epoch: Arc::new(thelxinoe_core::id()),
            playback: Arc::new(
                thelxinoe_playback::Pipelines::open(&config.cache, tools.runtime.clone()).await?,
            ),
            tools,
            transcoding: Arc::new(tokio::sync::RwLock::new(transcoding)),
            subtitle_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            db,
            secrets,
            config: Arc::new(config),
            events: tokio::sync::broadcast::channel(128).0,
            attention: Arc::new(tokio::sync::Notify::new()),
            attention_digests: Arc::new(Default::default()),
            password_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            compatibility_audio: Arc::new(tokio::sync::Mutex::new(())),
            online: Arc::new(online::Runtime::new()?),
            managers: Arc::new(managers::Runtime::new()?),
            media_operations: Arc::new(tokio::sync::RwLock::new(())),
            playback_lifecycle: Arc::new(Default::default()),
            media_resources: Arc::new(Default::default()),
            release_gate: Arc::new(tokio::sync::RwLock::new(())),
            release_quiescing: Arc::new(std::sync::atomic::AtomicBool::new(release_pending)),
            release_validating: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            release_command: Arc::new(tokio::sync::Mutex::new(())),
            release_check: Arc::new(tokio::sync::Mutex::new(())),
            release_install: Arc::new(tokio::sync::Mutex::new(())),
            library_watch: Arc::new(Default::default()),
            folder_sizes: Arc::new(Default::default()),
            dummy_hash: Arc::new(
                thelxinoe_auth::password_hash(thelxinoe_auth::token(), None).await?,
            ),
        })
    }
    pub async fn emit(
        &self,
        user_id: Option<String>,
        kind: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        // Any domain change may resolve or raise an attention item; progress ticks cannot.
        let observe = kind != "attention.changed" && !kind.ends_with(".progress");
        let kind = kind.to_owned();
        storage::emit(&self.db, kind, user_id, payload).await?;
        self.notify_events();
        if observe {
            self.notify_attention();
        }
        Ok(())
    }

    /// Wake subscribers after a storage operation has committed its event rows.
    pub(crate) fn notify_events(&self) {
        let _ = self.events.send(());
    }

    /// Recompute attention after a committed change that emits no event of its own.
    pub(crate) fn notify_attention(&self) {
        self.attention.notify_one();
    }
}
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(authentication::router())
        .merge(product::router())
        .merge(tools::router())
        .merge(jellyfin::router())
        .merge(online::router(state.clone()))
        .merge(managers::router())
        .merge(segments::router())
        .merge(operations::router())
        .merge(backups::router())
        .route("/api/v1/{*path}", axum::routing::any(not_found))
        .route("/api/v1/health", get(health))
        .route(
            "/api/v1/setup",
            get(accounts::setup_status).post(accounts::setup),
        )
        .route("/api/v1/auth/login", post(accounts::login))
        .route("/api/v1/auth/logout", post(accounts::logout))
        .route("/api/v1/auth/me", get(accounts::me))
        .route(
            "/api/v1/me/avatar",
            axum::routing::put(avatars::save).layer(DefaultBodyLimit::max(512 * 1024)),
        )
        .route(
            "/api/v1/me/password",
            axum::routing::put(accounts::change_password),
        )
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
            "/api/v1/me/queue/{client}",
            get(user_media::legacy_queue).put(user_media::legacy_queue),
        )
        .route("/api/v1/timezones", get(timezones::list))
        .route(
            "/api/v1/me/appearance",
            get(appearance::get).patch(appearance::update),
        )
        .route(
            "/api/v1/me/preferences",
            get(timezones::get).put(timezones::update),
        )
        .route("/api/v1/me/history", get(history::mine))
        .route("/api/v1/me/statistics", get(statistics::mine))
        .route("/api/v1/admin/statistics", get(statistics::admin))
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
        .route(
            "/api/v1/catalog/roots/{id}",
            axum::routing::patch(library::rename_root).delete(library::remove_root),
        )
        .route("/api/v1/catalog/roots/{id}/scan", post(library::scan))
        .route("/api/v1/catalog", get(library::browse))
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
            "/api/v1/playback/{id}/remote/{track}",
            get(playback::remote),
        )
        .route(
            "/api/v1/playback/{id}/hls/{revision}/{name}",
            get(playback::hls),
        )
        .route(
            "/api/v1/playback/{id}/subtitles/{track}",
            get(playback::subtitle),
        )
        .route("/api/v1/catalog/{id}/playback", get(playback::media_info))
        .route("/api/v1/admin/jobs", get(realtime::jobs))
        .route(
            "/api/v1/admin/settings",
            get(accounts::settings).put(accounts::save_settings),
        )
        .route("/api/v1/admin/health", get(admin_health))
        .fallback(managers::access::fallback)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}
async fn health() -> Json<serde_json::Value> {
    Json(
        json!({"status":"ok","version":thelxinoe_core::VERSION,"api_version":thelxinoe_core::API_VERSION,"api_min":thelxinoe_core::API_MIN_VERSION,"api_max":thelxinoe_core::API_VERSION}),
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
    tokio::try_join!(run_general_jobs(state.clone()), run_service_jobs(state))?;
    Ok(())
}

/// The first wait after a job lane's database access failed; repeated failures wait longer.
const JOB_RETRY: std::time::Duration = std::time::Duration::from_secs(1);

/// Waits before a job lane retries failed bookkeeping, longer while failures repeat. A
/// database that is shutting down ends the lane instead.
async fn back_off(error: anyhow::Error, delay: &mut std::time::Duration) -> anyhow::Result<()> {
    if matches!(
        error.downcast_ref(),
        Some(thelxinoe_database::AccessError::Closed)
    ) {
        return Err(error);
    }
    tracing::warn!(retry_in = ?delay, "Job bookkeeping failed and will be retried: {error:#}");
    tokio::time::sleep(*delay).await;
    *delay = (*delay * 2).min(std::time::Duration::from_secs(60));
    Ok(())
}

/// Runs one claimed job and records its outcome on that job alone. Neither its error or
/// panic nor a failed event stops its lane, the other lane or the jobs running beside it.
async fn run_job(state: AppState, job: thelxinoe_jobs::Job) {
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(handle_job(&state, &job))
        .catch_unwind()
        .await
        .unwrap_or_else(|panic| {
            let reason = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown cause");
            Err(anyhow::anyhow!("The job stopped unexpectedly: {reason}"))
        });
    // A job that queued itself again has nothing to record yet.
    if !matches!(result, Ok(false)) {
        let error = result.err().map(|e| e.to_string());
        let mut delay = JOB_RETRY;
        while let Err(failed) = Queue(state.db.clone()).finish(&job, error.clone()).await {
            if back_off(failed, &mut delay).await.is_err() {
                return;
            }
        }
    }
    if let Err(error) = state.emit(None, "jobs.changed", json!({"id":job.id})).await {
        tracing::warn!(job = %job.id, "Job change could not be announced: {error:#}");
    }
}

/// Returns false when the job queued itself again instead of finishing.
async fn handle_job(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<bool> {
    match job.kind.as_str() {
        "stack.install" => managers::provision(state, job).await.map(|()| true),
        "service.transfer" => managers::transfer(state, job).await,
        "recyclarr.sync" => managers::recyclarr::run_job(state, job).await,
        "service.update" => managers::update_service(state, job).await,
        "manager.request" => managers::acquire(state, job).await.map(|()| true),
        "metadata.match" | "metadata.refresh" => managers::run_metadata(state, &job.payload)
            .await
            .map(|()| true),
        "library.scan" => scan_library(state, job).await.map(|()| true),
        #[cfg(test)]
        "test.panic" => panic!("test job panicked"),
        _ => anyhow::bail!("Unknown job type"),
    }
}

async fn scan_library(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let roots = thelxinoe_catalog::roots(&state.db).await?;
    let Some(root) = roots
        .into_iter()
        .find(|r| Some(r.id.as_str()) == job.payload["root_id"].as_str())
    else {
        // Removing a library retires its queued scans; one claimed just before has nothing
        // left to do.
        return Ok(());
    };
    let keys = media_resources::for_root(&state.db, &root.id).await?;
    let _files = state
        .media_resources
        .read(&keys.iter().map(String::as_str).collect::<Vec<_>>())
        .await;
    state
        .emit(None, "catalog.scan.started", json!({"root_id":root.id}))
        .await?;
    let scanned = match state.tools.runtime.media() {
        Ok(tools) => {
            thelxinoe_catalog::index_with_progress(
                &state.db,
                root.clone(),
                tools,
                |completed, total| {
                    let state = state.clone();
                    let root_id = root.id.clone();
                    async move {
                        state
                            .emit(
                                None,
                                "catalog.scan.progress",
                                json!({"root_id":root_id,"completed":completed,"total":total}),
                            )
                            .await
                            .map(|_| ())
                    }
                },
            )
            .await
        }
        Err(error) => Err(error),
    };
    // Probing can take minutes; only the write and the reconcile after it
    // hold the media lease, so playback reports never queue behind a scan.
    let _lease = state.media_operations.read().await;
    let result = match scanned {
        Ok(scanned) => scanned.commit(&state.db).await,
        Err(error) => Err(error),
    };
    // A library removed during its scan stopped it; that's no failure.
    let removed = result.is_err()
        && !thelxinoe_catalog::roots(&state.db)
            .await?
            .iter()
            .any(|r| r.id == root.id);
    match &result {
        Ok(_) => managers::reconcile_after_scan(state).await,
        Err(_) if removed => {}
        Err(error) => storage::run_jobs(error.to_string(), root, &state.db).await?,
    }
    state
        .emit(
            None,
            "catalog.changed",
            json!({"root_id":job.payload["root_id"]}),
        )
        .await?;
    if removed {
        return Ok(());
    }
    result.map(|_| ())
}

async fn run_service_jobs(state: AppState) -> anyhow::Result<()> {
    let queue = Queue(state.db.clone());
    let mut running = tokio::task::JoinSet::new();
    let mut delay = JOB_RETRY;
    loop {
        // Each job records its own outcome, so a finished task has nothing left to report.
        while running.try_join_next().is_some() {}
        if running.len() >= 6 {
            running.join_next().await;
            continue;
        }
        let claimed = match managers::recyclarr::tick(&state).await {
            // Every service job needs the controller, which a release validation holds.
            Ok(()) => {
                let _gate = state.release_gate.read().await;
                if state
                    .release_quiescing
                    .load(std::sync::atomic::Ordering::SeqCst)
                    || product::validating(&state)
                {
                    Ok(None)
                } else {
                    queue.claim_services().await
                }
            }
            Err(error) => Err(error),
        };
        match claimed {
            Ok(Some(job)) => {
                delay = JOB_RETRY;
                state.notify_attention();
                running.spawn(run_job(state.clone(), job));
            }
            Ok(None) => {
                delay = JOB_RETRY;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(error) => back_off(error, &mut delay).await?,
        }
    }
}

async fn run_general_jobs(state: AppState) -> anyhow::Result<()> {
    state.tools.wait_ready().await;
    let queue = Queue(state.db.clone());
    let mut delay = JOB_RETRY;
    loop {
        let claimed = {
            let _gate = state.release_gate.read().await;
            if state
                .release_quiescing
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                Ok(None)
            } else {
                queue.claim_general().await
            }
        };
        match claimed {
            Ok(Some(job)) => {
                delay = JOB_RETRY;
                state.notify_attention();
                run_job(state.clone(), job).await;
            }
            Ok(None) => {
                delay = JOB_RETRY;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(error) => back_off(error, &mut delay).await?,
        }
    }
}
pub async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error":{"code":"not_found","message":"Unknown API endpoint"}})),
    )
}

#[cfg(test)]
mod job_tests {
    use crate::{AppState, test_support::fixture};
    use serde_json::json;
    use thelxinoe_jobs::Queue;

    async fn finished(state: &AppState, id: &str) -> (String, Option<String>) {
        for _ in 0..200 {
            let job = id.to_owned();
            let row = state
                .db
                .read("test.job", move |c| {
                    Ok(
                        c.query_row("SELECT state,error FROM jobs WHERE id=?1", [job], |r| {
                            Ok((r.get::<_, String>(0)?, r.get(1)?))
                        })?,
                    )
                })
                .await
                .unwrap();
            if !["queued", "running"].contains(&row.0.as_str()) {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("Job {id} did not finish");
    }

    #[tokio::test]
    async fn a_panicking_job_fails_alone_and_both_lanes_keep_working() {
        let (_temp, state, _) = fixture().await;
        let queue = Queue(state.db.clone());
        let lanes = tokio::spawn(super::run_jobs(state.clone()));
        let panicking = queue.enqueue("test.panic", "a", json!({})).await.unwrap();
        assert_eq!(
            finished(&state, &panicking).await,
            (
                "failed".into(),
                Some("The job stopped unexpectedly: test job panicked".into())
            )
        );
        let general = queue
            .enqueue("library.scan", "gone", json!({"root_id":"gone"}))
            .await
            .unwrap();
        let service = queue
            .enqueue("service.transfer", "t", json!({}))
            .await
            .unwrap();
        assert_eq!(
            finished(&state, &general).await,
            ("failed".into(), Some("Library no longer exists".into()))
        );
        assert_eq!(
            finished(&state, &service).await,
            ("failed".into(), Some("Missing transfer identity".into()))
        );
        assert!(!lanes.is_finished());
        lanes.abort();
    }

    #[tokio::test]
    async fn a_failed_claim_waits_and_retries_instead_of_stopping_the_lanes() {
        let (_temp, state, _) = fixture().await;
        let queue = Queue(state.db.clone());
        let job = queue.enqueue("test.unknown", "a", json!({})).await.unwrap();
        state
            .db
            .write("test.fail_claims", |c| {
                c.execute_batch("CREATE TEMP TRIGGER reject_claim BEFORE UPDATE ON jobs BEGIN SELECT RAISE(ABORT,'test claim failure'); END;")?;
                Ok(())
            })
            .await
            .unwrap();
        let lanes = tokio::spawn(super::run_jobs(state.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!lanes.is_finished());
        state
            .db
            .write("test.allow_claims", |c| {
                c.execute_batch("DROP TRIGGER reject_claim")?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            finished(&state, &job).await,
            ("failed".into(), Some("Unknown job type".into()))
        );
        assert!(!lanes.is_finished());
        lanes.abort();
    }
}
