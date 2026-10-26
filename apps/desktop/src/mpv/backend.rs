use anyhow::Result;
use serde_json::Value;
use std::time::Duration;

/// A response the server refused; its status says whether repeating the request can help.
#[derive(Debug)]
pub struct Rejected {
    pub status: reqwest::StatusCode,
    pub(super) message: String,
}
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Rejected {}

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
            client: crate::http().map_err(anyhow::Error::msg)?,
            origin: crate::server_url(app.clone()).map_err(anyhow::Error::msg)?,
            token: crate::credential(app)
                .map_err(anyhow::Error::msg)?
                .get_password()?,
        })
    }
    pub async fn call(&self, path: &str, method: &str, body: Option<Value>) -> Result<Value> {
        loop {
            let mut request = self
                .client
                .request(method.parse()?, format!("{}/api/v1{path}", self.origin))
                .bearer_auth(&self.token)
                .header("X-Thelxinoe-Client", "1")
                .header("X-Thelxinoe-API", thelxinoe_core::API_VERSION.to_string());
            if let Some(body) = &body {
                request = request.json(body);
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
                if status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                    && value["error"]["code"] == "tools_preparing"
                {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                return Err(Rejected {
                    status,
                    message: value["error"]["message"]
                        .as_str()
                        .unwrap_or("The server rejected the playback request")
                        .into(),
                }
                .into());
            }
            return Ok(value);
        }
    }
}
