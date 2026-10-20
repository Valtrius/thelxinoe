use anyhow::Result;
use thelxinoe_server::{AppState, config::Config, router, run_jobs};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thelxinoe=info".into()),
        )
        .init();
    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.state)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.state.join("server.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    if let Some(command) = std::env::args().nth(1) {
        return match command.as_str() {
            "validate-state" => thelxinoe_server::validation::run(config).await,
            "recover-user" => {
                let username = std::env::args().nth(2).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Usage: thelxinoe-server recover-user <username> (stop the server first)"
                    )
                })?;
                println!(
                    "{}",
                    thelxinoe_server::authentication::recover_from_host(config, username).await?
                );
                Ok(())
            }
            _ => anyhow::bail!("Unknown server command"),
        };
    }
    let bind = config.bind;
    let state = AppState::open(config).await?;
    thelxinoe_server::tools::initialize(&state).await?;
    if let Err(error) = thelxinoe_server::operations::prime_attention(&state).await {
        tracing::warn!(%error, "Attention could not be read; the first check records it");
    }
    tracing::info!(version=thelxinoe_core::VERSION, %bind, "Starting Thelxinoe");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(supervise("jobs", state.clone(), run_jobs));
    workers.spawn(supervise(
        "tools",
        state.clone(),
        thelxinoe_server::tools::run,
    ));
    workers.spawn(supervise(
        "library",
        state.clone(),
        thelxinoe_server::library::reconcile,
    ));
    workers.spawn(supervise(
        "playback",
        state.clone(),
        thelxinoe_server::playback::maintain,
    ));
    workers.spawn(supervise(
        "maintenance",
        state.clone(),
        thelxinoe_server::maintenance::run,
    ));
    // LAN discovery errors are invalid settings or an unavailable port; they stay fatal.
    workers.spawn(thelxinoe_server::run_discovery(state.clone()));
    workers.spawn(supervise(
        "online",
        state.clone(),
        thelxinoe_server::run_online,
    ));
    workers.spawn(supervise(
        "downloads",
        state.clone(),
        thelxinoe_server::run_downloads,
    ));
    workers.spawn(supervise(
        "service updates",
        state.clone(),
        thelxinoe_server::run_service_updates,
    ));
    workers.spawn(supervise(
        "service connections",
        state.clone(),
        thelxinoe_server::run_service_connections,
    ));
    workers.spawn(supervise(
        "retention",
        state.clone(),
        thelxinoe_server::run_retention,
    ));
    workers.spawn(supervise(
        "title status",
        state.clone(),
        thelxinoe_server::run_title_status,
    ));
    workers.spawn(supervise(
        "segments",
        state.clone(),
        thelxinoe_server::segments::run,
    ));
    workers.spawn(supervise(
        "operations",
        state.clone(),
        thelxinoe_server::operations::run,
    ));
    workers.spawn(supervise(
        "attention",
        state.clone(),
        thelxinoe_server::operations::watch,
    ));
    workers.spawn(supervise(
        "backups",
        state.clone(),
        thelxinoe_server::backups::run,
    ));
    workers.spawn(supervise(
        "product updates",
        state.clone(),
        thelxinoe_server::product::run,
    ));
    let server = axum::serve(
        listener,
        router(state.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown());
    let result: Result<()> = tokio::select! {
        result = server => result.map_err(Into::into),
        result = workers.join_next() => match result {
            Some(Ok(result)) => result,
            Some(Err(error)) => Err(error.into()),
            None => Ok(()),
        },
    };
    // Stop producers first. Already accepted storage operations still finish.
    workers.shutdown().await;
    let drained = state.db.shutdown().await;
    result.and(drained)
}
/// The longest wait before a failed worker restarts, and how long one must have run for
/// its next failure to restart it promptly again.
const RESTART_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Keeps a background worker running across errors, such as a failed database write or an
/// unreachable service, instead of stopping playback and every other worker with it. A
/// restarted worker repeats its startup recovery, as after a server restart. A panic or a
/// worker that finishes still stops the server.
async fn supervise<F, Fut>(name: &'static str, state: AppState, worker: F) -> Result<()>
where
    F: Fn(AppState) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    restart(name, std::time::Duration::from_secs(1), || {
        worker(state.clone())
    })
    .await
}
async fn restart<F, Fut>(name: &'static str, first: std::time::Duration, mut start: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut delay = first;
    loop {
        let started = tokio::time::Instant::now();
        let Err(error) = start().await else {
            return Ok(());
        };
        if started.elapsed() >= RESTART_LIMIT {
            delay = first;
        }
        tracing::error!(worker = name, retry_in = ?delay, "Background worker failed and will restart: {error:#}");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RESTART_LIMIT);
    }
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn a_failed_worker_restarts_until_it_finishes() {
        let attempts = &AtomicUsize::new(0);
        super::restart(
            "test",
            std::time::Duration::from_millis(1),
            move || async move {
                match attempts.fetch_add(1, Ordering::SeqCst) {
                    0 | 1 => Err(anyhow::anyhow!("transient")),
                    _ => Ok(()),
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
