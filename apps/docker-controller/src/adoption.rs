//! Register exact existing deployments without changing their Docker or application state.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Capability {
    pub available: bool,
    pub reason: Option<String>,
}
impl Capability {
    fn available() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }
    fn blocked(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Capabilities {
    pub lifecycle: Capability,
    pub backup: Capability,
    pub update: Capability,
    pub recreate: Capability,
    pub release: Capability,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StorageReference {
    pub kind: String,
    pub source: String,
    pub destination: String,
    pub writable: bool,
    pub borrowed: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Imported {
    pub engine: String,
    pub deployment: String,
    pub storage: Vec<StorageReference>,
    pub capabilities: Capabilities,
}
#[derive(Clone, Serialize, Deserialize)]
struct Plan {
    review_id: String,
    kind: String,
    container_id: String,
    name: String,
    image: String,
    mode: String,
    authentication: String,
    restart_required: bool,
    changes: Vec<Value>,
    capabilities: Capabilities,
    source_config: Option<String>,
    compose_project: Option<String>,
    compose_service: Option<String>,
    running: bool,
    automation: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Review {
    plan: Plan,
    original: Value,
    imported: Imported,
    deployment: String,
}
fn review_path(key: &str) -> std::path::PathBuf {
    store::root()
        .join("adoption-reviews")
        .join(format!("{key}.json"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preview {
    kind: String,
    container_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Adopt {
    operation_id: String,
    kind: String,
    container_id: String,
    #[serde(default)]
    released_compose: bool,
}
pub(super) async fn engine_identity() -> Result<String> {
    engine("/info").await?["ID"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(unavailable)
}
fn media_destination(destination: &str) -> bool {
    ["/media", "/movies", "/tv", "/music", "/downloads", "/data"]
        .iter()
        .any(|root| destination == *root || destination.starts_with(&format!("{root}/")))
}
/// Media and download storage is shared with Thelxinoe rather than backed up per service.
fn media_mount(m: &Value, d: &Deployment) -> bool {
    m["Destination"].as_str().is_some_and(media_destination)
        || m["Source"]
            .as_str()
            .is_some_and(|source| policy::overlapping(source, &d.media_source))
}
pub(super) fn state_isolated(source: &str, server: &Value, d: &Deployment) -> bool {
    policy::media_disjoint(source, &d.media_source)
        && server["Mounts"].as_array().is_some_and(|mounts| {
            mounts.iter().all(|m| {
                !m["Destination"].as_str().is_some_and(media_destination)
                    || m["Source"]
                        .as_str()
                        .is_some_and(|media| policy::media_disjoint(source, media))
            })
        })
}
pub(super) fn backup_source(raw: &Value, d: &Deployment) -> Result<String> {
    if raw["HostConfig"]["AutoRemove"] == true {
        return Err(conflict(
            "Docker auto-remove is enabled for this container, so it can't be stopped for a consistent backup",
        ));
    }
    let mounts = raw["Mounts"].as_array().ok_or_else(unavailable)?;
    let config = mounts
        .iter()
        .find(|m| m["Destination"] == "/config")
        .ok_or_else(|| conflict("Backups need the service configuration mounted at /config"))?;
    if config["Type"] == "volume" {
        return Err(conflict(format!(
            "Backups need /config to be a host folder; it currently uses the Docker volume {}",
            config["Name"].as_str().unwrap_or_default()
        )));
    }
    if config["Type"] != "bind" {
        return Err(conflict("Backups need /config to be a host folder"));
    }
    let source = config["Source"].as_str().ok_or_else(unavailable)?;
    if config["RW"] != true {
        return Err(conflict(
            "Backups need /config to be writable so a restore can put it back",
        ));
    }
    if !state_isolated(source, &d.server, d) {
        return Err(conflict(format!(
            "/config ({source}) overlaps media storage; keep the service configuration outside media folders so backups never copy media"
        )));
    }
    for m in mounts
        .iter()
        .filter(|m| m["Destination"] != "/config" && m["Type"] != "tmpfs")
    {
        let destination = m["Destination"].as_str().ok_or_else(unavailable)?;
        if !m["Source"]
            .as_str()
            .is_some_and(|other| policy::media_disjoint(source, other))
        {
            return Err(conflict(format!(
                "The {destination} mount overlaps /config ({source}); keep configuration and other storage in separate folders"
            )));
        }
        if m["RW"] == true && !media_mount(m, d) {
            return Err(conflict(format!(
                "{destination} is writable and isn't a media or download folder, so backing up /config alone could miss service data stored there"
            )));
        }
    }
    Ok(policy::host_path(source)
        .ok_or_else(|| conflict(format!("/config uses an unsupported host path ({source})")))?
        .to_string_lossy()
        .into_owned())
}
pub(super) fn refresh_capabilities(raw: &Value, d: &Deployment, capabilities: &mut Capabilities) {
    capabilities.lifecycle = if raw["HostConfig"]["AutoRemove"] == true {
        Capability::blocked(
            "Docker auto-remove is enabled for this container, so stopping it would delete it",
        )
    } else {
        Capability::available()
    };
    capabilities.backup = match backup_source(raw, d) {
        Ok(_) => Capability::available(),
        Err(error) => Capability::blocked(&error.1),
    };
}
/// The first host-level privilege that keeps this container outside controller ownership.
fn boundary(host: &Value, network_peer: Option<&str>) -> Option<String> {
    if host["Privileged"] == true {
        return Some("Privileged containers can't be adopted".into());
    }
    if host["NetworkMode"] == "host" {
        return Some("Containers using host networking can't be adopted".into());
    }
    for (key, label) in [
        ("PidMode", "process namespace (pid: host)"),
        ("IpcMode", "IPC namespace (ipc: host)"),
        ("UsernsMode", "user namespace (userns_mode: host)"),
    ] {
        if host[key] == "host" {
            return Some(format!(
                "Containers sharing the host's {label} can't be adopted"
            ));
        }
    }
    if let Some(cap) = host["CapAdd"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|cap| matches!(*cap, "SYS_ADMIN" | "CAP_SYS_ADMIN" | "ALL"))
    {
        return Some(format!(
            "Containers with the {cap} capability can't be adopted"
        ));
    }
    if let Some(device) = host["Devices"]
        .as_array()
        .and_then(|devices| devices.first())
    {
        return Some(format!(
            "Containers with host devices ({}) can't be adopted",
            device["PathOnHost"].as_str().unwrap_or("device")
        ));
    }
    network_peer.map(|peer| {
        format!(
            "This container uses the network of {peer} (as VPN setups do), which isn't supported for adoption yet"
        )
    })
}
/// Name the container whose network namespace this one joins, as `docker ps` would.
async fn network_peer(host: &Value) -> Option<String> {
    let peer = host["NetworkMode"].as_str()?.strip_prefix("container:")?;
    let name = if !peer.is_empty()
        && peer
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        engine(&format!("/containers/{peer}/json"))
            .await
            .ok()
            .and_then(|raw| {
                raw["Name"]
                    .as_str()
                    .map(|n| n.trim_start_matches('/').to_owned())
            })
    } else {
        None
    };
    Some(name.unwrap_or_else(|| peer.chars().take(12).collect()))
}
/// Why one existing mount keeps this deployment outside controller ownership, if it does.
fn mount_problem(
    kind: &str,
    source: &str,
    destination: &str,
    writable: bool,
    d: &Deployment,
) -> Option<String> {
    if ["/var/run/docker.sock", "/run/docker.sock"].contains(&destination)
        || source.ends_with("/docker.sock")
    {
        return Some(format!(
            "The Docker socket is mounted at {destination}; containers with Docker access can't be adopted"
        ));
    }
    if kind != "bind" || (!writable && policy::host_time_zone(source)) {
        return None;
    }
    if destination == "/config" {
        return (!policy::appdata_isolated(source, &d.appdata_source)).then(|| {
            format!(
                "/config uses {source}; service configuration needs its own folder, not a host system folder, a top-level folder or Thelxinoe's own storage"
            )
        });
    }
    policy::foreign_source_problem(source, &d.appdata_source)
        .map(|problem| format!("The {destination} mount {problem} ({source}) and can't be adopted"))
}
/// Updaters that recreate containers would silently replace the adopted identity.
async fn automation(raw: &Value) -> Result<Vec<String>> {
    const UPDATERS: [&str; 4] = [
        "containrrr/watchtower",
        "nickfedor/watchtower",
        "beatkind/watchtower",
        "pyouroboros/ouroboros",
    ];
    let labels = &raw["Config"]["Labels"];
    if labels["com.centurylinklabs.watchtower.enable"] == "false" {
        return Ok(vec![]);
    }
    let mut warnings = Vec::new();
    for row in engine("/containers/json")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        let image = row["Image"].as_str().unwrap_or_default();
        let repository = image.split('@').next().unwrap_or(image);
        let repository = repository
            .rsplit_once(':')
            .filter(|(_, tag)| !tag.contains('/'))
            .map_or(repository, |(repository, _)| repository);
        let Some(updater) = UPDATERS
            .iter()
            .find(|u| repository == **u || repository.ends_with(&format!("/{u}")))
        else {
            continue;
        };
        let watchtower = updater.ends_with("/watchtower");
        if watchtower && labels["com.centurylinklabs.watchtower.enable"] != "true" {
            // Label-enable mode only updates containers that opted in.
            let id = row["Id"].as_str().ok_or_else(unavailable)?;
            let updater = engine(&format!("/containers/{id}/json")).await?;
            if updater["Config"]["Env"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|entry| {
                    matches!(
                        entry.as_str(),
                        Some("WATCHTOWER_LABEL_ENABLE=true" | "WATCHTOWER_LABEL_ENABLE=1")
                    )
                })
            {
                continue;
            }
        }
        let name = row["Names"][0]
            .as_str()
            .unwrap_or("An updater")
            .trim_start_matches('/');
        warnings.push(format!(
            "{name} updates containers automatically and could replace this one, after which Thelxinoe would lose track of it. Exclude this container from {name} before taking ownership{}.",
            if watchtower {
                " (label com.centurylinklabs.watchtower.enable=false)"
            } else {
                ""
            }
        ));
    }
    Ok(warnings)
}
async fn inspect(input: &Preview, d: &Deployment) -> Result<(Value, Imported, String)> {
    let t = templates::find(&input.kind).ok_or_else(|| bad("Unknown supported service"))?;
    if input.container_id.len() != 64 || !input.container_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad("Use the full Docker container ID"));
    }
    let raw = engine(&format!("/containers/{}/json", input.container_id)).await?;
    let image = engine(&format!(
        "/images/{}/json",
        raw["Image"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    let pinned = policy::adoption_image(&raw, &image, t).map_err(conflict)?;
    if image["Os"] != "linux" || image["Architecture"] != "amd64" {
        return Err(conflict(format!(
            "Ownership requires a Linux x86-64 image; this container runs {}/{}",
            image["Os"].as_str().unwrap_or("unknown"),
            image["Architecture"].as_str().unwrap_or("unknown")
        )));
    }
    policy::transfer_owner(&raw, true).map_err(conflict)?;
    let host = &raw["HostConfig"];
    if let Some(reason) = boundary(host, network_peer(host).await.as_deref()) {
        return Err(conflict(reason));
    }
    let mut storage = Vec::new();
    for m in raw["Mounts"].as_array().ok_or_else(unavailable)? {
        let kind = m["Type"].as_str().ok_or_else(unavailable)?;
        let source = if kind == "volume" {
            &m["Name"]
        } else {
            &m["Source"]
        }
        .as_str()
        .ok_or_else(unavailable)?;
        let destination = m["Destination"].as_str().ok_or_else(unavailable)?;
        if let Some(reason) = mount_problem(kind, source, destination, m["RW"] == true, d) {
            return Err(conflict(reason));
        }
        storage.push(StorageReference {
            kind: kind.into(),
            source: source.into(),
            destination: destination.into(),
            writable: m["RW"] == true,
            borrowed: true,
        });
    }
    let mut capabilities = Capabilities {
        lifecycle: Capability::available(),
        backup: Capability::available(),
        update: Capability::blocked(
            "Faithful recreation and recovery must be verified before updating this imported deployment",
        ),
        recreate: Capability::blocked(
            "Review a replacement deployment before changing the registered container identity",
        ),
        release: Capability::available(),
    };
    refresh_capabilities(&raw, d, &mut capabilities);
    Ok((
        raw,
        Imported {
            engine: engine_identity().await?,
            deployment: d.id.clone(),
            storage,
            capabilities,
        },
        pinned,
    ))
}
pub(super) async fn preview(
    State(runtime): State<Runtime>,
    Json(input): Json<Preview>,
) -> Result<Json<Value>> {
    if input.kind == "recyclarr" {
        return recyclarr_import::preview(
            State(runtime),
            Json(
                serde_json::from_value(
                    json!({"kind":input.kind,"container_id":input.container_id}),
                )
                .map_err(|_| unavailable())?,
            ),
        )
        .await;
    }
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    ensure_kind_available(&d, &input.kind).await?;
    let (raw, imported, image) = inspect(&input, &d).await?;
    let automation = automation(&raw).await?;
    let plan = Plan {
        review_id: thelxinoe_core::id(),
        kind: input.kind,
        container_id: input.container_id,
        name: raw["Name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('/')
            .into(),
        image,
        mode: "in_place".into(),
        authentication: "preserved".into(),
        restart_required: false,
        changes: vec![],
        capabilities: imported.capabilities.clone(),
        source_config: imported
            .storage
            .iter()
            .find(|m| m.destination == "/config")
            .map(|m| m.source.clone()),
        compose_project: raw["Config"]["Labels"]["com.docker.compose.project"]
            .as_str()
            .map(str::to_owned),
        compose_service: raw["Config"]["Labels"]["com.docker.compose.service"]
            .as_str()
            .map(str::to_owned),
        running: raw["State"]["Running"] == true,
        automation,
    };
    let review = Review {
        plan,
        original: raw,
        imported,
        deployment: d.id,
    };
    persisted(store::write_secret(
        &review_path(&review.plan.review_id),
        &format!("review:{}", review.plan.review_id),
        &review,
    ))?;
    Ok(Json(
        serde_json::to_value(review.plan).map_err(|_| unavailable())?,
    ))
}
async fn checked(d: &Deployment, input: &Adopt) -> Result<Review> {
    id(&input.operation_id)?;
    let review: Review = persisted(store::read_secret(
        &review_path(&input.operation_id),
        &format!("review:{}", input.operation_id),
    ))?;
    if review.plan.kind != input.kind
        || review.plan.container_id != input.container_id
        || review.deployment != d.id
    {
        return Err(conflict(
            "This ownership review belongs to a different deployment or container",
        ));
    }
    if review.plan.compose_project.is_some() && !input.released_compose {
        return Err(conflict(
            "Retire the entire external deployment automation before adopting in place; an active Compose project requires supported detachment",
        ));
    }
    let (raw, imported, image) = inspect(
        &Preview {
            kind: input.kind.clone(),
            container_id: input.container_id.clone(),
        },
        d,
    )
    .await?;
    if imported.engine != review.imported.engine
        || image != review.plan.image
        || policy::fingerprint(&raw) != policy::fingerprint(&review.original)
        || raw["State"]["Running"] != review.original["State"]["Running"]
    {
        return Err(conflict(
            "The reviewed deployment changed; review ownership again",
        ));
    }
    Ok(review)
}
pub(super) async fn check(
    State(runtime): State<Runtime>,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    if input.kind == "recyclarr" {
        return recyclarr_import::check(
            State(runtime),
            Json(
                serde_json::from_value(serde_json::to_value(&input).map_err(|_| unavailable())?)
                    .map_err(|_| unavailable())?,
            ),
        )
        .await;
    }
    let _guard = runtime.0.service(&input.kind).await;
    checked(&bootstrap().await?, &input).await?;
    Ok(Json(json!({"accepted":true})))
}
pub(super) async fn adopt(
    State(runtime): State<Runtime>,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    if input.kind == "recyclarr" {
        return recyclarr_import::adopt(
            State(runtime),
            Json(
                serde_json::from_value(serde_json::to_value(&input).map_err(|_| unavailable())?)
                    .map_err(|_| unavailable())?,
            ),
        )
        .await;
    }
    id(&input.operation_id)?;
    let _guard = runtime.0.service(&input.kind).await;
    if !crate::lease::active() {
        return Err(conflict("The controller writer lease is unavailable"));
    }
    let d = bootstrap().await?;
    if service_path(&input.operation_id).exists() {
        let s = load(&input.operation_id)?;
        if s.kind != input.kind
            || s.container != input.container_id
            || s.imported.is_none()
            || s.phase == "returned"
        {
            return Err(conflict(
                "This operation ID already belongs to another ownership decision",
            ));
        }
        verify_fingerprint(
            &d,
            &s,
            &engine(&format!("/containers/{}/json", s.container)).await?,
        )
        .await?;
        return Ok(Json(
            json!({"id":s.id,"kind":s.kind,"container_id":s.container,"accepted":true}),
        ));
    }
    ensure_kind_available(&d, &input.kind).await?;
    let review = checked(&d, &input).await?;
    let s = Managed {
        id: input.operation_id,
        kind: input.kind,
        container: input.container_id,
        name: review.plan.name,
        image: review.plan.image,
        phase: "active".into(),
        active_update: None,
        spec: json!({"Image":review.original["Image"],"Env":review.original["Config"]["Env"],"HostConfig":review.original["HostConfig"],"Config":review.original["Config"],"Networks":review.original["NetworkSettings"]["Networks"]}),
        expected: policy::fingerprint(&review.original),
        imported: Some(review.imported),
        error: None,
    };
    save(&s)?;
    Ok(Json(
        json!({"id":s.id,"kind":s.kind,"container_id":s.container,"accepted":true}),
    ))
}
pub(super) async fn release(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if !crate::lease::active() {
        return Err(conflict("The controller writer lease is unavailable"));
    }
    if s.phase == "returned" {
        return Ok(Json(json!({"accepted":true,"released":true})));
    }
    if !matches!(s.phase.as_str(), "active" | "changing") || s.active_update.is_some() {
        return Err(conflict(
            "Finish the active operation before releasing ownership",
        ));
    }
    if s.imported.is_none() {
        return Err(conflict(
            "Release ownership is available for imported deployments",
        ));
    }
    if s.imported.as_ref().is_some_and(|v| v.engine.is_empty()) || d.id.is_empty() {
        return Err(unavailable());
    }
    let imported = s.imported.as_ref().unwrap();
    if imported.deployment != d.id || imported.engine != engine_identity().await? {
        return Err(conflict(
            "The registered deployment or Docker engine identity changed",
        ));
    }
    s.phase = "returned".into();
    save(s)?;
    Ok(Json(
        json!({"accepted":true,"released":true,"container_id":s.container,"data_preserved":true}),
    ))
}
pub(super) fn pending(s: &Managed) -> bool {
    s.kind == "recyclarr" && recyclarr_import::pending(s)
}
pub(super) async fn complete(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if s.kind == "recyclarr" {
        return recyclarr_import::complete(d, s).await;
    }
    updates::verified(s, d).await?;
    Ok(Json(json!({"accepted":true})))
}

pub(super) async fn cancel_unsubmitted(d: &Deployment, key: &str) -> Result<Json<Value>> {
    recyclarr_import::cancel_unsubmitted(d, key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment() -> Deployment {
        serde_json::from_value(json!({"id":"deployment","generation":1,"version":"test",
            "server":{"Mounts":[{"Type":"bind","Source":"/data/media","Destination":"/media","RW":true}]},
            "controller":{},"network":"media","media_source":"/data/media",
            "appdata_source":"/var/lib/docker/volumes/thelxinoe_deployment/_data"}))
        .unwrap()
    }
    fn bind(source: &str, destination: &str, writable: bool) -> Value {
        json!({"Type":"bind","Source":source,"Destination":destination,"RW":writable})
    }

    #[test]
    fn trash_guides_layout_is_adoptable_and_backed_up() {
        let d = deployment();
        let mounts = [
            bind("/etc/localtime", "/etc/localtime", false),
            bind("/docker/appdata/radarr", "/config", true),
            bind("/data", "/data", true),
        ];
        for m in &mounts {
            let source = m["Source"].as_str().unwrap();
            let destination = m["Destination"].as_str().unwrap();
            assert_eq!(
                mount_problem("bind", source, destination, m["RW"] == true, &d),
                None,
                "{destination}"
            );
        }
        let raw = json!({"HostConfig":{},"Mounts":mounts});
        assert_eq!(backup_source(&raw, &d).unwrap(), "/docker/appdata/radarr");
    }

    #[test]
    fn rejected_mounts_and_privileges_name_their_reason() {
        let d = deployment();
        let problem = |source: &str, destination: &str, writable: bool| {
            mount_problem("bind", source, destination, writable, &d).unwrap()
        };
        assert!(problem("/etc/localtime", "/etc/localtime", true).contains("host system folder"));
        assert!(problem("/etc", "/host-etc", false).contains("The /host-etc mount"));
        assert!(
            problem("/var/run/docker.sock", "/var/run/docker.sock", true).contains("Docker socket")
        );
        assert!(problem("/radarr", "/config", true).starts_with("/config uses /radarr"));
        assert!(
            problem(&format!("{}/x", d.appdata_source), "/backups", true)
                .contains("Thelxinoe's own deployment storage")
        );
        assert_eq!(
            boundary(&json!({"CapAdd":["NET_ADMIN","SYS_ADMIN"]}), None).unwrap(),
            "Containers with the SYS_ADMIN capability can't be adopted"
        );
        assert!(
            boundary(&json!({"Devices":[{"PathOnHost":"/dev/dri"}]}), None)
                .unwrap()
                .contains("/dev/dri")
        );
        assert!(
            boundary(&json!({"NetworkMode":"container:abc"}), Some("gluetun"))
                .unwrap()
                .contains("network of gluetun")
        );
        assert!(
            boundary(&json!({"PidMode":"host"}), None)
                .unwrap()
                .contains("pid: host")
        );
        assert_eq!(boundary(&json!({"NetworkMode":"media"}), None), None);
    }

    #[test]
    fn backup_coverage_reports_the_mount_that_blocks_it() {
        let d = deployment();
        let reason = |mounts: Value| {
            backup_source(&json!({"HostConfig":{},"Mounts":mounts}), &d)
                .unwrap_err()
                .1
        };
        assert!(
            reason(json!([{"Type":"volume","Name":"radarr_config","Source":"/var/lib/docker/volumes/radarr_config/_data","Destination":"/config","RW":true}]))
                .contains("Docker volume radarr_config")
        );
        assert!(reason(json!([])).contains("mounted at /config"));
        assert!(
            reason(json!([
                bind("/srv/radarr", "/config", true),
                bind("/srv/backups", "/backups", true)
            ]))
            .starts_with("/backups is writable")
        );
        assert!(
            reason(json!([
                bind("/srv/radarr", "/config", true),
                bind("/srv", "/srv", false)
            ]))
            .contains("overlaps /config")
        );
        assert!(
            reason(json!([bind("/data/media/radarr", "/config", true)]))
                .contains("overlaps media storage")
        );
        // Shared Thelxinoe media is recognized by its source, whatever its destination.
        assert!(
            backup_source(
                &json!({"HostConfig":{},"Mounts":[bind("/srv/radarr", "/config", true), bind("/data/media/films", "/films", true)]}),
                &d
            )
            .is_ok()
        );
    }
}
