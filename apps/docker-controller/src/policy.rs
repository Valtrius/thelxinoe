//! Validate service ownership transfers and compare stable Docker configuration.
use crate::templates::Template;
use serde_json::{Value, json};
use std::borrow::Cow;
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
        "Hostname",
        "Domainname",
        "StopSignal",
        "StopTimeout",
        "Tty",
        "OpenStdin",
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

/// Docker leaves network IDs unresolved until a new stopped container first
/// starts. Accept only that initialization during a journaled lifecycle action.
pub fn first_start_matches(expected: &Value, actual: &Value) -> bool {
    let mut normalized = expected.clone();
    let mut initialized = false;
    if let Some(networks) = normalized["networks"].as_object_mut() {
        for (name, network) in networks {
            if network["network_id"] == ""
                && actual["networks"][name]["network_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
            {
                network["network_id"] = actual["networks"][name]["network_id"].clone();
                initialized = true;
            }
        }
    }
    // Docker Desktop normalizes its default OOM setting on the same transition.
    if initialized
        && normalized["host"]["OomKillDisable"] == false
        && actual["host"]["OomKillDisable"].is_null()
    {
        normalized["host"]["OomKillDisable"] = Value::Null;
    }
    initialized && normalized == *actual
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
pub fn server_media_source(container: &Value) -> Result<&str, &'static str> {
    let mounts = container["Mounts"]
        .as_array()
        .ok_or("Missing media mount evidence")?;
    let roots = mounts
        .iter()
        .filter(|m| m["Destination"] == "/media")
        .collect::<Vec<_>>();
    if roots.len() != 1 || roots[0]["Type"] != "bind" || roots[0]["RW"] != true {
        return Err("Media services require one writable bind mount at /media");
    }
    roots[0]["Source"]
        .as_str()
        .filter(|s| host_path(s).is_some())
        .ok_or("Invalid media mount source")
}
/// Fresh templates share one media root with no nested overrides.
pub fn media_source(container: &Value) -> Result<&str, &'static str> {
    let source = server_media_source(container)?;
    if container["Mounts"].as_array().is_some_and(|mounts| {
        mounts.iter().any(|m| {
            m["Destination"]
                .as_str()
                .is_some_and(|p| p.starts_with("/media/"))
        })
    }) {
        return Err(
            "Fresh installations require one shared writable mount at /media; additional media mounts can be used with connected or adopted services",
        );
    }
    Ok(source)
}
/// Docker Desktop reports Windows sources case-insensitively.
fn normalized(path: &str) -> Option<std::path::PathBuf> {
    host_path(path).map(|p| {
        if p.starts_with("/run/desktop/mnt/host") {
            std::path::PathBuf::from(p.to_string_lossy().to_ascii_lowercase())
        } else {
            p
        }
    })
}
/// Trusted deployment volumes may live under Docker's state directory.
/// This check only excludes media overlap; adoption uses the stricter policy.
pub fn media_disjoint(state: &str, media: &str) -> bool {
    match (normalized(state), normalized(media)) {
        (Some(state), Some(media)) => !state.starts_with(&media) && !media.starts_with(&state),
        _ => false,
    }
}
/// Both paths are valid host paths and one contains the other.
pub fn overlapping(first: &str, second: &str) -> bool {
    normalized(first)
        .zip(normalized(second))
        .is_some_and(|(first, second)| first.starts_with(&second) || second.starts_with(&first))
}
/// Read-only host time zone data is common in existing deployments and grants no write access.
pub fn host_time_zone(source: &str) -> bool {
    matches!(source, "/etc/localtime" | "/etc/timezone")
        || std::path::Path::new(source).starts_with("/usr/share/zoneinfo")
}
/// Why a host folder other than `/config` can't stay mounted in an adopted service.
pub fn foreign_source_problem(source: &str, deployment: &str) -> Option<&'static str> {
    let Some(path) = normalized(source) else {
        return Some("uses an unsupported host path");
    };
    if path.parent().is_none() {
        return Some("uses a host system folder");
    }
    // Deployment volumes usually live under Docker's own state directory.
    if overlapping(source, deployment) {
        return Some("overlaps Thelxinoe's own deployment storage");
    }
    system_path(&path).then_some("uses a host system folder")
}
/// Too broad to belong to one service: the filesystem root or a top-level folder.
fn shallow(path: &std::path::Path) -> bool {
    match path.strip_prefix("/run/desktop/mnt/host") {
        Ok(tail) => tail.components().count() < 3,
        Err(_) => path.components().count() <= 2,
    }
}
/// Host operating system, Docker engine and Windows program folders.
fn system_path(path: &std::path::Path) -> bool {
    if let Ok(tail) = path.strip_prefix("/run/desktop/mnt/host") {
        return matches!(
            tail.components()
                .nth(1)
                .and_then(|v| v.as_os_str().to_str()),
            Some("windows" | "program files" | "program files (x86)" | "programdata")
        );
    }
    [
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
    .any(|root| path.starts_with(root))
}
pub fn validate_adoption(
    container: &Value,
    image: &Value,
    t: Template,
    network: &str,
    media_source: &str,
    released_compose: bool,
) -> Result<(), &'static str> {
    if image["Architecture"] != "amd64" || image["Os"] != "linux" {
        return Err("Adoption requires the supported Linux x86-64 image");
    }
    transfer_owner(container, released_compose)?;
    // Callers report `adoption_image` first, with the offending image named.
    adoption_image(container, image, t)
        .map_err(|_| "Transfer requires the service's official image on a stable tag")?;
    let host = &container["HostConfig"];
    if host["AutoRemove"] == true {
        return Err("Auto-remove containers cannot be retained for ownership transfer");
    }
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
        if t.kind == "recyclarr" && key == "User" {
            crate::identity::Identity::from_user(config)
                .map_err(|_| "Recyclarr requires a numeric non-root user")?;
            continue;
        }
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
        if !(matches!(key, "PUID" | "PGID" | "TZ" | "UMASK")
            || (t.kind == "recyclarr" && key == "CRON_SCHEDULE"))
            || value.len() > 100
            || value.contains(['\n', '\r'])
        {
            return Err("Unsupported application environment overrides");
        }
        if matches!(key, "PUID" | "PGID")
            && !value
                .parse::<u32>()
                .is_ok_and(|id| id > 0 && id < i32::MAX as u32)
        {
            return Err("Adoption requires non-root numeric PUID and PGID values");
        }
    }
    let mounts = container["Mounts"]
        .as_array()
        .ok_or("Missing mount evidence")?;
    if mounts.len() != if t.media { 2 } else { 1 } {
        return Err("Adoption supports only /config and the shared /media bind mount");
    }
    for mount in mounts {
        if mount["Type"] != "bind" || mount["RW"] != true {
            return Err("Adoption requires writable bind mounts");
        }
        match mount["Destination"].as_str() {
            Some("/media") if t.media && mount["Source"] == media_source => {}
            Some("/config")
                if mount["Source"]
                    .as_str()
                    .is_some_and(|source| appdata_isolated(source, media_source)) => {}
            _ => {
                return Err(
                    "Media mount must use the same host source and /media path as Thelxinoe",
                );
            }
        }
    }
    Ok(())
}
pub fn transfer_owner(container: &Value, released_compose: bool) -> Result<(), &'static str> {
    let mut standalone = container.clone();
    if let Some(labels) = standalone["Config"]["Labels"].as_object_mut() {
        let compose = labels
            .keys()
            .any(|key| key.starts_with("com.docker.compose."));
        if compose && !released_compose {
            return Err(
                "Disable the service in its old Compose project and confirm the ownership transfer",
            );
        }
        labels.retain(|key, _| !key.starts_with("com.docker.compose."));
    }
    if orchestrated(&standalone) {
        return Err("Services managed by another controller or cluster cannot be transferred");
    }
    Ok(())
}
/// Keep the exact installed stable upstream image: ownership transfer is not an upgrade.
pub fn adoption_image(
    container: &Value,
    image: &Value,
    t: Template,
) -> Result<String, Cow<'static, str>> {
    let allowed = crate::templates::sources(t);
    let reference = container["Config"]["Image"].as_str().unwrap_or("");
    let (repository, version) = reference
        .split_once('@')
        .or_else(|| {
            reference
                .rsplit_once(':')
                .filter(|(_, tag)| !tag.contains('/'))
        })
        .unwrap_or((reference, "latest"));
    // Versions may be written 3.5.0 or v3.5.0; branch and commit builds never qualify.
    let number = version.strip_prefix('v').unwrap_or(version);
    let stable = version == "latest"
        || version.starts_with("sha256:")
        || (number.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && !["develop", "nightly", "beta", "alpha", "preview", "rc"]
                .iter()
                .any(|label| version.to_ascii_lowercase().contains(label)));
    if !allowed.iter().any(|item| item == repository) {
        return Err(format!(
            "Ownership requires the official image from {}; this container uses {repository}",
            allowed.join(", ")
        )
        .into());
    }
    if !stable {
        return Err(format!(
            "Ownership requires a stable image tag (latest, a version such as 3.5.0 or v3.5.0, or a digest); this container uses {version}"
        )
        .into());
    }
    image["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|reference| {
            reference
                .split_once('@')
                .is_some_and(|(repository, digest)| {
                    allowed.iter().any(|item| item == repository)
                        && digest.starts_with("sha256:")
                        && digest.len() == 71
                        && digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
                })
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            "The installed service image must have a verified upstream repository digest".into()
        })
}
/// Service state needs its own folder: never the host system, a top-level folder or `media`.
pub fn appdata_isolated(source: &str, media: &str) -> bool {
    let (Some(source), Some(media)) = (normalized(source), normalized(media)) else {
        return false;
    };
    !shallow(&source)
        && !system_path(&source)
        && !source.starts_with(&media)
        && !media.starts_with(&source)
}
/// Docker Desktop may report a Windows bind source in either host or VM notation.
pub fn host_path(value: &str) -> Option<std::path::PathBuf> {
    let bytes = value.as_bytes();
    let value = if bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        format!(
            "/run/desktop/mnt/host/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
            value[3..].replace('\\', "/")
        )
    } else {
        value.into()
    };
    let path = std::path::PathBuf::from(value);
    (path.is_absolute()
        && path.components().all(|c| {
            matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        }))
    .then_some(path)
}
#[cfg(test)]
mod tests {
    #[test]
    fn shared_media_mount_rejects_overrides_and_preserves_state_isolation() {
        let container = serde_json::json!({"Mounts":[{"Type":"bind","Source":"/nas/media","Destination":"/media","RW":true}]});
        assert_eq!(super::media_source(&container).unwrap(), "/nas/media");
        for (field, value) in [
            ("Destination", serde_json::json!("/movies")),
            ("Type", serde_json::json!("volume")),
            ("RW", serde_json::json!(false)),
        ] {
            let mut invalid = container.clone();
            invalid["Mounts"][0][field] = value;
            assert!(super::media_source(&invalid).is_err());
        }
        let mut invalid = container.clone();
        invalid["Mounts"].as_array_mut().unwrap().push(serde_json::json!({"Type":"bind","Source":"/nas/films","Destination":"/media/movies","RW":true}));
        assert!(super::media_source(&invalid).is_err());
        assert!(super::media_disjoint(
            "/var/lib/docker/volumes/deployment/_data",
            "/nas/media"
        ));
        assert!(!super::media_disjoint("/nas/media/config", "/nas/media"));
        assert!(!super::media_disjoint("/nas", "/nas/media"));
    }

    use super::*;
    #[test]
    fn appdata_restore_cannot_target_system_or_media_roots() {
        assert!(!appdata_isolated("/", "/media"));
        assert!(!appdata_isolated("/etc/service", "/media"));
        assert!(!appdata_isolated("/media/appdata", "/media"));
        assert!(!appdata_isolated("/srv", "/srv/media"));
        assert!(appdata_isolated("/srv/appdata/radarr", "/srv/media"));
        assert!(!appdata_isolated(
            "/run/desktop/mnt/host/c/Users/Jake/Data/Appdata",
            r"C:\Users\jake\data"
        ));
        assert!(!appdata_isolated(
            r"C:\Windows\System32",
            r"C:\Media\Movies"
        ));
        assert!(appdata_isolated(
            r"C:\Users\Jake\appdata\server",
            r"C:\Users\Jake\media"
        ));
        assert!(!appdata_isolated("/srv/state/../../etc/service", "/media"));
    }
    #[test]
    fn existing_media_and_time_zone_mounts_stay_adoptable_without_host_system_access() {
        let deployment = "/var/lib/docker/volumes/thelxinoe_deployment/_data";
        for source in ["/data", "/media", "/mnt/user/data", r"C:\Media"] {
            assert_eq!(foreign_source_problem(source, deployment), None, "{source}");
        }
        for source in [
            "/",
            "/etc",
            "/etc/ssl",
            "/var/run/docker.sock",
            r"C:\Windows\Fonts",
        ] {
            assert_eq!(
                foreign_source_problem(source, deployment),
                Some("uses a host system folder"),
                "{source}"
            );
        }
        assert_eq!(
            foreign_source_problem(&format!("{deployment}/services"), deployment),
            Some("overlaps Thelxinoe's own deployment storage")
        );
        assert!(host_time_zone("/etc/localtime"));
        assert!(host_time_zone("/usr/share/zoneinfo/Europe/Paris"));
        assert!(!host_time_zone("/etc/passwd"));
        assert!(overlapping("/data", "/data/media"));
        assert!(!overlapping("/data", "/srv/media"));
        assert!(!overlapping("relative", "/data"));
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
    fn compose_transfer_requires_release_and_never_accepts_a_cluster_owner() {
        let mut c = json!({"Config":{"Labels":{"com.docker.compose.project":"nas","com.docker.compose.service":"radarr"}}});
        assert!(transfer_owner(&c, false).is_err());
        assert!(transfer_owner(&c, true).is_ok());
        for owner in [
            "com.docker.swarm.service.id",
            "io.kubernetes.pod.name",
            "app.thelxinoe.managed-id",
        ] {
            c["Config"]["Labels"][owner] = json!("other");
            assert!(transfer_owner(&c, true).is_err());
            c["Config"]["Labels"].as_object_mut().unwrap().remove(owner);
        }
    }
    #[test]
    fn ownership_transfer_pins_the_installed_image_and_rejects_untrusted_or_unstable_sources() {
        let template = crate::templates::find("radarr").unwrap();
        let digest = format!("lscr.io/linuxserver/radarr@sha256:{}", "a".repeat(64));
        let image = json!({"RepoDigests":[digest]});
        for reference in [
            "lscr.io/linuxserver/radarr:latest",
            "linuxserver/radarr:6.1.0",
            digest.as_str(),
        ] {
            let container = json!({"Config":{"Image":reference}});
            assert_eq!(
                adoption_image(&container, &image, template).unwrap(),
                digest
            );
        }
        for reference in [
            "untrusted/radarr:latest",
            "linuxserver/radarr:develop",
            "linuxserver/radarr:6.2.0-beta",
        ] {
            assert!(
                adoption_image(&json!({"Config":{"Image":reference}}), &image, template).is_err()
            );
        }
        assert!(
            adoption_image(
                &json!({"Config":{"Image":"linuxserver/radarr:latest"}}),
                &json!({"RepoDigests":[]}),
                template
            )
            .is_err()
        );
    }
    #[test]
    fn seerr_ownership_accepts_its_official_registries_and_release_tags() {
        let template = crate::templates::find("seerr").unwrap();
        let digest = format!("sha256:{}", "b".repeat(64));
        for (reference, published) in [
            (
                "ghcr.io/seerr-team/seerr:v3.5.0",
                "ghcr.io/seerr-team/seerr",
            ),
            ("ghcr.io/seerr-team/seerr:v3", "ghcr.io/seerr-team/seerr"),
            ("seerr/seerr:latest", "seerr/seerr"),
            ("docker.io/seerr/seerr:3.5.0", "seerr/seerr"),
        ] {
            let pinned = format!("{published}@{digest}");
            assert_eq!(
                adoption_image(
                    &json!({"Config":{"Image":reference}}),
                    &json!({"RepoDigests":[pinned]}),
                    template
                )
                .unwrap(),
                pinned,
                "{reference}"
            );
        }
        let image = json!({"RepoDigests":[format!("ghcr.io/seerr-team/seerr@{digest}")]});
        for reference in [
            "ghcr.io/seerr-team/seerr:develop",
            "ghcr.io/seerr-team/seerr:sha-939000f",
            "ghcr.io/seerr-team/seerr:preview-music-support",
        ] {
            let error = adoption_image(&json!({"Config":{"Image":reference}}), &image, template)
                .unwrap_err();
            assert!(error.contains("stable image tag"), "{reference}: {error}");
        }
        let error = adoption_image(
            &json!({"Config":{"Image":"fallenbagel/jellyseerr:latest"}}),
            &image,
            template,
        )
        .unwrap_err();
        assert!(error.contains("this container uses fallenbagel/jellyseerr"));
        assert!(error.contains("seerr/seerr"));
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

    #[test]
    fn first_start_accepts_only_deferred_network_identity_and_oom_defaults() {
        let before = json!({"config":{"Env":["PUID=10001"]},"host":{"OomKillDisable":false,"Privileged":false},"networks":{"media":{"network_id":"","aliases":["radarr"],"ipam":null}}});
        let mut after = before.clone();
        after["networks"]["media"]["network_id"] = json!("network-id");
        after["host"]["OomKillDisable"] = Value::Null;
        assert!(first_start_matches(&before, &after));
        let initialized = after.clone();
        after["networks"]["media"]["network_id"] = json!("different-network");
        assert!(!first_start_matches(&initialized, &after));
        after["host"]["Privileged"] = json!(true);
        assert!(!first_start_matches(&before, &after));
        after = initialized.clone();
        after["config"]["Env"] = json!(["PUID=0"]);
        assert!(!first_start_matches(&before, &after));
        after = initialized;
        after["networks"]["media"]["aliases"] = json!(["changed"]);
        assert!(!first_start_matches(&before, &after));
    }
}
