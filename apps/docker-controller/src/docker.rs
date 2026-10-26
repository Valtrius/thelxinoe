//! Read-only Docker evidence. Never forwards arbitrary engine paths or configuration.
use axum::{Json, Router, extract::Path, http::StatusCode, routing::get};
use serde_json::{Value, json};
use std::borrow::Cow;
pub(crate) type Failure = (StatusCode, Cow<'static, str>);
pub(crate) type Result<T> = std::result::Result<T, Failure>;
pub fn router() -> Router {
    Router::new()
        .route("/docker/containers", get(list))
        .route("/docker/containers/{id}", get(inspect))
}
pub(crate) fn unavailable() -> Failure {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Docker inspection unavailable".into(),
    )
}
/// The Docker socket itself did not answer, or not in time; a change may or
/// may not have happened.
pub(crate) fn unreachable(error: &reqwest::Error) -> Failure {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        if error.is_timeout() {
            "Docker did not answer in time".into()
        } else {
            "Docker is not reachable".into()
        },
    )
}
/// Resource limits for a worker container. CPU time uses period and quota: an
/// engine without CFS bandwidth control (Synology kernels) drops those with a
/// warning, but refuses `NanoCpus`.
pub(crate) fn limit(spec: &mut Value, memory: u64, cpus: u64, pids: u64) {
    let host = &mut spec["HostConfig"];
    host["Memory"] = json!(memory);
    host["CpuPeriod"] = json!(100_000);
    host["CpuQuota"] = json!(cpus * 100_000);
    host["PidsLimit"] = json!(pids);
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
    progress: impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    let started = std::time::Instant::now();
    let result = pull_image_stream(image, progress).await;
    eprintln!(
        "Docker image pull: image={image} elapsed_ms={} outcome={}",
        started.elapsed().as_millis(),
        if result.is_ok() { "complete" } else { "failed" }
    );
    result
}
async fn pull_image_stream(
    image: &str,
    mut progress: impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    if !crate::lease::active() {
        return Err((
            StatusCode::CONFLICT,
            "Controller does not hold the Docker mutation lease".into(),
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
        .map_err(|error| {
            pull_failure(
                image,
                "connect",
                if error.is_timeout() {
                    "Image download timed out"
                } else {
                    "Image download transport failed"
                },
            )
        })?;
    if !response.status().is_success() {
        let status = response.status();
        eprintln!("Docker image pull: image={image} http_status={status}");
        let body = response.json::<Value>().await.unwrap_or_default();
        let category = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                "Image registry authorization failed"
            }
            StatusCode::NOT_FOUND => "Image registry manifest not found",
            StatusCode::TOO_MANY_REQUESTS => "Image registry rate limit exceeded",
            _ => pull_category(body["message"].as_str().unwrap_or_default()),
        };
        return Err(pull_failure(image, "response", category));
    }
    let mut pending = Vec::new();
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(120), response.chunk())
            .await
            .map_err(|_| pull_failure(image, "stream", "Image download stalled"))?
            .map_err(|_| pull_failure(image, "stream", "Image download transport failed"))?;
        let Some(chunk) = chunk else { break };
        for byte in chunk {
            if byte == b'\n' {
                pull_event(&pending, image, &mut progress)?;
                pending.clear();
            } else {
                if pending.len() >= 64 * 1024 {
                    return Err(pull_failure(
                        image,
                        "stream",
                        "Image download event exceeds size limit",
                    ));
                }
                pending.push(byte);
            }
        }
    }
    pull_event(&pending, image, &mut progress)
}
fn pull_failure(image: &str, stage: &str, reason: &'static str) -> Failure {
    eprintln!("Docker image pull: image={image} stage={stage} category={reason}");
    (StatusCode::SERVICE_UNAVAILABLE, reason.into())
}
fn pull_category(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if ["unauthorized", "denied", "authentication", "forbidden"]
        .iter()
        .any(|value| message.contains(value))
    {
        "Image registry authorization failed"
    } else if message.contains("manifest unknown") || message.contains("not found") {
        "Image registry manifest not found"
    } else if message.contains("429")
        || message.contains("toomanyrequests")
        || message.contains("rate limit")
    {
        "Image registry rate limit exceeded"
    } else if message.contains("no space") {
        "Docker image storage is full"
    } else if message.contains("timeout") || message.contains("timed out") {
        "Image download timed out"
    } else {
        "Image download failed"
    }
}
fn pull_event(
    bytes: &[u8],
    image: &str,
    progress: &mut impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| pull_failure(image, "parse", "Invalid image download response"))?;
    if value.get("error").is_some() || value.get("errorDetail").is_some() {
        let message = value["errorDetail"]["message"]
            .as_str()
            .or(value["error"].as_str())
            .unwrap_or_default();
        return Err(pull_failure(image, "stream", pull_category(message)));
    }
    progress(&value)
}
pub(crate) async fn request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    #[cfg(test)]
    if let Ok(result) = crate::test_support::DOCKER
        .try_with(|transport| transport.request(&method, path, body.as_ref()))
    {
        return result;
    }
    if method != reqwest::Method::GET && !crate::lease::active() {
        return Err((
            StatusCode::CONFLICT,
            "Controller does not hold the Docker mutation lease".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .unix_socket("/var/run/docker.sock")
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|_| unavailable())?;
    let mut request = client.request(method.clone(), format!("http://docker{path}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request.send().await.map_err(|error| {
        eprintln!("Docker request {path} failed: {error}");
        unreachable(&error)
    })?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        eprintln!("Docker request {path} failed while reading: {error}");
        unreachable(&error)
    })? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            eprintln!("Docker request {path} returned more than 8 MiB");
            return Err(unavailable());
        }
        bytes.extend(chunk);
    }
    if status == StatusCode::NOT_MODIFIED {
        return Ok(Value::Null);
    }
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body["message"].as_str().map(str::to_owned))
            .unwrap_or_default();
        eprintln!("Docker request {path} returned HTTP {status}: {message}");
        return Err(engine_failure(&method, path, status, &message));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| {
        eprintln!("Docker request {path} returned invalid JSON");
        unavailable()
    })?;
    if let Some(error) = value.get("error") {
        eprintln!("Docker request {path} reported an error: {error}");
        return Err(unavailable());
    }
    // An engine that cannot apply a setting (CPU or process limits) drops it and says so.
    if let Some(warnings) = value["Warnings"].as_array().filter(|w| !w.is_empty()) {
        eprintln!(
            "Docker request {path} warned: {}",
            Value::Array(warnings.clone())
        );
    }
    Ok(value)
}
/// What a refused request was doing, without echoing query strings or bodies.
fn operation(method: &reqwest::Method, path: &str) -> String {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let parts = route.trim_start_matches('/').split('/').collect::<Vec<_>>();
    let short = |id: &str| {
        if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            id[..12].to_owned()
        } else {
            id.to_owned()
        }
    };
    match (method.as_str(), parts.as_slice()) {
        ("POST", ["containers", "create"]) => match query.strip_prefix("name=") {
            Some(name) => format!("create container {}", name.split('&').next().unwrap_or("")),
            None => "create a container".into(),
        },
        ("POST", ["containers", id, action]) => format!("{action} container {}", short(id)),
        ("DELETE", ["containers", id]) => format!("remove container {}", short(id)),
        ("GET", ["containers", id, ..]) => format!("inspect container {}", short(id)),
        ("POST", ["images", .., "tag"]) => "tag an image".into(),
        ("DELETE", ["images", ..]) => "remove an image".into(),
        (_, ["images" | "distribution", ..]) => "inspect an image".into(),
        (_, ["networks", ..]) => "use a network".into(),
        (_, ["volumes", ..]) => "use a volume".into(),
        _ => "complete a request".into(),
    }
}
/// One line of an explanation, without the values of `key=value` settings or
/// query parameters (environment, API keys in URLs), URL credentials or
/// authorization tokens.
pub(crate) fn detail(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default().trim();
    let mut token = false;
    let words = line
        .split(' ')
        .map(|word| {
            let keyword = ["bearer", "basic", "token", "authorization:"]
                .contains(&word.to_ascii_lowercase().as_str());
            if std::mem::replace(&mut token, keyword) && !keyword {
                "[redacted]".to_owned()
            } else if let Some((key, _)) = word.split_once('=') {
                format!("{key}=[redacted]")
            } else if let Some((scheme, rest)) = word.split_once("://")
                && let Some((_, host)) = rest.split_once('@')
            {
                format!("{scheme}://[redacted]@{host}")
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    words
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect()
}
/// The last lines a finished worker printed, each through [`detail`].
pub(crate) async fn output_tail(container: &str) -> Vec<String> {
    #[cfg(test)]
    if crate::test_support::DOCKER.try_with(|_| ()).is_ok() {
        return Vec::new();
    }
    let read = async {
        let client = reqwest::Client::builder()
            .unix_socket("/var/run/docker.sock")
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .ok()?;
        let response = client
            .get(format!(
                "http://docker/containers/{container}/logs?stdout=true&stderr=true&tail=10"
            ))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        response.bytes().await.ok()
    };
    let Some(bytes) = read.await else {
        return Vec::new();
    };
    // Without a TTY, Docker frames each chunk with an 8-byte stream header.
    let mut text = Vec::new();
    let mut rest = &bytes[..];
    while rest.len() >= 8 && rest[0] <= 2 && rest[1..4] == [0, 0, 0] {
        let size = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let end = size.saturating_add(8).min(rest.len());
        text.extend_from_slice(&rest[8..end]);
        rest = &rest[end..];
    }
    text.extend_from_slice(rest);
    String::from_utf8_lossy(&text)
        .lines()
        .map(detail)
        .filter(|line| !line.is_empty())
        .collect()
}
/// Translate Docker's own error into a stable explanation an administrator can act on.
fn engine_failure(
    method: &reqwest::Method,
    path: &str,
    status: StatusCode,
    message: &str,
) -> Failure {
    let lower = message.to_ascii_lowercase();
    let conflict = |text: String| (StatusCode::CONFLICT, Cow::Owned(text));
    if [
        "port is already allocated",
        "address already in use",
        "only one usage of each socket address",
        "ports are not available",
    ]
    .iter()
    .any(|value| lower.contains(value))
    {
        return conflict(match published_port(message) {
            Some(port) => format!(
                "Local port {port} is already in use by another container or program; choose a different port"
            ),
            None => "The chosen local port is already in use by another container or program; choose a different port".into(),
        });
    }
    if let Some((_, source)) = message.split_once("bind source path does not exist: ") {
        return conflict(format!(
            "The host folder {} mounted by this container no longer exists",
            source.trim()
        ));
    }
    if lower.contains("no space left on device") {
        return conflict("Docker storage is full; free disk space and retry".into());
    }
    if lower.contains("is already in use by container") {
        return conflict("Another container already uses this container name".into());
    }
    if lower.contains("marked for removal") || lower.contains("removal of container") {
        return conflict("Docker is already removing this container".into());
    }
    if status == StatusCode::NOT_FOUND {
        let missing = if lower.contains("no such image")
            || path.starts_with("/images/")
            || path.starts_with("/distribution/")
        {
            "The service image is not available in Docker"
        } else if lower.contains("network") {
            "The Docker network used by this container no longer exists"
        } else {
            "Container no longer exists"
        };
        return (StatusCode::NOT_FOUND, missing.into());
    }
    // Anything else keeps Docker's reason; an engine fault may pass, a refusal won't.
    let (status, verb) = if status.is_server_error() {
        (StatusCode::SERVICE_UNAVAILABLE, "Docker failed to")
    } else {
        (StatusCode::CONFLICT, "Docker refused to")
    };
    let operation = operation(method, path);
    let detail = detail(message);
    (
        status,
        if detail.is_empty() {
            format!("{verb} {operation}")
        } else {
            format!("{verb} {operation}: {detail}")
        }
        .into(),
    )
}
/// Docker names the conflicting binding as `127.0.0.1:7878` or `[::]:7878` in free text.
fn published_port(message: &str) -> Option<u16> {
    message.match_indices(':').find_map(|(index, _)| {
        let tail = &message[index + 1..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        tail[..digits].parse::<u16>().ok().filter(|port| *port > 0)
    })
}
async fn list() -> Result<Json<Value>> {
    let data = engine("/containers/json?all=true").await?;
    let items=data.as_array().ok_or_else(unavailable)?.iter().take(1000).map(|c|json!({"id":c["Id"],"names":c["Names"],"image":c["Image"],"image_id":c["ImageID"],"state":c["State"],"ports":c["Ports"],"compose_project":c["Labels"]["com.docker.compose.project"]})).collect::<Vec<_>>();
    Ok(Json(json!({"items":items})))
}
async fn inspect(Path(id): Path<String>) -> Result<Json<Value>> {
    if !(12..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((StatusCode::BAD_REQUEST, "Use a Docker container ID".into()));
    }
    let mut value = redact(&engine(&format!("/containers/{id}/json")).await?)?;
    value["engine"] = json!(engine_identity().await?);
    Ok(Json(value))
}
/// The Docker engine serving this controller; connections and transfers are bound to it.
pub(crate) async fn engine_identity() -> Result<String> {
    static ENGINE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    ENGINE
        .get_or_try_init(|| async {
            engine("/info").await?["ID"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .ok_or_else(unavailable)
        })
        .await
        .cloned()
}
fn redact(c: &Value) -> Result<Value> {
    let mounts=c["Mounts"].as_array().ok_or_else(unavailable)?.iter().map(|m|json!({"kind":m["Type"],"name":m["Name"],"subpath":c["HostConfig"]["Mounts"].as_array().into_iter().flatten().find(|v| v["Target"]==m["Destination"]).map(|v|v["VolumeOptions"]["Subpath"].clone()).unwrap_or(Value::Null),"source":m["Source"],"destination":m["Destination"],"writable":m["RW"]})).collect::<Vec<_>>();
    let networks=c["NetworkSettings"]["Networks"].as_object().ok_or_else(unavailable)?.iter().map(|(name,n)|json!({"name":name,"id":n["NetworkID"],"address":n["IPAddress"],"ipv6":n["GlobalIPv6Address"],"aliases":n["Aliases"]})).collect::<Vec<_>>();
    let labels = &c["Config"]["Labels"];
    Ok(
        json!({"id":c["Id"],"name":c["Name"],"image":c["Config"]["Image"],"image_id":c["Image"],"running":c["State"]["Running"],"mounts":mounts,"networks":networks,"ports":published(c),"network_mode":c["HostConfig"]["NetworkMode"],"restart_policy":c["HostConfig"]["RestartPolicy"]["Name"],"compose_project":labels["com.docker.compose.project"],"compose_service":labels["com.docker.compose.service"]}),
    )
}
/// Host ports published for TCP; a stopped container reports only its configuration.
/// `loopback` says every binding of the port is on loopback, out of other containers' reach.
fn published(c: &Value) -> Vec<Value> {
    let live = &c["NetworkSettings"]["Ports"];
    let bindings = if live.as_object().is_some_and(|ports| !ports.is_empty()) {
        live
    } else {
        &c["HostConfig"]["PortBindings"]
    };
    let mut ports: Vec<Value> = Vec::new();
    for (key, hosts) in bindings.as_object().into_iter().flatten() {
        let Some((private, "tcp")) = key.split_once('/') else {
            continue;
        };
        for host in hosts.as_array().into_iter().flatten() {
            let (Ok(private), Some(Ok(public))) = (
                private.parse::<u16>(),
                host["HostPort"].as_str().map(str::parse::<u16>),
            ) else {
                continue;
            };
            // Docker reports an empty HostIp for every address.
            let loopback = host["HostIp"]
                .as_str()
                .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback());
            match ports
                .iter_mut()
                .find(|p| p["private"] == private && p["public"] == public)
            {
                Some(port) if !loopback => port["loopback"] = json!(false),
                Some(_) => {}
                None => ports.push(json!({"private":private,"public":public,"loopback":loopback})),
            }
        }
    }
    ports
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
    #[test]
    fn inspection_reports_published_tcp_ports_live_or_configured() {
        let live = json!({"NetworkSettings":{"Ports":{"6789/tcp":[{"HostIp":"0.0.0.0","HostPort":"16789"},{"HostIp":"::","HostPort":"16789"}],"7359/udp":[{"HostPort":"7359"}],"8080/tcp":null}},
            "HostConfig":{"PortBindings":{"6789/tcp":[{"HostPort":"1"}]}}});
        assert_eq!(
            published(&live),
            [json!({"private":6789,"public":16789,"loopback":false})]
        );
        let stopped = json!({"NetworkSettings":{"Ports":{}},
            "HostConfig":{"PortBindings":{"6789/tcp":[{"HostIp":"127.0.0.1","HostPort":"6789"}]}}});
        assert_eq!(
            published(&stopped),
            [json!({"private":6789,"public":6789,"loopback":true})]
        );
    }
    #[test]
    fn engine_failures_name_the_conflicting_resource() {
        let post = reqwest::Method::POST;
        let (status, message) = engine_failure(
            &post,
            "/containers/abc/start",
            StatusCode::INTERNAL_SERVER_ERROR,
            "driver failed programming external connectivity on endpoint radarr (f00): Bind for 127.0.0.1:7878 failed: port is already allocated",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(message.contains("Local port 7878 is already in use"));
        let (_, message) = engine_failure(
            &post,
            "/containers/abc/start",
            StatusCode::INTERNAL_SERVER_ERROR,
            "listen tcp6 [::]:8989: bind: address already in use",
        );
        assert!(message.contains("Local port 8989"));
        let (status, message) = engine_failure(
            &post,
            "/containers/create",
            StatusCode::BAD_REQUEST,
            "invalid mount config for type \"bind\": bind source path does not exist: /srv/radarr",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(message.contains("/srv/radarr"));
        assert_eq!(
            engine_failure(
                &reqwest::Method::GET,
                "/containers/abc/json",
                StatusCode::NOT_FOUND,
                "No such container: abc"
            ),
            (StatusCode::NOT_FOUND, "Container no longer exists".into())
        );
        assert_eq!(
            engine_failure(
                &post,
                "/containers/create",
                StatusCode::NOT_FOUND,
                "No such image: radarr"
            )
            .1,
            "The service image is not available in Docker"
        );
    }
    #[test]
    fn other_engine_failures_keep_dockers_reason_without_settings() {
        let post = reqwest::Method::POST;
        assert_eq!(
            engine_failure(
                &post,
                "/containers/create?name=thelxinoe-state-fbb830e1-preflight-snapshot",
                StatusCode::BAD_REQUEST,
                "NanoCPUs can not be set, as your kernel does not support CPU CFS scheduler or the cgroup is not mounted"
            ),
            (
                StatusCode::CONFLICT,
                "Docker refused to create container thelxinoe-state-fbb830e1-preflight-snapshot: NanoCPUs can not be set, as your kernel does not support CPU CFS scheduler or the cgroup is not mounted".into()
            )
        );
        let id = "a".repeat(64);
        let (status, message) = engine_failure(
            &post,
            &format!("/containers/{id}/start"),
            StatusCode::INTERNAL_SERVER_ERROR,
            "OCI runtime create failed: API_KEY=private-secret rejected\nsecond line",
        );
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            message,
            "Docker failed to start container aaaaaaaaaaaa: OCI runtime create failed: API_KEY=[redacted] rejected"
        );
        assert_eq!(
            engine_failure(
                &reqwest::Method::DELETE,
                "/images/x",
                StatusCode::CONFLICT,
                ""
            )
            .1,
            "Docker refused to remove an image"
        );
        assert_eq!(
            detail("GET https://admin:hunter2@radarr:7878/api failed: Authorization: Bearer abc"),
            "GET https://[redacted]@radarr:7878/api failed: Authorization: Bearer [redacted]"
        );
    }
    #[test]
    fn worker_limits_avoid_settings_engines_refuse() {
        let mut spec = json!({"HostConfig":{"NetworkMode":"none"}});
        limit(&mut spec, 1 << 30, 2, 64);
        assert_eq!(
            spec["HostConfig"],
            json!({"NetworkMode":"none","Memory":1u64 << 30,"CpuPeriod":100000,"CpuQuota":200000,"PidsLimit":64})
        );
        // Engines without CFS bandwidth control refuse NanoCpus outright.
        for source in [
            include_str!("product.rs"),
            include_str!("updates.rs"),
            include_str!("transfer.rs"),
            include_str!("service_removal.rs"),
            include_str!("recyclarr.rs"),
        ] {
            assert!(!source.contains(concat!("\"Nano", "Cpus\"")));
        }
    }
}
