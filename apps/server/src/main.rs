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
    let bind = config.bind;
    let state = AppState::open(config).await?;
    tracing::info!(version=thelxinoe_core::VERSION, %bind, "Starting Thelxinoe; first-run code is in the state directory at secrets/setup-token");
    let worker = tokio::spawn(run_jobs(state.clone()));
    let scanner = tokio::spawn(thelxinoe_server::library::reconcile(state.clone()));
    let playback = tokio::spawn(thelxinoe_server::playback::maintain(state.clone()));
    let discovery = tokio::spawn(thelxinoe_server::run_discovery(state.clone()));
    let online = tokio::spawn(thelxinoe_server::run_online(state.clone()));
    let downloads = tokio::spawn(thelxinoe_server::run_downloads(state.clone()));
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let server = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown());
    tokio::select! {result=server=>result?,result=worker=>{result??;},result=scanner=>{result??;},result=playback=>{result??;},result=discovery=>{result??;},result=online=>{result??;},result=downloads=>{result??;}}
    Ok(())
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
