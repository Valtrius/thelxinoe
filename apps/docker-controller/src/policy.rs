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
    let id = container["Id"].as_str();
    let networks = container["NetworkSettings"]["Networks"].as_object().map(|n| {
        n.iter().map(|(name,v)| (name.clone(),json!({"network_id":v["NetworkID"],"aliases":aliases(&v["Aliases"],id),"ipam":v["IPAMConfig"]}))).collect::<serde_json::Map<String,Value>>()
    }).unwrap_or_default();
    json!({"image_id":container["Image"],"config":configuration,"host":host_config,"networks":networks})
}
/// Docker Engine before 26 adds a container's short ID to its network aliases
/// when it first starts; that alias is not part of the configuration.
pub fn aliases(aliases: &Value, container: Option<&str>) -> Value {
    let Some(list) = aliases.as_array() else {
        return aliases.clone();
    };
    let own = container.map(|id| id.get(..12).unwrap_or(id));
    let kept = list
        .iter()
        .filter(|alias| alias.as_str() != own)
        .cloned()
        .collect::<Vec<_>>();
    if kept.is_empty() {
        Value::Null
    } else {
        Value::Array(kept)
    }
}
/// Whether `container` still has the configuration fingerprinted in `expected`.
pub fn matches(expected: &Value, container: &Value) -> bool {
    same(
        &recorded(expected, container["Id"].as_str()),
        &fingerprint(container),
    )
}
/// A stored fingerprint of `container` in today's form: records taken after a
/// first start on Docker before 26 hold the short ID alias.
pub fn recorded(expected: &Value, container: Option<&str>) -> Value {
    let mut expected = expected.clone();
    if let Some(networks) = expected["networks"].as_object_mut() {
        for network in networks.values_mut() {
            network["aliases"] = aliases(&network["aliases"], container);
        }
    }
    expected
}
/// A newer Docker Engine reports more host settings, and unset ones as empty or
/// null; compare the recorded settings only. An omitted OOM option reads as
/// either null or false across create, start and recreate; both keep the OOM
/// killer enabled.
fn same(expected: &Value, actual: &Value) -> bool {
    let mut expected = expected.clone();
    let mut actual = actual.clone();
    if let (Some(recorded), Some(current)) =
        (expected["host"].as_object(), actual["host"].as_object_mut())
    {
        current.retain(|key, _| recorded.contains_key(key));
        for (key, value) in recorded {
            if same_default(value, current.get(key).unwrap_or(&Value::Null)) {
                current.insert(key.clone(), value.clone());
            }
        }
    }
    for value in [&mut expected, &mut actual] {
        if value["host"].is_object() && value["host"]["OomKillDisable"].is_null() {
            value["host"]["OomKillDisable"] = json!(false);
        }
    }
    expected == actual
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
    initialized && same(&normalized, actual)
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
            "Fresh installations require one shared writable mount at /media; additional media mounts can be used by Connected and transferred services",
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
/// This check only excludes media overlap; transfers use the stricter policy.
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
/// Why a host folder other than `/config` can't stay mounted in a transferred service.
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
/// The source's exact stable upstream image; qualification decides whether a transfer keeps it.
pub fn source_image(
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
/// Fields of the configuration as `docker inspect` names them, for review.
fn inspect_path(path: &str) -> String {
    let (head, tail) = path.split_once('.').unwrap_or((path, ""));
    let head = match head {
        "config" => "Config",
        "host" => "HostConfig",
        "networks" => "NetworkSettings.Networks",
        "image_id" => "Image",
        other => other,
    };
    if tail.is_empty() {
        head.into()
    } else {
        format!("{head}.{tail}")
    }
}
/// Differences from the accepted configuration. Environment and label values can hold
/// secrets, so reviews name those entries without showing their values.
pub fn changes(before: &Value, after: &Value, path: &str, out: &mut Vec<Value>) {
    if before == after || out.len() >= 100 {
        return;
    }
    if path == "config.Env" {
        let entries = |value: &Value| {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|entry| {
                    let (key, value) = entry.split_once('=').unwrap_or((entry, ""));
                    (key.to_owned(), value.to_owned())
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let (before, after) = (entries(before), entries(after));
        let keys: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
        for key in keys {
            let change = match (before.get(key), after.get(key)) {
                (Some(old), Some(new)) if old == new => continue,
                (Some(_), Some(_)) => "changed",
                (Some(_), None) => "removed",
                _ => "added",
            };
            out.push(json!({"path":inspect_path(&format!("{path}.{key}")),"change":change}));
        }
        return;
    }
    if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
        let keys: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
        for key in keys {
            let child = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let missing = Value::Null;
            changes(
                before.get(key).unwrap_or(&missing),
                after.get(key).unwrap_or(&missing),
                &child,
                out,
            );
        }
        return;
    }
    // Docker can list the same mounts or options in a different order.
    fn sorted(value: &Value) -> Option<Vec<&str>> {
        let mut texts = value
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        texts.sort_unstable();
        Some(texts)
    }
    if sorted(before).is_some() && sorted(before) == sorted(after) {
        return;
    }
    let hidden = path.starts_with("config.Labels.");
    let shown = |value: &Value| {
        (!hidden && !value.is_null()).then(|| {
            let text = value.to_string();
            if text.chars().count() > 200 {
                format!("{}…", text.chars().take(200).collect::<String>())
            } else {
                text
            }
        })
    };
    let change = if before.is_null() {
        "added"
    } else if after.is_null() {
        "removed"
    } else {
        "changed"
    };
    out.push(json!({"path":inspect_path(path),"change":change,
        "before":shown(before),"after":shown(after)}));
}
#[cfg(test)]
mod tests {
    #[test]
    fn configuration_reviews_ignore_reordered_lists() {
        let mut found = Vec::new();
        super::changes(
            &serde_json::json!({"host":{"Binds":["/a:/a","/b:/b"]}}),
            &serde_json::json!({"host":{"Binds":["/b:/b","/a:/a"]}}),
            "",
            &mut found,
        );
        assert!(found.is_empty());
        super::changes(
            &serde_json::json!({"host":{"Binds":["/a:/a"]}}),
            &serde_json::json!({"host":{"Binds":["/c:/c"]}}),
            "",
            &mut found,
        );
        assert_eq!(found[0]["path"], "HostConfig.Binds");
    }
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
    fn existing_media_and_time_zone_mounts_stay_transferable_without_host_system_access() {
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
            assert_eq!(source_image(&container, &image, template).unwrap(), digest);
        }
        for reference in [
            "untrusted/radarr:latest",
            "linuxserver/radarr:develop",
            "linuxserver/radarr:6.2.0-beta",
        ] {
            assert!(
                source_image(&json!({"Config":{"Image":reference}}), &image, template).is_err()
            );
        }
        assert!(
            source_image(
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
                source_image(
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
            let error =
                source_image(&json!({"Config":{"Image":reference}}), &image, template).unwrap_err();
            assert!(error.contains("stable image tag"), "{reference}: {error}");
        }
        let error = source_image(
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

    #[test]
    fn short_id_alias_from_older_engines_is_not_drift() {
        let id = format!("3de5eacc72c1{}", "0".repeat(52));
        let created = json!({"Id":id,"Image":"sha256:a","Config":{"Env":[]},"HostConfig":{"Privileged":false},
            "NetworkSettings":{"Networks":{"thelxinoe_default":{"NetworkID":"n","Aliases":["thelxinoe-server","server"]}}}});
        // Docker 24 adds the short ID when the container first starts.
        let mut started = created.clone();
        started["NetworkSettings"]["Networks"]["thelxinoe_default"]["Aliases"] =
            json!(["thelxinoe-server", "server", "3de5eacc72c1"]);
        assert!(matches(&fingerprint(&created), &started));
        assert_eq!(fingerprint(&created), fingerprint(&started));
        // Fingerprints recorded with the alias still match.
        let mut recorded = fingerprint(&created);
        recorded["networks"]["thelxinoe_default"]["aliases"] =
            json!(["thelxinoe-server", "server", "3de5eacc72c1"]);
        assert!(matches(&recorded, &started));
        // Backups compare stored records in today's form.
        assert_eq!(self::recorded(&recorded, Some(&id)), fingerprint(&created));
        let mut renamed = started.clone();
        renamed["NetworkSettings"]["Networks"]["thelxinoe_default"]["Aliases"] =
            json!(["thelxinoe-server", "3de5eacc72c1"]);
        assert!(!matches(&fingerprint(&created), &renamed));
        // A container with only its short ID alias reads as having none.
        assert_eq!(aliases(&json!(["3de5eacc72c1"]), Some(&id)), Value::Null);
        assert_eq!(aliases(&Value::Null, Some(&id)), Value::Null);
    }

    #[test]
    fn newer_engine_host_settings_are_not_service_drift() {
        let raw = json!({"Id":"a","Image":"sha256:a","Config":{},"HostConfig":{"Privileged":false,"CapAdd":null,"OomKillDisable":false}});
        let expected = fingerprint(&raw);
        let mut newer = raw.clone();
        newer["HostConfig"]["Annotations"] = json!({});
        newer["HostConfig"]["CapAdd"] = json!([]);
        newer["HostConfig"]["OomKillDisable"] = Value::Null;
        assert!(matches(&expected, &newer));
        newer["HostConfig"]["Privileged"] = json!(true);
        assert!(!matches(&expected, &newer));
    }
}
