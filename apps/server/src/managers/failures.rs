//! Explain service and Docker failures in terms an administrator can act on.
use super::*;
use axum::http::StatusCode;

pub(super) fn label(kind: &str) -> &'static str {
    match kind {
        "radarr" => "Radarr",
        "sonarr" => "Sonarr",
        "lidarr" => "Lidarr",
        "prowlarr" => "Prowlarr",
        "bazarr" => "Bazarr",
        "nzbget" => "NZBGet",
        "seerr" => "Seerr",
        _ => "The service",
    }
}
fn default_port(kind: &str) -> Option<u16> {
    Some(match kind {
        "radarr" => 7878,
        "sonarr" => 8989,
        "lidarr" => 8686,
        "prowlarr" => 9696,
        "bazarr" => 6767,
        "nzbget" => 6789,
        "seerr" => 5055,
        _ => return None,
    })
}
pub(super) fn container_missing() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "container_missing",
        "The selected container no longer exists".into(),
    )
}
pub(super) fn controller_unreachable() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "dependency_unavailable",
        "Thelxinoe can't reach its Docker controller; check that the controller container is running"
            .into(),
    )
}
/// A connection that is otherwise valid, but cannot be verified until its container runs.
pub(super) fn stopped(name: &str) -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "service_stopped",
        format!(
            "The {name} container is stopped, so Thelxinoe can't verify the connection. Start it and try again, or continue without checking."
        ),
    )
}
/// A service API request that never produced an HTTP response.
pub(super) fn unreachable(kind: &str, base: &str, error: &reqwest::Error) -> ApiError {
    let service = label(kind);
    let target = base.trim_start_matches("http://");
    let host = target.rsplit_once(':').map_or(target, |(host, _)| host);
    let cause = std::iter::successors(Some(error as &dyn std::error::Error), |e| e.source())
        .map(|e| e.to_string().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(": ");
    let message = if error.is_timeout() {
        format!(
            "{service} didn't respond at {target} within 20 seconds; it may be busy or still starting"
        )
    } else if [
        "dns error",
        "failed to lookup address",
        "name or service not known",
    ]
    .iter()
    .any(|text| cause.contains(text))
    {
        format!(
            "Thelxinoe can't resolve {host} on the shared Docker network. Docker's default bridge network doesn't resolve container names; connect both containers to a user-defined network"
        )
    } else if error.is_connect() {
        match default_port(kind) {
            Some(port) => format!(
                "Nothing is answering at {target}. Check the internal port ({service} uses {port} by default) and that {service} has finished starting"
            ),
            None => format!(
                "Nothing is answering at {target}. Check the internal port and that the service has finished starting"
            ),
        }
    } else {
        format!("The connection to {service} at {target} failed before it answered")
    };
    ApiError(StatusCode::CONFLICT, "dependency_unavailable", message)
}
/// A service answered, but refused a request sent to `endpoint`.
pub(super) fn rejected(kind: &str, endpoint: &str, status: reqwest::StatusCode) -> ApiError {
    let service = label(kind);
    let code = status.as_u16();
    ApiError::conflict(match code {
        401 | 403 if kind == "nzbget" => format!(
            "NZBGet rejected the username or password (HTTP {code}). Use the control username and password from NZBGet's Settings → Security"
        ),
        401 | 403 => format!(
            "{service} rejected the API key (HTTP {code}). Copy the current key from {service}'s Settings → General"
        ),
        404 if endpoint.ends_with("/status") => {
            if access::supported(kind) && kind != "nzbget" {
                format!(
                    "No {service} API answered at {endpoint}. Check that this is the {service} container and that the URL Base matches {service}'s Settings → General → URL Base"
                )
            } else {
                format!(
                    "No {service} API answered at {endpoint}. Check that this is the {service} container and its internal port"
                )
            }
        }
        _ => format!("{service} rejected the request (HTTP {code})"),
    })
}
/// The container answered a status probe as a different application.
pub(super) fn identity(kind: &str, reported: &Value) -> ApiError {
    let service = label(kind);
    ApiError::conflict(
        match reported
            .as_str()
            .filter(|app| !app.is_empty() && app.len() <= 40 && !app.contains(char::is_control))
        {
            Some(app) => format!("The selected container answered as {app}, not {service}"),
            None => format!("The selected container didn't answer like {service}"),
        },
    )
}
fn network_names(observed: &Value) -> Vec<String> {
    observed["networks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["name"].as_str().map(str::to_owned))
        .collect()
}
fn user_network(server: &Value) -> Option<String> {
    network_names(server)
        .into_iter()
        .find(|name| !matches!(name.as_str(), "bridge" | "host" | "none"))
}
fn connect_hint(server: &Value, name: &str) -> String {
    match user_network(server) {
        Some(network) => format!("Connect it with: docker network connect {network} {name}"),
        None => "Put both containers on the same user-defined Docker network".into(),
    }
}
/// The service shares no Docker network with Thelxinoe.
pub(super) async fn isolated(
    state: &AppState,
    server: &Value,
    service: &Value,
    name: &str,
) -> ApiError {
    let mode = service["network_mode"].as_str().unwrap_or_default();
    if let Some(peer) = mode.strip_prefix("container:") {
        let resolved =
            if (12..=64).contains(&peer.len()) && peer.bytes().all(|b| b.is_ascii_hexdigit()) {
                docker(state, &format!("containers/{peer}"))
                    .await
                    .ok()
                    .and_then(|o| {
                        o["name"]
                            .as_str()
                            .map(|n| n.trim_start_matches('/').to_owned())
                    })
            } else {
                None
            };
        let peer = resolved.unwrap_or_else(|| peer.chars().take(12).collect());
        return ApiError::conflict(format!(
            "{name} uses the network of {peer} (as VPN setups do). Thelxinoe can't connect to services behind another container's network yet"
        ));
    }
    if mode == "host" {
        return ApiError::conflict(format!(
            "{name} uses host networking, which Thelxinoe can't reach through Docker. {}",
            connect_hint(server, name)
        ));
    }
    let theirs = network_names(service);
    ApiError::conflict(format!(
        "{name} isn't on a Docker network shared with Thelxinoe (Thelxinoe uses {}; {name} uses {}). {}",
        network_names(server).join(", "),
        if theirs.is_empty() {
            "none".into()
        } else {
            theirs.join(", ")
        },
        connect_hint(server, name)
    ))
}
/// Docker's default bridge network routes by address only, never by container name.
pub(super) fn default_bridge(server: &Value, name: &str) -> ApiError {
    ApiError::conflict(format!(
        "{name} only shares Docker's default bridge network with Thelxinoe, which can't resolve container names. {}",
        connect_hint(server, name)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn network_failures_name_both_sides_and_a_fix() {
        let (_temp, state, _) = crate::test_support::fixture().await;
        let server = json!({"networks":[{"name":"bridge"},{"name":"thelxinoe_media"}]});
        let message = default_bridge(&server, "radarr").2;
        assert!(message.contains("docker network connect thelxinoe_media radarr"));
        let service = json!({"networks":[{"name":"arr_default"}]});
        let message = isolated(&state, &server, &service, "radarr").await.2;
        assert!(
            message.contains("Thelxinoe uses bridge, thelxinoe_media; radarr uses arr_default")
        );
        assert!(message.contains("docker network connect thelxinoe_media radarr"));
        let service = json!({"network_mode":"container:gluetun","networks":[]});
        let message = isolated(&state, &server, &service, "nzbget").await.2;
        assert!(message.contains("nzbget uses the network of gluetun"));
        let service = json!({"network_mode":"host","networks":[{"name":"host"}]});
        let message = isolated(&state, &server, &service, "sonarr").await.2;
        assert!(message.contains("sonarr uses host networking"));
    }

    #[test]
    fn responses_explain_keys_url_bases_and_identities() {
        let unauthorized = rejected(
            "radarr",
            "/api/v3/system/status",
            reqwest::StatusCode::UNAUTHORIZED,
        );
        assert!(unauthorized.2.contains("Radarr rejected the API key"));
        assert!(
            rejected(
                "sonarr",
                "/sonarr/api/v3/system/status",
                reqwest::StatusCode::NOT_FOUND
            )
            .2
            .contains("URL Base")
        );
        assert!(
            rejected("seerr", "/api/v1/status", reqwest::StatusCode::NOT_FOUND)
                .2
                .contains("internal port")
        );
        assert_eq!(
            rejected("radarr", "/api/v3/movie/4", reqwest::StatusCode::NOT_FOUND).2,
            "Radarr rejected the request (HTTP 404)"
        );
        assert!(
            rejected("nzbget", "/jsonrpc", reqwest::StatusCode::UNAUTHORIZED)
                .2
                .contains("username or password")
        );
        assert_eq!(
            identity("prowlarr", &json!("Radarr")).2,
            "The selected container answered as Radarr, not Prowlarr"
        );
        assert_eq!(
            identity("bazarr", &Value::Null).2,
            "The selected container didn't answer like Bazarr"
        );
    }
}
