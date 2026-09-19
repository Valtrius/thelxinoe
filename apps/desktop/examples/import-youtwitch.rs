// Read-only access to YouTwitch's application configuration. Never imports viewer
// tokens: those must be linked to a particular Thelxinoe user through OAuth.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let destination = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Pass the Thelxinoe state directory"))?;
    let entry = keyring::Entry::new("app.youtwitch.desktop", "oauth-client-configuration")?;
    let value: serde_json::Value = serde_json::from_str(&entry.get_password()?)?;
    let state = std::path::Path::new(&destination);
    anyhow::ensure!(
        state.join("secrets/master.key").is_file(),
        "Start the destination server once before importing credentials"
    );
    let db = thelxinoe_database::Database::open(state.join("thelxinoe.sqlite3"))?;
    let store = thelxinoe_auth::SecretStore::open(&state.join("secrets"))?;
    for (source, target) in [
        ("google", "provider.google"),
        ("twitch_client_id", "provider.twitch_client_id"),
        ("kick", "provider.kick"),
    ] {
        if let Some(configuration) = value.get(source).filter(|v| !v.is_null()) {
            store
                .put(&db, target.into(), &serde_json::to_vec(configuration)?)
                .await?;
            println!("Imported encrypted application configuration: {source}");
        } else {
            println!("Not configured in YouTwitch: {source}");
        }
    }
    Ok(())
}
