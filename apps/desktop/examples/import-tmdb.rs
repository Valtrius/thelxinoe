#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "Usage: import-tmdb <state directory> <token file>"
    );
    let token = std::fs::read_to_string(&args[2])?;
    anyhow::ensure!(token.trim().len() >= 20, "Invalid TMDB token");
    let path = std::path::Path::new(&args[1]);
    anyhow::ensure!(
        path.join("secrets/master.key").is_file(),
        "Start the destination server once before importing credentials"
    );
    let db = thelxinoe_database::Database::open(path.join("thelxinoe.sqlite3"))?;
    let secrets = thelxinoe_auth::SecretStore::open(&path.join("secrets"))?;
    secrets
        .put(&db, "provider.tmdb".into(), token.trim().as_bytes())
        .await?;
    println!("Imported TMDB token into encrypted server storage");
    Ok(())
}
