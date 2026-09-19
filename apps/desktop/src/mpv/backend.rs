use anyhow::{Result, bail};
use serde_json::Value;
use std::time::Duration;

// Capture one server and device credential for the entire MPV run. A change-server
// operation cannot accidentally report this run's progress to a different server.
pub struct Backend {
    client: reqwest::Client,
    pub origin: String,
    token: String,
}
impl Backend {
    pub fn new(app: &tauri::AppHandle) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?,
            origin: crate::server_url(app.clone()).map_err(anyhow::Error::msg)?,
            token: crate::credential()
                .map_err(anyhow::Error::msg)?
                .get_password()?,
        })
    }
    pub async fn call(&self, path: &str, method: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .client
            .request(method.parse()?, format!("{}/api/v1{path}", self.origin))
            .bearer_auth(&self.token)
            .header("X-Thelxinoe-Client", "1");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("The server connection was interrupted"))?;
        let status = response.status();
        let value: Value = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("The server returned an invalid response"))?;
        if !status.is_success() {
            bail!(
                "{}",
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("The server rejected the playback request")
            );
        }
        Ok(value)
    }
}
