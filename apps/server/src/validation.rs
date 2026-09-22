//! One-shot database and release validation. No listeners, job runners or provider calls are started.
use crate::{AppState, config::Config};
use anyhow::{Result, ensure};
pub async fn run(config: Config) -> Result<()> {
    ensure!(
        config.state.join("thelxinoe.sqlite3").is_file(),
        "Validation requires existing state"
    );
    ensure!(
        std::fs::metadata(config.state.join("secrets/master.key"))?.len() == 32,
        "Validation requires the original credential key"
    );
    let state = AppState::open(config).await?;
    let schema =
        thelxinoe_database::verify_snapshot(&state.config.state.join("thelxinoe.sqlite3"))?;
    ensure!(
        schema == thelxinoe_database::SCHEMA_VERSION,
        "Unexpected database schema"
    );
    ensure!(
        state.config.web.join("index.html").is_file(),
        "Missing web bundle"
    );
    let report = serde_json::json!({"format":1,"version":thelxinoe_core::VERSION,"schema":schema,"api":thelxinoe_core::API_VERSION});
    let destination = state.config.state.join(".release-validation.json");
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(destination)?;
    file.write_all(&serde_json::to_vec(&report)?)?;
    file.sync_all()?;
    Ok(())
}
