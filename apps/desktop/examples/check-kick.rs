// Read-only live-provider probe. Prints public channel slugs, never credentials.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let entry = keyring::Entry::new("app.youtwitch.desktop", "oauth-client-configuration")?;
    let configuration: serde_json::Value = serde_json::from_str(&entry.get_password()?)?;
    let client = configuration["kick"]["client_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Kick application is not configured"))?;
    let secret = configuration["kick"]["client_secret"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Kick application is not configured"))?;
    let http = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let response = http
        .post("https://id.kick.com/oauth/token")
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client),
            ("client_secret", secret),
        ])
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "Kick rejected the application"
    );
    let token: serde_json::Value = response.json().await?;
    let response = http
        .get("https://api.kick.com/public/v1/livestreams")
        .query(&[("limit", "5")])
        .bearer_auth(
            token["access_token"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid token response"))?,
        )
        .send()
        .await?;
    anyhow::ensure!(
        response.status().is_success(),
        "Kick livestream listing failed"
    );
    let listing: serde_json::Value = response.json().await?;
    let channels = listing["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid livestream response"))?
        .iter()
        .filter_map(|r| r["slug"].as_str().or(r["channel"]["slug"].as_str()))
        .take(5)
        .collect::<Vec<_>>();
    println!("{}", serde_json::json!({"channels":channels}));
    Ok(())
}
