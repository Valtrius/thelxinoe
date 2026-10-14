//! Pure ownership-transfer decisions: qualified images, storage layout, runtime identity,
//! network policy and blocking reasons. Docker and file mutations live in `transfer`.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A supported service family and its qualified compatibility branch.
#[derive(Clone, Copy)]
pub struct Family {
    pub kind: &'static str,
    /// Major version whose stable releases upgrade forward in place.
    pub branch: u64,
    /// Minimum qualified application version and image build of the branch.
    pub floor: &'static str,
    pub floor_build: Option<u64>,
    /// Immutable acceptance image for the floor; forward upgrades target it.
    pub floor_image: &'static str,
}

/// Measured from the minimum deployment matrix (`scripts/ci-images.json`).
pub const FAMILIES: [Family; 8] = [
    Family {
        kind: "radarr",
        branch: 6,
        floor: "6.4.4.10685",
        floor_build: Some(318),
        floor_image: "lscr.io/linuxserver/radarr@sha256:adb6c09d6b729ea5e642c99cea35af72702ef476bf4763f153299ac5db9f0b4f",
    },
    Family {
        kind: "sonarr",
        branch: 4,
        floor: "4.0.20.3014",
        floor_build: Some(326),
        floor_image: "lscr.io/linuxserver/sonarr@sha256:f247545d23ba8b233d6604575347e48a623fe6ad75dda02348bf81917f3b5c06",
    },
    Family {
        kind: "lidarr",
        branch: 3,
        floor: "3.1.0.4875",
        floor_build: Some(42),
        floor_image: "lscr.io/linuxserver/lidarr@sha256:044d616beb43c5e7810991242c6a9c42b93ff238c8c0850684939634ea751208",
    },
    Family {
        kind: "bazarr",
        branch: 1,
        floor: "1.6.2",
        floor_build: Some(366),
        floor_image: "lscr.io/linuxserver/bazarr@sha256:8b30e81c4aec2991f469e78fae8afaa89ecc9b21a80e3897f50427216b55470c",
    },
    Family {
        kind: "prowlarr",
        branch: 2,
        floor: "2.6.5.5623",
        floor_build: Some(162),
        floor_image: "lscr.io/linuxserver/prowlarr@sha256:f2b26429893d4c4cb71941b7ee50b1bdecd9d5f9f9e02d5410615e9f4f7c8d95",
    },
    Family {
        kind: "nzbget",
        branch: 26,
        floor: "26.3",
        floor_build: Some(265),
        floor_image: "lscr.io/linuxserver/nzbget@sha256:ac88c1293bec960e287bdaaca174020fab4c0f179f739e26f500d3def8015ed7",
    },
    Family {
        kind: "seerr",
        branch: 3,
        floor: "3.5.0",
        floor_build: None,
        floor_image: "ghcr.io/seerr-team/seerr@sha256:27602401178d54f1964442287b9f23f67a3fa2252645ee8065839ea1c3f69e45",
    },
    Family {
        kind: "recyclarr",
        branch: 8,
        floor: "8.7.2",
        floor_build: None,
        floor_image: "ghcr.io/recyclarr/recyclarr@sha256:6e69e009e1cd7493ff6093e8e187b5d3788c75b4a2c0c5127b6a1beda1c19728",
    },
];
pub fn family(kind: &str) -> Option<Family> {
    FAMILIES.iter().find(|f| f.kind == kind).copied()
}

/// A stable application version, with an optional LinuxServer image build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub numbers: Vec<u64>,
    pub build: Option<u64>,
    pub text: String,
}
impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let text = value.trim().trim_start_matches('v').to_owned();
        let (application, build) = match text.rsplit_once("-ls") {
            Some((application, build)) => (application, Some(build.parse::<u64>().ok()?)),
            None => (text.as_str(), None),
        };
        // Preview, branch and development builds never qualify.
        let numbers = application
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()?;
        (!numbers.is_empty() && numbers.len() <= 5).then_some(Self {
            numbers,
            build,
            text,
        })
    }
    pub fn major(&self) -> u64 {
        self.numbers[0]
    }
    /// Application release order, ignoring the image build.
    pub fn release_cmp(&self, other: &Self) -> std::cmp::Ordering {
        let length = self.numbers.len().max(other.numbers.len());
        let pad = |v: &Self| {
            let mut n = v.numbers.clone();
            n.resize(length, 0);
            n
        };
        pad(self).cmp(&pad(other))
    }
}

/// The application version an image declares in its labels.
pub fn image_version(image: &Value) -> Result<Version, String> {
    let labels = &image["Config"]["Labels"];
    let declared = labels["org.opencontainers.image.version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            labels["build_version"]
                .as_str()
                .and_then(|v| v.split("version:-").nth(1))
                .and_then(|v| v.split_whitespace().next())
        })
        .ok_or("The image does not declare its application version")?;
    Version::parse(declared).ok_or_else(|| {
        format!("{declared} is not a stable release; preview and development builds can't be transferred")
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ImageDecision {
    /// Keep the reviewed source image; no application upgrade.
    Same { image: String },
    /// Qualify a forward upgrade within the compatibility branch before activation.
    Upgrade { image: String, reason: String },
}
impl ImageDecision {
    pub fn image(&self) -> &str {
        match self {
            Self::Same { image } | Self::Upgrade { image, .. } => image,
        }
    }
}

/// Keep a supported source image; upgrade older releases of the branch to its floor image.
pub fn qualify_image(kind: &str, source: &str, version: &Version) -> Result<ImageDecision, String> {
    let family = family(kind).ok_or("Unsupported service")?;
    let floor = Version::parse(family.floor).ok_or("Invalid qualification floor")?;
    let label = crate::templates::label(kind);
    if version.major() != family.branch {
        return Err(format!(
            "Thelxinoe qualifies {label} {}.x; this container runs {}. Upgrade or migrate it with {label}'s own supported path first",
            family.branch, version.text
        ));
    }
    let older = match version.release_cmp(&floor) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => match (version.build, family.floor_build) {
            (Some(build), Some(floor)) => build < floor,
            _ => false,
        },
    };
    if !older {
        return Ok(ImageDecision::Same {
            image: source.to_owned(),
        });
    }
    let same_release = version.release_cmp(&floor) == std::cmp::Ordering::Equal;
    Ok(ImageDecision::Upgrade {
        image: family.floor_image.to_owned(),
        reason: if same_release {
            format!(
                "{} is an earlier image build of {}; the qualified build is validated against a copy first",
                version.text, family.floor
            )
        } else {
            format!(
                "{} is older than the qualified {} release; it is upgraded forward within {}.x after validation against a copy",
                version.text, family.floor, family.branch
            )
        },
    })
}

/// Where the image keeps its persistent state, and the variable that can move it.
pub fn appdata_destination(kind: &str, raw: &Value) -> String {
    let variable = match kind {
        "seerr" => Some(("CONFIG_DIRECTORY", "/app/config")),
        "recyclarr" => Some(("RECYCLARR_CONFIG_DIR", "/config")),
        _ => None,
    };
    let Some((name, default)) = variable else {
        return "/config".into();
    };
    env(raw)
        .into_iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| default.into())
        .trim_end_matches('/')
        .to_owned()
}

pub fn env(raw: &Value) -> Vec<(String, String)> {
    raw["Config"]["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    /// `bind` or `volume`.
    pub kind: String,
    /// Host path for binds, volume name for volumes.
    pub source: String,
    pub destination: String,
    pub writable: bool,
    pub role: String,
}

/// Source storage and how the replacement reaches the same files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layout {
    pub appdata: Mount,
    /// Media, download and time-zone mounts kept at the same container paths.
    pub retained: Vec<Mount>,
    /// Read-only configuration mounts copied into managed state and mounted from there.
    pub supplementary: Vec<Mount>,
}

pub struct Context<'a> {
    pub media_source: &'a str,
    /// Every host folder the server mounts at or below /media.
    pub server_media: &'a [String],
    pub appdata_source: &'a str,
    /// Every other container, inspected: writers and overlapping state.
    pub others: &'a [Value],
}

fn media_destination(destination: &str) -> bool {
    ["/media", "/movies", "/tv", "/music", "/downloads", "/data"]
        .iter()
        .any(|root| destination == *root || destination.starts_with(&format!("{root}/")))
}

fn mount_of(m: &Value) -> Result<Mount, String> {
    let kind = m["Type"].as_str().ok_or("Missing mount evidence")?;
    let source = if kind == "volume" {
        &m["Name"]
    } else {
        &m["Source"]
    }
    .as_str()
    .ok_or("Missing mount evidence")?;
    Ok(Mount {
        kind: kind.into(),
        source: source.into(),
        destination: m["Destination"]
            .as_str()
            .ok_or("Missing mount evidence")?
            .trim_end_matches('/')
            .into(),
        writable: m["RW"] == true,
        role: String::new(),
    })
}

fn within(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Classify the source container's storage. Every rejection names the mount and why.
pub fn layout(kind: &str, raw: &Value, context: &Context) -> Result<Layout, String> {
    let destination = appdata_destination(kind, raw);
    let mut mounts = Vec::new();
    for m in raw["Mounts"].as_array().ok_or("Missing mount evidence")? {
        let mount = mount_of(m)?;
        if !matches!(mount.kind.as_str(), "bind" | "volume") {
            return Err(format!(
                "The {} mount uses {} storage; transfers support host folders and Docker volumes",
                mount.destination, mount.kind
            ));
        }
        if mount.destination.ends_with("docker.sock") || mount.source.ends_with("docker.sock") {
            return Err(format!(
                "The Docker socket is mounted at {}; containers with Docker access can't be transferred",
                mount.destination
            ));
        }
        mounts.push(mount);
    }
    let Some(appdata) = mounts
        .iter()
        .find(|m| m.destination == destination)
        .cloned()
    else {
        return Err(format!(
            "{} keeps its configuration at {destination} inside the container's writable layer. Move it to a host folder or Docker volume first, then review again",
            crate::templates::label(kind)
        ));
    };
    if !appdata.writable {
        return Err(format!(
            "The configuration mount {destination} is read-only"
        ));
    }
    if let Some(nested) = mounts
        .iter()
        .find(|m| m.destination != destination && within(&m.destination, &destination))
    {
        return Err(format!(
            "{} is mounted separately inside the configuration folder; the configuration location is ambiguous",
            nested.destination
        ));
    }
    if appdata.kind == "bind" {
        if !crate::policy::appdata_isolated(&appdata.source, context.media_source)
            || context
                .server_media
                .iter()
                .any(|media| !crate::policy::appdata_isolated(&appdata.source, media))
        {
            return Err(format!(
                "The configuration folder {} overlaps media storage or a host system folder",
                appdata.source
            ));
        }
        if crate::policy::overlapping(&appdata.source, context.appdata_source) {
            return Err(
                "The configuration folder overlaps Thelxinoe's own deployment storage".into(),
            );
        }
    }
    let mut retained = Vec::new();
    let mut supplementary = Vec::new();
    for mut mount in mounts.into_iter().filter(|m| m.destination != destination) {
        if mount.kind == "bind" && crate::policy::host_time_zone(&mount.source) && !mount.writable {
            mount.role = "timezone".into();
            retained.push(mount);
            continue;
        }
        let shared_media = mount.kind == "bind"
            && (crate::policy::overlapping(&mount.source, context.media_source)
                || context
                    .server_media
                    .iter()
                    .any(|media| crate::policy::overlapping(&mount.source, media)));
        if media_destination(&mount.destination) || shared_media {
            if mount.kind == "bind"
                && let Some(problem) =
                    crate::policy::foreign_source_problem(&mount.source, context.appdata_source)
            {
                return Err(format!("The {} mount {problem}", mount.destination));
            }
            if mount.kind == "bind" && crate::policy::overlapping(&mount.source, &appdata.source) {
                return Err(format!(
                    "The {} mount overlaps the configuration folder",
                    mount.destination
                ));
            }
            mount.role = "media".into();
            retained.push(mount);
            continue;
        }
        if mount.writable {
            return Err(format!(
                "{} is writable and isn't a media or download folder, so it could hold application state outside the configuration copy",
                mount.destination
            ));
        }
        if mount.kind == "bind"
            && let Some(problem) =
                crate::policy::foreign_source_problem(&mount.source, context.appdata_source)
        {
            return Err(format!("The {} mount {problem}", mount.destination));
        }
        mount.role = "supplementary".into();
        supplementary.push(mount);
    }
    // Another service's configuration must never share this state.
    for other in context.others {
        if other["Id"] == raw["Id"] {
            continue;
        }
        for m in other["Mounts"].as_array().into_iter().flatten() {
            let Ok(theirs) = mount_of(m) else { continue };
            let same = theirs.kind == appdata.kind
                && if appdata.kind == "volume" {
                    theirs.source == appdata.source
                } else {
                    crate::policy::overlapping(&theirs.source, &appdata.source)
                };
            if same {
                let name = other["Name"]
                    .as_str()
                    .unwrap_or("another container")
                    .trim_start_matches('/');
                return Err(format!(
                    "{name} also mounts this configuration ({}); stop sharing it so the copy has a single writer",
                    appdata.source
                ));
            }
        }
    }
    Ok(Layout {
        appdata,
        retained,
        supplementary,
    })
}

/// Host-level access or deployment overrides that keep a container outside managed ownership.
pub fn boundary(kind: &str, raw: &Value, image: &Value) -> Option<String> {
    let host = &raw["HostConfig"];
    if host["Privileged"] == true {
        return Some("Privileged containers can't be transferred".into());
    }
    // The stopped original is the fallback; Docker would delete an auto-remove container.
    if host["AutoRemove"] == true {
        return Some(
            "The original runs with Docker auto-remove (--rm), so stopping it would delete it. Recreate it without auto-remove so it can be kept as a fallback, then review again"
                .into(),
        );
    }
    match host["NetworkMode"].as_str() {
        Some("host") => {
            return Some("Containers using host networking can't be transferred".into());
        }
        Some(mode) if mode.starts_with("container:") => {
            return Some(
                "This container uses another container's network (as VPN setups do), which transfers don't support"
                    .into(),
            );
        }
        _ => {}
    }
    for (key, label) in [
        ("PidMode", "process namespace"),
        ("IpcMode", "IPC namespace"),
        ("UsernsMode", "user namespace"),
        ("UTSMode", "host name namespace"),
    ] {
        if host[key] == "host" {
            return Some(format!(
                "Containers sharing the host's {label} can't be transferred"
            ));
        }
    }
    if let Some(cap) = host["CapAdd"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(Value::as_str)
    {
        return Some(format!("The added {cap} capability isn't supported"));
    }
    if let Some(device) = host["Devices"].as_array().and_then(|d| d.first()) {
        return Some(format!(
            "Host devices ({}) aren't supported",
            device["PathOnHost"].as_str().unwrap_or("device")
        ));
    }
    if host["DeviceRequests"]
        .as_array()
        .is_some_and(|d| !d.is_empty())
    {
        return Some("GPU and device requests aren't supported".into());
    }
    if host["SecurityOpt"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|v| {
            !matches!(
                v.as_str(),
                Some("no-new-privileges" | "no-new-privileges:true")
            )
        })
    {
        return Some("Custom security profiles aren't supported".into());
    }
    if host["Sysctls"].as_object().is_some_and(|s| !s.is_empty()) {
        return Some("Kernel parameter overrides (sysctls) aren't supported".into());
    }
    let config = &raw["Config"];
    for key in ["Cmd", "Entrypoint", "WorkingDir"] {
        if !crate::policy::same_default(&config[key], &image["Config"][key]) {
            return Some(format!(
                "A custom {} replaces the image's own startup and isn't supported",
                key.to_ascii_lowercase()
            ));
        }
    }
    for (name, value) in env(raw) {
        if image["Config"]["Env"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|d| d.as_str() == Some(&format!("{name}={value}")))
        {
            continue;
        }
        if !allowed_env(kind, &name) {
            return Some(format!(
                "The environment setting {name} changes how the image runs and isn't supported"
            ));
        }
    }
    None
}

/// Environment the managed template understands. Values are reviewed, never shown when secret.
fn allowed_env(kind: &str, name: &str) -> bool {
    match kind {
        "seerr" => matches!(name, "TZ" | "LOG_LEVEL" | "CONFIG_DIRECTORY" | "PORT"),
        "recyclarr" => matches!(
            name,
            "TZ" | "CRON_SCHEDULE" | "RECYCLARR_CONFIG_DIR" | "RECYCLARR_CREATE_CONFIG"
        ),
        _ => matches!(
            name,
            "PUID" | "PGID" | "TZ" | "UMASK" | "UMASK_SET" | "LANG" | "LC_ALL"
        ),
    }
}

/// The source runtime identity, as the image applies it.
pub fn source_identity(kind: &str, raw: &Value, image: &Value) -> Result<(u32, u32), String> {
    let numeric = |value: &str| {
        value
            .parse::<u32>()
            .map_err(|_| format!("{value} isn't a numeric user or group"))
    };
    match kind {
        "seerr" | "recyclarr" => {
            let user = raw["Config"]["User"]
                .as_str()
                .filter(|u| !u.is_empty())
                .or(image["Config"]["User"].as_str())
                .unwrap_or("");
            match user.split_once(':').unwrap_or((user, user)) {
                // The official Seerr image runs as its node user.
                ("node", _) | ("", _) if kind == "seerr" => Ok((1000, 1000)),
                (uid, gid) => {
                    let identity = (numeric(uid)?, numeric(gid)?);
                    if identity.0 == 0 {
                        return Err("Transfers require a non-root runtime identity".into());
                    }
                    Ok(identity)
                }
            }
        }
        _ => {
            let vars = env(raw);
            let value = |name: &str| {
                vars.iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("911")
                    .to_owned()
            };
            let identity = (numeric(&value("PUID"))?, numeric(&value("PGID"))?);
            if identity.0 == 0 || identity.1 == 0 {
                return Err("Transfers require non-root PUID and PGID values".into());
            }
            Ok(identity)
        }
    }
}

/// Docker-visible automation that could restart, recreate or write the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Automation {
    pub kind: String,
    pub name: String,
    pub scope: String,
    /// Observed scope can affect the source: blocks the transfer.
    pub blocks: bool,
}

pub fn automation(raw: &Value, others: &[Value], controller: &str) -> Vec<Automation> {
    const UPDATERS: [&str; 5] = [
        "containrrr/watchtower",
        "nickfedor/watchtower",
        "beatkind/watchtower",
        "pyouroboros/ouroboros",
        "jauderho/watchtower",
    ];
    let labels = &raw["Config"]["Labels"];
    let mut found = Vec::new();
    for other in others {
        if other["Id"] == raw["Id"]
            || other["Id"]
                .as_str()
                .is_some_and(|id| id.starts_with(controller))
            || other["Config"]["Labels"]["app.thelxinoe.deployment"].is_string()
            || other["Config"]["Labels"]["app.thelxinoe.component"].is_string()
        {
            continue;
        }
        let name = other["Name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('/')
            .to_owned();
        let image = other["Config"]["Image"].as_str().unwrap_or("");
        let repository = image.split('@').next().unwrap_or(image);
        let repository = repository
            .rsplit_once(':')
            .filter(|(_, tag)| !tag.contains('/'))
            .map_or(repository, |(r, _)| r);
        let updater = UPDATERS
            .iter()
            .any(|u| repository == *u || repository.ends_with(&format!("/{u}")));
        let socket = other["Mounts"].as_array().into_iter().flatten().any(|m| {
            m["Source"]
                .as_str()
                .is_some_and(|s| s.ends_with("docker.sock"))
        });
        if updater {
            let env = env(other);
            let label_mode = env
                .iter()
                .any(|(k, v)| k == "WATCHTOWER_LABEL_ENABLE" && matches!(v.as_str(), "true" | "1"));
            let scope_env = env
                .iter()
                .find(|(k, _)| k == "WATCHTOWER_SCOPE")
                .map(|(_, v)| v.clone());
            let excluded = labels["com.centurylinklabs.watchtower.enable"] == "false";
            let opted_in = labels["com.centurylinklabs.watchtower.enable"] == "true";
            let scoped_out = scope_env.as_ref().is_some_and(|scope| {
                labels["com.centurylinklabs.watchtower.scope"] != scope.as_str()
            });
            let affects = !excluded && !scoped_out && (!label_mode || opted_in);
            found.push(Automation {
                kind: "updater".into(),
                name,
                scope: if label_mode {
                    "Containers labelled com.centurylinklabs.watchtower.enable=true".into()
                } else if let Some(scope) = scope_env {
                    format!("Containers in the {scope} scope")
                } else {
                    "Every container on this Docker engine".into()
                },
                blocks: affects,
            });
        } else if socket {
            found.push(Automation {
                kind: "docker_access".into(),
                name,
                scope: "Unknown: this container controls Docker and Thelxinoe can't inspect what it automates".into(),
                blocks: false,
            });
        }
    }
    found
}

/// Compose ownership of the source and the unrelated services sharing its project.
pub fn compose(raw: &Value, others: &[Value]) -> Option<Value> {
    let labels = &raw["Config"]["Labels"];
    let project = labels["com.docker.compose.project"].as_str()?;
    let members = others
        .iter()
        .filter(|o| o["Id"] != raw["Id"] && o["Config"]["Labels"]["com.docker.compose.project"] == project)
        .map(|o| json!({"name":o["Name"].as_str().unwrap_or("").trim_start_matches('/'),
            "service":o["Config"]["Labels"]["com.docker.compose.service"],"running":o["State"]["Running"]}))
        .collect::<Vec<_>>();
    Some(
        json!({"project":project,"service":labels["com.docker.compose.service"],
        "working_dir":labels["com.docker.compose.project.working_dir"],
        "config_files":labels["com.docker.compose.project.config_files"],"unrelated":members}),
    )
}

/// Network aliases the replacement may keep without colliding with another container.
pub fn aliases(kind: &str, raw: &Value, network: &str, others: &[Value]) -> Vec<String> {
    let id = raw["Id"].as_str().unwrap_or("");
    let name = raw["Name"].as_str().unwrap_or("").trim_start_matches('/');
    let mut wanted: Vec<String> = raw["NetworkSettings"]["Networks"][network]["Aliases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|a| !id.starts_with(*a) && *a != id)
        .map(str::to_owned)
        .collect();
    wanted.push(name.to_owned());
    wanted.retain(|alias| {
        !alias.is_empty()
            && alias.len() <= 63
            && alias
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && !others.iter().any(|o| {
                o["Id"] != raw["Id"]
                    && (o["Name"].as_str().map(|n| n.trim_start_matches('/')) == Some(alias)
                        || o["NetworkSettings"]["Networks"][network]["Aliases"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|a| a == alias.as_str()))
            })
    });
    wanted.push(format!("thelxinoe-{kind}"));
    wanted.sort();
    wanted.dedup();
    wanted
}

/// Published host ports, rebound to loopback as fresh installations are.
pub fn ports(raw: &Value, port: Option<u16>) -> (Option<u16>, Vec<Value>) {
    let mut published = None;
    let mut changes = Vec::new();
    for (key, bindings) in raw["HostConfig"]["PortBindings"]
        .as_object()
        .into_iter()
        .flatten()
    {
        for binding in bindings.as_array().into_iter().flatten() {
            let host = binding["HostIp"]
                .as_str()
                .filter(|h| !h.is_empty())
                .unwrap_or("0.0.0.0");
            let host_port = binding["HostPort"].as_str().unwrap_or("");
            let before = format!("{host}:{host_port} → {key}");
            let internal = key.split('/').next().and_then(|p| p.parse::<u16>().ok());
            if internal == port
                && published.is_none()
                && let Ok(number) = host_port.parse::<u16>()
            {
                published = Some(number);
                if host != "127.0.0.1" || internal != port {
                    changes.push(json!({"area":"ports","before":before,"after":format!("127.0.0.1:{number} → {}/tcp",port.unwrap_or(0)),
                        "reason":"Managed services publish only on this host's loopback address; use Thelxinoe or a reverse proxy for remote access"}));
                }
            } else {
                changes.push(json!({"area":"ports","before":before,"after":"Not published",
                    "reason":"Only the service's own web port is published by managed deployments"}));
            }
        }
    }
    (published, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(others: &'a [Value]) -> Context<'a> {
        Context {
            media_source: "/srv/media",
            server_media: &[],
            appdata_source: "/var/lib/docker/volumes/thelxinoe_deployment/_data",
            others,
        }
    }
    fn bind(source: &str, destination: &str, writable: bool) -> Value {
        json!({"Type":"bind","Source":source,"Destination":destination,"RW":writable})
    }

    #[test]
    fn stable_versions_parse_and_previews_do_not() {
        let v = Version::parse("6.4.4.10685-ls318").unwrap();
        assert_eq!(
            (v.numbers.clone(), v.build),
            (vec![6, 4, 4, 10685], Some(318))
        );
        assert_eq!(Version::parse("v3.5.0").unwrap().numbers, vec![3, 5, 0]);
        for preview in [
            "develop",
            "6.5.0.1-nightly",
            "nightly-ls1",
            "v3.6.0-beta.1",
            "",
        ] {
            assert!(Version::parse(preview).is_none(), "{preview}");
        }
        let image = json!({"Config":{"Labels":{"build_version":"Linuxserver.io version:- v26.3-ls265 Build-date:- 2026"}}});
        assert_eq!(image_version(&image).unwrap().text, "26.3-ls265");
    }

    #[test]
    fn same_branch_images_are_kept_or_upgraded_forward_and_other_branches_rejected() {
        let source = "lscr.io/linuxserver/radarr@sha256:source";
        let keep = qualify_image("radarr", source, &Version::parse("6.5.0.1-ls1").unwrap());
        assert_eq!(
            keep.unwrap(),
            ImageDecision::Same {
                image: source.into()
            }
        );
        let floor = qualify_image(
            "radarr",
            source,
            &Version::parse("6.4.4.10685-ls318").unwrap(),
        );
        assert!(matches!(floor.unwrap(), ImageDecision::Same { .. }));
        // An earlier build of the same release changes the image without an upgrade.
        let build = qualify_image(
            "radarr",
            source,
            &Version::parse("6.4.4.10685-ls317").unwrap(),
        )
        .unwrap();
        assert!(
            matches!(&build, ImageDecision::Upgrade { reason, .. } if reason.contains("earlier image build"))
        );
        let older = qualify_image(
            "radarr",
            source,
            &Version::parse("6.3.0.10000-ls300").unwrap(),
        )
        .unwrap();
        assert_eq!(older.image(), family("radarr").unwrap().floor_image);
        assert!(
            qualify_image(
                "radarr",
                source,
                &Version::parse("5.26.2.10099-ls276").unwrap()
            )
            .unwrap_err()
            .contains("6.x")
        );
        assert!(qualify_image("radarr", source, &Version::parse("7.0.0.1-ls1").unwrap()).is_err());
    }

    #[test]
    fn layouts_accept_binds_volumes_and_legacy_media_paths() {
        let raw = json!({"Id":"a","Config":{"Env":[]},"Mounts":[
            {"Type":"volume","Name":"radarr_config","Source":"/var/lib/docker/volumes/radarr_config/_data","Destination":"/config","RW":true},
            bind("/srv/media/movies","/movies",true), bind("/srv/media/downloads","/downloads",true),
            bind("/etc/localtime","/etc/localtime",false), bind("/srv/certs","/certs",false)]});
        let layout = layout("radarr", &raw, &context(&[])).unwrap();
        assert_eq!(layout.appdata.kind, "volume");
        assert_eq!(layout.appdata.source, "radarr_config");
        assert_eq!(
            layout
                .retained
                .iter()
                .map(|m| m.role.as_str())
                .collect::<Vec<_>>(),
            ["media", "media", "timezone"]
        );
        assert_eq!(layout.supplementary[0].destination, "/certs");
    }

    #[test]
    fn configuration_containing_any_server_media_mount_is_rejected() {
        let raw = json!({"Id":"a","Config":{"Env":[]},"Mounts":[
            bind("/srv/shared", "/config", true), bind("/srv/media/movies", "/movies", true)]});
        let secondary = ["/srv/shared/movies".to_owned()];
        let context = Context {
            server_media: &secondary,
            ..context(&[])
        };
        assert!(
            layout("radarr", &raw, &context)
                .unwrap_err()
                .contains("overlaps media")
        );
        assert!(layout("radarr", &raw, &self::context(&[])).is_ok());
    }

    #[test]
    fn ambiguous_or_unsupported_storage_is_rejected_with_its_reason() {
        let reason = |mounts: Value, others: &[Value]| {
            layout(
                "radarr",
                &json!({"Id":"a","Config":{"Env":[]},"Mounts":mounts}),
                &context(others),
            )
            .unwrap_err()
        };
        assert!(
            reason(json!([bind("/srv/media/movies", "/movies", true)]), &[])
                .contains("writable layer")
        );
        assert!(
            reason(
                json!([
                    bind("/srv/appdata/radarr", "/config", true),
                    bind("/srv/logs", "/config/logs", true)
                ]),
                &[]
            )
            .contains("ambiguous")
        );
        assert!(
            reason(
                json!([
                    bind("/srv/appdata/radarr", "/config", true),
                    bind("/srv/backups", "/backups", true)
                ]),
                &[]
            )
            .contains("/backups is writable")
        );
        assert!(
            reason(json!([bind("/srv/media/radarr", "/config", true)]), &[])
                .contains("overlaps media")
        );
        assert!(
            reason(
                json!([
                    bind("/srv/appdata/radarr", "/config", true),
                    bind("/var/run/docker.sock", "/var/run/docker.sock", true)
                ]),
                &[]
            )
            .contains("Docker socket")
        );
        let writer = json!({"Id":"b","Name":"/backup-sidecar","Mounts":[bind("/srv/appdata/radarr","/source",false)]});
        assert!(
            reason(
                json!([bind("/srv/appdata/radarr", "/config", true)]),
                &[writer]
            )
            .contains("backup-sidecar")
        );
    }

    #[test]
    fn seerr_and_recyclarr_use_their_image_native_configuration_folders() {
        assert_eq!(
            appdata_destination("seerr", &json!({"Config":{"Env":[]}})),
            "/app/config"
        );
        assert_eq!(
            appdata_destination(
                "seerr",
                &json!({"Config":{"Env":["CONFIG_DIRECTORY=/data/seerr/"]}})
            ),
            "/data/seerr"
        );
        assert_eq!(
            appdata_destination("recyclarr", &json!({"Config":{"Env":[]}})),
            "/config"
        );
        let image = json!({"Config":{"User":"node:node"}});
        assert_eq!(
            source_identity("seerr", &json!({"Config":{}}), &image).unwrap(),
            (1000, 1000)
        );
        assert_eq!(
            source_identity("recyclarr", &json!({"Config":{"User":"1001:1001"}}), &image).unwrap(),
            (1001, 1001)
        );
        assert!(source_identity("recyclarr", &json!({"Config":{"User":"0:0"}}), &image).is_err());
        assert_eq!(
            source_identity(
                "radarr",
                &json!({"Config":{"Env":["PUID=1001","PGID=1001"]}}),
                &image
            )
            .unwrap(),
            (1001, 1001)
        );
    }

    #[test]
    fn privileges_and_runtime_overrides_block_before_any_change() {
        let image = json!({"Config":{"Env":["PATH=/usr/bin"]}});
        let base = json!({"HostConfig":{},"Config":{"Env":["PUID=1000","PATH=/usr/bin"]}});
        assert_eq!(boundary("radarr", &base, &image), None);
        let mut mods = base.clone();
        mods["Config"]["Env"] = json!(["DOCKER_MODS=linuxserver/mods:radarr-striptracks"]);
        assert!(
            boundary("radarr", &mods, &image)
                .unwrap()
                .contains("DOCKER_MODS")
        );
        let mut vpn = base.clone();
        vpn["HostConfig"]["NetworkMode"] = json!("container:gluetun");
        assert!(boundary("radarr", &vpn, &image).unwrap().contains("VPN"));
        let mut removed = base.clone();
        removed["HostConfig"]["AutoRemove"] = json!(true);
        assert!(
            boundary("radarr", &removed, &image)
                .unwrap()
                .contains("auto-remove")
        );
        let mut gpu = base;
        gpu["HostConfig"]["Devices"] = json!([{"PathOnHost":"/dev/dri"}]);
        assert!(
            boundary("radarr", &gpu, &image)
                .unwrap()
                .contains("/dev/dri")
        );
    }

    #[test]
    fn automation_evidence_separates_blocking_updaters_from_unknown_docker_access() {
        let source = json!({"Id":"source","Config":{"Labels":{}}});
        let watchtower = json!({"Id":"w","Name":"/watchtower","Config":{"Image":"containrrr/watchtower:latest","Env":[]},"Mounts":[]});
        let labelled = json!({"Id":"l","Name":"/labelled","Config":{"Image":"containrrr/watchtower","Env":["WATCHTOWER_LABEL_ENABLE=true"]},"Mounts":[]});
        let portainer = json!({"Id":"p","Name":"/portainer","Config":{"Image":"portainer/portainer-ce"},"Mounts":[{"Source":"/var/run/docker.sock"}]});
        let found = automation(&source, &[watchtower, labelled, portainer], "controller");
        assert_eq!(
            found.iter().map(|a| a.blocks).collect::<Vec<_>>(),
            [true, false, false]
        );
        assert_eq!(found[2].kind, "docker_access");
        let excluded = json!({"Id":"source","Config":{"Labels":{"com.centurylinklabs.watchtower.enable":"false"}}});
        let watchtower = json!({"Id":"w","Name":"/watchtower","Config":{"Image":"containrrr/watchtower:latest","Env":[]},"Mounts":[]});
        assert!(!automation(&excluded, &[watchtower], "controller")[0].blocks);
    }

    #[test]
    fn replacements_keep_collision_free_aliases_and_publish_only_on_loopback() {
        let raw = json!({"Id":"abcdef1234567890","Name":"/radarr","NetworkSettings":{"Networks":{"media":{"Aliases":["radarr","movies","abcdef123456"]}}},
            "HostConfig":{"PortBindings":{"7878/tcp":[{"HostIp":"","HostPort":"7878"}],"9000/tcp":[{"HostPort":"9000"}]}}});
        let other = json!({"Id":"other","Name":"/movies","NetworkSettings":{"Networks":{}}});
        assert_eq!(
            aliases("radarr", &raw, "media", &[other]),
            ["radarr", "thelxinoe-radarr"]
        );
        let (published, changes) = ports(&raw, Some(7878));
        assert_eq!(published, Some(7878));
        assert_eq!(changes.len(), 2);
    }
}
