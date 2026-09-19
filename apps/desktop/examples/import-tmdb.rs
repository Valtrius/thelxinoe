#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3 || args.len() == 4,
        "Usage: import-tmdb <state directory> <token file> [MusicBrainz contact file]"
    );
    let token = std::fs::read_to_string(&args[2])?;
    anyhow::ensure!(token.trim().len() >= 20, "Invalid TMDB token");
    let contact = args
        .get(3)
        .map(std::fs::read_to_string)
        .transpose()?
        .map(|s| s.trim().to_string());
    if let Some(value) = &contact {
        anyhow::ensure!(
            !value.is_empty()
                && value.len() <= 200
                && !value.contains(['\r', '\n'])
                && (value.contains('@') || value.starts_with("https://")),
            "Invalid MusicBrainz contact"
        );
    }
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
    if let Some(contact) = contact {
        db.call(move |db| { db.execute("INSERT INTO settings(key,value) VALUES ('musicbrainz_contact',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [contact])?; Ok(()) }).await?;
    }
    println!("Imported TMDB token into encrypted server storage");
    Ok(())
}
