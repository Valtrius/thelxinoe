//! Validate standalone adoption and compare only stable Docker configuration.
use crate::templates::Template;
use serde_json::{Value, json};
pub fn orchestrated(container: &Value) -> bool {
    container["Config"]["Labels"]
        .as_object()
        .is_some_and(|labels| {
            labels.keys().any(|key| {
                [
                    "com.docker.compose.",
                    "com.docker.swarm.",
                    "com.docker.stack.",
                    "io.kubernetes.",
                    "io.podman.compose.",
                    "com.hashicorp.nomad.",
                    "app.thelxinoe.",
                ]
                .iter()
                .any(|prefix| key.starts_with(prefix))
            })
        })
}
pub fn fingerprint(container: &Value) -> Value {
    let config = &container["Config"];
    let host = &container["HostConfig"];
    let mut configuration = serde_json::Map::new();
    for key in [
        "Image",
        "Env",
        "Cmd",
        "Entrypoint",
        "User",
        "WorkingDir",
        "Labels",
        "ExposedPorts",
        "Healthcheck",
    ] {
        configuration.insert(key.into(), config[key].clone());
    }
    let host_config = host.clone();
    let networks = container["NetworkSettings"]["Networks"].as_object().map(|n| {
        n.iter().map(|(name,v)| (name.clone(),json!({"network_id":v["NetworkID"],"aliases":v["Aliases"],"ipam":v["IPAMConfig"]}))).collect::<serde_json::Map<String,Value>>()
    }).unwrap_or_default();
    json!({"image_id":container["Image"],"config":configuration,"host":host_config,"networks":networks})
}
fn empty(value: &Value) -> bool {
    value.is_null()
        || value == ""
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}
pub fn same_default(expected: &Value, actual: &Value) -> bool {
    expected == actual || (empty(expected) && empty(actual))
}
pub fn validate_adoption(
    container: &Value,
    image: &Value,
    t: Template,
    network: &str,
    media_source: &str,
) -> Result<(), &'static str> {
    if image["Architecture"] != "amd64" || image["Os"] != "linux" {
        return Err("Adoption requires the supported Linux x86-64 image");
    }
    if orchestrated(container) {
        return Err(
            "Remove competing Compose or orchestrator ownership before adopting this container",
        );
    }
    if !image["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|d| d == format!("{}@{}", t.repository, t.digest))
    {
        return Err("Adoption requires the tested immutable image from this service template");
    }
    let host = &container["HostConfig"];
    if host["Privileged"] == true || host["ReadonlyRootfs"] == true {
        return Err("Unsupported privilege or filesystem configuration");
    }
    for key in [
        "CapAdd",
        "Devices",
        "DeviceRequests",
        "ExtraHosts",
        "Dns",
        "Sysctls",
        "GroupAdd",
        "Tmpfs",
        "PidMode",
        "UsernsMode",
        "UTSMode",
    ] {
        if !empty(&host[key]) {
            return Err("Unsupported host access in the existing container");
        }
    }
    if !matches!(host["IpcMode"].as_str(), None | Some("") | Some("private")) {
        return Err("Shared IPC is not supported");
    }
    if host["SecurityOpt"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|value| value != "no-new-privileges:true" && value != "no-new-privileges")
    {
        return Err("Custom security profiles are not supported for adoption");
    }
    let expected_hostname = container["Id"].as_str().and_then(|id| id.get(..12));
    if container["Config"]["Hostname"].as_str() != expected_hostname {
        return Err("Custom container hostnames are not supported for adoption");
    }
    let networks = container["NetworkSettings"]["Networks"]
        .as_object()
        .ok_or("Missing network evidence")?;
    if networks.len() != 1 || !networks.contains_key(network) {
        return Err("Adoption requires only the Thelxinoe media network");
    }
    let config = &container["Config"];
    for key in ["Cmd", "Entrypoint", "User", "WorkingDir", "Healthcheck"] {
        if !same_default(&config[key], &image["Config"][key]) {
            return Err("Custom commands, users or health checks are not supported");
        }
    }
    let defaults = image["Config"]["Env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for value in config["Env"]
        .as_array()
        .ok_or("Missing environment evidence")?
    {
        let value = value.as_str().ok_or("Invalid environment entry")?;
        if defaults.iter().any(|d| d.as_str() == Some(value)) {
            continue;
        }
        let (key, value) = value.split_once('=').ok_or("Invalid environment entry")?;
        if !matches!(key, "PUID" | "PGID" | "TZ" | "UMASK")
            || value.len() > 100
            || value.contains(['\n', '\r'])
        {
            return Err("Unsupported application environment overrides");
        }
        if matches!(key, "PUID" | "PGID") && value != "10001" {
            return Err("Adoption requires PUID and PGID 10001");
        }
    }
    let mounts = container["Mounts"]
        .as_array()
        .ok_or("Missing mount evidence")?;
    let expected = if t.media { 2 } else { 1 };
    if mounts.len() != expected {
        return Err("Adoption supports only /config and the canonical /data media mount");
    }
    for mount in mounts {
        if mount["Type"] != "bind" || mount["RW"] != true {
            return Err("Adoption requires writable bind mounts");
        }
        match mount["Destination"].as_str() {
            Some("/data") if t.media && mount["Source"] == media_source => {}
            Some("/config")
                if mount["Source"]
                    .as_str()
                    .is_some_and(|source| appdata_isolated(source, media_source)) => {}
            _ => return Err("Media mount does not match the canonical server media tree"),
        }
    }
    Ok(())
}
pub fn appdata_isolated(source: &str, media: &str) -> bool {
    use std::path::Path;
    let source = Path::new(source);
    let media = Path::new(media);
    source.is_absolute()
        && source.components().count() > 2
        && !source.starts_with(media)
        && !media.starts_with(source)
        && ![
            "/etc",
            "/proc",
            "/sys",
            "/dev",
            "/boot",
            "/bin",
            "/sbin",
            "/usr",
            "/run/docker",
            "/run/containerd",
            "/var/run",
            "/var/lib/docker",
        ]
        .iter()
        .any(|root| source.starts_with(root))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appdata_restore_cannot_target_system_or_media_roots() {
        assert!(!appdata_isolated("/", "/media"));
        assert!(!appdata_isolated("/etc/service", "/media"));
        assert!(!appdata_isolated("/media/appdata", "/media"));
        assert!(!appdata_isolated("/srv", "/srv/media"));
        assert!(appdata_isolated("/srv/appdata/radarr", "/srv/media"));
    }
    #[test]
    fn foreign_orchestrator_ownership_is_always_rejected() {
        for prefix in [
            "com.docker.compose.project",
            "com.docker.swarm.service.id",
            "io.kubernetes.pod.name",
            "io.podman.compose.project",
            "com.hashicorp.nomad.job_name",
            "app.thelxinoe.managed-id",
        ] {
            let c = json!({"Config":{"Labels":{prefix:"foreign"}}});
            assert!(orchestrated(&c));
        }
        assert!(!orchestrated(
            &json!({"Config":{"Labels":{"maintainer":"upstream"}}})
        ));
    }
    #[test]
    fn drift_includes_dangerous_configuration_but_ignores_runtime_addresses() {
        let mut c = json!({"Image":"sha256:a","Config":{"Env":["PUID=10001"]},"HostConfig":{"Privileged":false},"NetworkSettings":{"Networks":{"media":{"NetworkID":"n","IPAddress":"172.20.0.2"}}}});
        let before = fingerprint(&c);
        c["NetworkSettings"]["Networks"]["media"]["IPAddress"] = json!("172.20.0.3");
        assert_eq!(before, fingerprint(&c));
        c["HostConfig"]["Privileged"] = json!(true);
        assert_ne!(before, fingerprint(&c));
    }
}
