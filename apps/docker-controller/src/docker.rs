//! Read-only Docker evidence. Never forwards arbitrary engine paths or configuration.
use axum::{Json, Router, extract::Path, http::StatusCode, routing::get};
use serde_json::{Value, json};
pub(crate) type Result<T> = std::result::Result<T, (StatusCode, &'static str)>;
pub fn router() -> Router {
    Router::new()
        .route("/docker/containers", get(list))
        .route("/docker/containers/{id}", get(inspect))
}
pub(crate) fn unavailable() -> (StatusCode, &'static str) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Docker inspection unavailable",
    )
}
pub(crate) async fn engine(path: &str) -> Result<Value> {
    request(reqwest::Method::GET, path, None).await
}
pub(crate) async fn ensure_pinned_image(image: &str) -> Result<Value> {
    let path = format!("/images/{image}/json");
    match engine(&path).await {
        Ok(value) => Ok(value),
        Err((StatusCode::NOT_FOUND, _)) => {
            pull_image(image, |_| Ok(())).await?;
            engine(&path).await
        }
        Err(error) => Err(error),
    }
}
pub(crate) async fn pull_image(
    image: &str,
    mut progress: impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    if !crate::lease::active() {
        return Err((
            StatusCode::CONFLICT,
            "Controller does not hold the Docker mutation lease",
        ));
    }
    let client = reqwest::Client::builder()
        .unix_socket("/var/run/docker.sock")
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .map_err(|_| unavailable())?;
    let mut response = client
        .post("http://docker/images/create")
        .query(&[("fromImage", image)])
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let mut pending = Vec::new();
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(120), response.chunk())
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        let Some(chunk) = chunk else { break };
        for byte in chunk {
            if byte == b'\n' {
                pull_event(&pending, &mut progress)?;
                pending.clear();
            } else {
                if pending.len() >= 64 * 1024 {
                    return Err(unavailable());
                }
                pending.push(byte);
            }
        }
    }
    pull_event(&pending, &mut progress)
}
fn pull_event(bytes: &[u8], progress: &mut impl FnMut(&Value) -> Result<()>) -> Result<()> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| unavailable())?;
    if value.get("error").is_some() || value.get("errorDetail").is_some() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "Image download failed"));
    }
    progress(&value)
}
pub(crate) async fn request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    if method != reqwest::Method::GET && !crate::lease::active() {
        return Err((
            StatusCode::CONFLICT,
            "Controller does not hold the Docker mutation lease",
        ));
    }
    let client = reqwest::Client::builder()
        .unix_socket("/var/run/docker.sock")
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|_| unavailable())?;
    let mut request = client.request(method, format!("http://docker{path}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request.send().await.map_err(|_| unavailable())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err((StatusCode::NOT_FOUND, "Container no longer exists"));
    }
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(Value::Null);
    }
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(unavailable());
        }
        bytes.extend(chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice::<Value>(&bytes)
        .and_then(|value| {
            if value.get("error").is_some() {
                Err(<serde_json::Error as serde::de::Error>::custom(
                    "Docker operation failed",
                ))
            } else {
                Ok(value)
            }
        })
        .or_else(|_| {
            // Image pull returns newline-delimited progress objects. Never return registry bodies.
            for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                let entry: Value = serde_json::from_slice(line)?;
                if entry.get("error").is_some() {
                    return Err(<serde_json::Error as serde::de::Error>::custom(
                        "Image pull failed",
                    ));
                }
            }
            Ok(json!({"complete":true}))
        })
        .map_err(|_| unavailable())
}
async fn list() -> Result<Json<Value>> {
    let data = engine("/containers/json?all=true").await?;
    let items=data.as_array().ok_or_else(unavailable)?.iter().take(1000).map(|c|json!({"id":c["Id"],"names":c["Names"],"image":c["Image"],"image_id":c["ImageID"],"state":c["State"],"ports":c["Ports"],"compose_project":c["Labels"]["com.docker.compose.project"]})).collect::<Vec<_>>();
    Ok(Json(json!({"items":items})))
}
async fn inspect(Path(id): Path<String>) -> Result<Json<Value>> {
    if !(12..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((StatusCode::BAD_REQUEST, "Use a Docker container ID"));
    }
    Ok(Json(redact(
        &engine(&format!("/containers/{id}/json")).await?,
    )?))
}
fn redact(c: &Value) -> Result<Value> {
    let mounts=c["Mounts"].as_array().ok_or_else(unavailable)?.iter().map(|m|json!({"kind":m["Type"],"source":m["Source"],"destination":m["Destination"],"writable":m["RW"]})).collect::<Vec<_>>();
    let networks=c["NetworkSettings"]["Networks"].as_object().ok_or_else(unavailable)?.iter().map(|(name,n)|json!({"name":name,"id":n["NetworkID"],"address":n["IPAddress"],"ipv6":n["GlobalIPv6Address"],"aliases":n["Aliases"]})).collect::<Vec<_>>();
    Ok(
        json!({"id":c["Id"],"name":c["Name"],"image":c["Config"]["Image"],"image_id":c["Image"],"running":c["State"]["Running"],"mounts":mounts,"networks":networks,"compose_project":c["Config"]["Labels"]["com.docker.compose.project"]}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_excludes_environment_commands_and_unrelated_labels() {
        let raw = json!({"Id":"abc","Config":{"Env":["API_KEY=private-secret"],"Cmd":["private-command"],"Image":"image","Labels":{"secret":"private-label"}},"Mounts":[{"Source":"/media","Destination":"/media","RW":true,"Type":"bind"}],"NetworkSettings":{"Networks":{"test":{"NetworkID":"network","IPAddress":"172.18.0.2"}}}});
        let result = redact(&raw).unwrap();
        assert!(!result.to_string().contains("private"));
        assert_eq!(result["mounts"][0]["destination"], "/media");
        assert_eq!(result["networks"][0]["address"], "172.18.0.2");
    }
}
