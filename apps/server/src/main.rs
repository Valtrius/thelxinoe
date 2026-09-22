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
        anyhow::ensure!(command == "validate-state", "Unknown server command");
        return thelxinoe_server::validation::run(config).await;
    }
    let bind = config.bind;
    let state = AppState::open(config).await?;
    tracing::info!(version=thelxinoe_core::VERSION, %bind, "Starting Thelxinoe");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(run_jobs(state.clone()));
    workers.spawn(thelxinoe_server::library::reconcile(state.clone()));
    workers.spawn(thelxinoe_server::playback::maintain(state.clone()));
    workers.spawn(thelxinoe_server::run_discovery(state.clone()));
    workers.spawn(thelxinoe_server::run_online(state.clone()));
    workers.spawn(thelxinoe_server::run_downloads(state.clone()));
    workers.spawn(thelxinoe_server::run_service_updates(state.clone()));
    workers.spawn(thelxinoe_server::run_retention(state.clone()));
    workers.spawn(thelxinoe_server::segments::run(state.clone()));
    workers.spawn(thelxinoe_server::operations::run(state.clone()));
    workers.spawn(thelxinoe_server::product::run(state.clone()));
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
