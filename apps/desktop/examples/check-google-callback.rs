//! Inspect the existing application's callback acceptance without signing in,
//! changing YouTwitch, or printing credentials/provider response bodies.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let redirect = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Pass the proposed absolute Google callback URL"))?;
    let redirect = url::Url::parse(&redirect)?;
    anyhow::ensure!(
        matches!(redirect.scheme(), "https" | "http")
            && redirect.host_str().is_some()
            && redirect.username().is_empty()
            && redirect.password().is_none(),
        "Invalid callback URL"
    );
    let entry = keyring::Entry::new("app.youtwitch.desktop", "oauth-client-configuration")?;
    let value: serde_json::Value = serde_json::from_str(&entry.get_password()?)?;
    let client = value["google"]["client_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No Google application configured in YouTwitch"))?;
    let http = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let response = http
        .get("https://accounts.google.com/o/oauth2/v2/auth")
        .query(&[
            ("client_id", client),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", "https://www.googleapis.com/auth/youtube.readonly"),
            ("access_type", "offline"),
            ("state", "callback-validation-no-login"),
        ])
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Google callback check could not reach the authorization endpoint")
        })?;
    let status = response.status();
    let final_url = response.url().as_str().to_owned();
    let text = response
        .text()
        .await
        .map_err(|_| anyhow::anyhow!("Google callback check received an unreadable response"))?;
    let result =
        if text.contains("redirect_uri_mismatch") || final_url.contains("redirect_uri_mismatch") {
            "redirect_uri_mismatch"
        } else if text.contains("invalid_client") || final_url.contains("invalid_client") {
            "invalid_client"
        } else if text.contains("invalid_request") || final_url.contains("invalid_request") {
            "invalid_request"
        } else if response_path_is_error(&final_url) {
            "authorization_request_rejected"
        } else if status.is_success() {
            "interactive_validation_required"
        } else {
            "authorization_request_rejected"
        };
    println!("Google callback check: {result} (HTTP {})", status.as_u16());
    Ok(())
}
fn response_path_is_error(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        u.path().contains("/oauth/error") || u.query_pairs().any(|(k, _)| k == "authError")
    })
}
