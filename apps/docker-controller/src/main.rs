#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let directory = std::path::PathBuf::from(
        std::env::var("THELXINOE_RUNTIME").unwrap_or("/run/thelxinoe".into()),
    );
    std::fs::create_dir_all(&directory)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("controller.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    let socket = directory.join("controller.sock");
    if socket.exists() {
        if !std::fs::symlink_metadata(&socket)?.file_type().is_socket() {
            anyhow::bail!("Refusing to replace a non-socket path");
        }
        std::fs::remove_file(&socket)?;
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))?;
    let app = axum::Router::new().route(
        "/health",
        axum::routing::get(|| async {
            axum::Json(
                serde_json::json!({"status":"ok","version":thelxinoe_core::VERSION,"protocol":1}),
            )
        }),
    );
    axum::serve(listener, app).await?;
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The Docker controller requires Linux. Use Docker Compose for local development.");
    std::process::exit(1);
}
