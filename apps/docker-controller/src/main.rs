#[cfg(unix)]
mod contract;
#[cfg(unix)]
mod docker;
#[cfg(unix)]
mod identity;
#[cfg(unix)]
mod lease;
#[cfg(unix)]
mod policy;
#[cfg(unix)]
mod recovery;
#[cfg(unix)]
mod service_releases;
#[cfg(unix)]
mod stack;
#[cfg(unix)]
mod state_copy;
#[cfg(unix)]
mod store;
#[cfg(unix)]
mod templates;
#[cfg(all(unix, test))]
mod test_support;
#[cfg(unix)]
mod transfer_plan;
#[cfg(unix)]
mod transfer_worker;
#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg = |n: usize| std::env::args().nth(n).unwrap_or_default();
    if let Some(command) = std::env::args().nth(1) {
        return match command.as_str() {
            "recyclarr-check" => contract::recyclarr().await,
            "recyclarr-seed" => contract::recyclarr_seed().await,
            "recyclarr-fixture" => {
                std::future::pending::<()>().await;
                Ok(())
            }
            "adapter-contract" => {
                contract::run(&std::env::args().nth(2).unwrap_or_default(), true).await
            }
            "adapter-health" => {
                contract::run(&std::env::args().nth(2).unwrap_or_default(), false).await
            }
            "snapshot-copy" => state_copy::snapshot(false),
            "snapshot-restore" => state_copy::snapshot(true),
            "transfer-inspect" => transfer_worker::inspect(&arg(2), &arg(3)),
            "transfer-prepare" => transfer_worker::prepare(&arg(2), &arg(3)),
            "transfer-fixture" => transfer_worker::fixture(),
            "transfer-access" => transfer_worker::access(),
            "transfer-digest" => transfer_worker::digest(),
            "transfer-effects" => transfer_worker::effects(&arg(2)),
            "transfer-contract" => contract::transfer(&arg(2), &arg(3)).await,
            "appdata-remove" => state_copy::remove(),
            "verify-state" => {
                let root = std::path::Path::new("/state");
                let schema = thelxinoe_database::verify_snapshot(&root.join("thelxinoe.sqlite3"))?;
                anyhow::ensure!(
                    std::fs::metadata(root.join("secrets/master.key"))?.len() == 32,
                    "Invalid credential key"
                );
                store::write_json(
                    &root.join(".snapshot-validation.json"),
                    &serde_json::json!({"schema":schema,"verified":true}),
                )
            }
            "controller-probe" => store::write_json(
                std::path::Path::new("/probe/controller.json"),
                &serde_json::json!({"version":thelxinoe_core::VERSION,"recovery_protocol":1}),
            ),
            _ => anyhow::bail!("Unknown worker command"),
        };
    }
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let directory = std::path::PathBuf::from(
        std::env::var("THELXINOE_RUNTIME").unwrap_or("/run/thelxinoe".into()),
    );
    std::fs::create_dir_all(&directory)?;
    loop {
        stack::product::standby().await?;
        let handoff = std::env::var("THELXINOE_HANDOFF").is_ok();
        let mut attempts = 0;
        let mut adopted = false;
        let writer = loop {
            attempts += 1;
            match lease::Lease::acquire(&directory) {
                Ok(writer) => {
                    if stack::product::accepted_writer().await? {
                        break writer;
                    }
                    drop(writer);
                    // Compose recreated these containers; a successor waits for its handoff.
                    if !handoff {
                        anyhow::ensure!(!adopted, "The adopted containers were not accepted");
                        stack::product::adopt_local(&directory).await?;
                        adopted = true;
                        continue;
                    }
                }
                Err(e) if !handoff => return Err(e),
                Err(_) => (),
            }
            anyhow::ensure!(
                attempts < 180,
                "Successor was not accepted before the handoff deadline"
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        };
        identity::prepare_runtime(&directory).await?;
        let socket = directory.join("controller.sock");
        stack::retain_worker_image().await?;
        stack::product::startup().await?;
        stack::recover_backups().await?;
        if socket.exists() {
            if !std::fs::symlink_metadata(&socket)?.file_type().is_socket() {
                anyhow::bail!("Refusing to replace a non-socket path");
            }
            std::fs::remove_file(&socket)?;
        }
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))?;
        let app = axum::Router::new().merge(docker::router()).merge(stack::router()).route(
        "/health",
        axum::routing::get(|| async {
            axum::Json(
                serde_json::json!({"status":"ok","version":thelxinoe_core::VERSION,"protocol":1}),
            )
        }),
    );
        tokio::select! {
            result = axum::serve(listener, app) => result?,
            _ = lease::HANDOFF.notified() => {
                drop(writer);
                if stack::product::watchdog(&directory).await? { continue; }
            }
        }
        break;
    }
    Ok(())
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The Docker controller requires Linux. Use Docker Compose for local development.");
    std::process::exit(1);
}
