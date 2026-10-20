//! Copy-based ownership transfer. The original keeps its container and appdata; a managed
//! replacement is built from an untouched snapshot after disposable isolated validation.
//! Each irreversible transition is journaled before it happens. After the production
//! boundary, recovery runs one operation-bound command at a time, journaled phase by phase,
//! so a lost reply or a restart continues that exact command instead of choosing again.
use super::*;
use crate::transfer_plan as plan;
use reqwest::Method;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

fn directory(key: &str) -> PathBuf {
    store::root().join("transfers").join(key)
}
fn host(d: &Deployment, key: &str, leaf: &str) -> String {
    format!("{}/transfers/{key}/{leaf}", d.appdata_source)
}
fn stale(message: impl Into<std::borrow::Cow<'static, str>>) -> crate::docker::Failure {
    (StatusCode::PRECONDITION_FAILED, message.into())
}
/// Another operation holds the service; the caller observes again rather than failing.
fn busy() -> crate::docker::Failure {
    (
        StatusCode::LOCKED,
        "This service has another active operation".into(),
    )
}
/// Outcomes that end a transfer; nothing remains to recover after them.
fn terminal(stage: &str) -> bool {
    matches!(stage, "complete" | "returned" | "cancelled" | "stale")
}
/// Supplementary single files are copied into a folder under this name and mounted from there.
const SUPPLEMENTARY_FILE: &str = "file";
fn digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

/// Everything the administrator reviewed, including private Docker evidence.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Review {
    pub id: String,
    pub kind: String,
    pub service: String,
    pub deployment: String,
    pub engine: String,
    original: Value,
    source_image: String,
    source_version: plan::Version,
    decision: plan::ImageDecision,
    target_version: plan::Version,
    layout: plan::Layout,
    source_identity: (u32, u32),
    target_identity: (u32, u32),
    aliases: Vec<String>,
    published: Option<u16>,
    automation: Vec<plan::Automation>,
    compose: Option<Value>,
    /// Read-only inspection of the live configuration at review time.
    inspection: Value,
    access: Value,
    changes: Vec<Value>,
    settings: Vec<Value>,
    transformations: Value,
    /// Recyclarr's supported targets; API keys only as SHA-256 digests.
    #[serde(default)]
    recyclarr: Value,
    blockers: Vec<String>,
    evidence_revision: String,
    /// One digest per reviewed component, to name what changed before a stale start.
    #[serde(default)]
    parts: BTreeMap<String, String>,
    pub revision: String,
    created_at: i64,
}

/// The Docker and file journal of one transfer.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Transfer {
    pub id: String,
    pub kind: String,
    pub review: Review,
    pub stage: String,
    pub error: Option<String>,
    source_running: bool,
    restart_disabled: bool,
    stopped: bool,
    snapshot: Option<Value>,
    source_state: Option<Value>,
    qualification: Option<Value>,
    production: Option<Value>,
    intent: Option<creation::Intent>,
    pub replacement: Option<String>,
    /// The first production boundary. It is never cleared: production effects already happened.
    boundary_at: Option<i64>,
    /// Every replacement that crossed the boundary, with the appdata generation it ran.
    #[serde(default)]
    activations: Vec<Value>,
    activated: bool,
    committed: bool,
    completed: bool,
    #[serde(default)]
    recovery: Value,
    /// The current or last recovery command.
    #[serde(default)]
    command: Option<Command>,
    #[serde(default)]
    generations: Vec<Value>,
    #[serde(default)]
    original_removed: bool,
    #[serde(default)]
    worker: Option<String>,
    declarations: Value,
    compose_mode: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// One approved recovery, bound to the server's operation identity. Each phase is journaled
/// before its effect, so a repeated submission or a resume continues this command exactly.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Command {
    operation: String,
    action: String,
    approval: Value,
    /// The stage the transfer was in when the command was accepted.
    from: String,
    phase: String,
    /// `running`, `applied` or `failed`.
    state: String,
    error: Option<String>,
    /// The preserved generation for replaced appdata, named before the appdata moves.
    generation: Option<String>,
    started_at: i64,
    finished_at: Option<i64>,
}
impl Command {
    /// Where a command that stopped with an error leaves the transfer. Before any
    /// irreversible step it returns to where it started. A discarded replacement needs a
    /// new decision. A return already under way has no alternative and only resumes.
    fn failure_stage(&self) -> Option<String> {
        let irreversible = match self.action.as_str() {
            "return_original" => matches!(
                self.phase.as_str(),
                "retiring-replacement" | "releasing" | "starting-original"
            ),
            "restore_snapshot" => !matches!(
                self.phase.as_str(),
                "accepted" | "checking-original" | "stopping-replacement"
            ),
            _ => false,
        };
        match (irreversible, self.action.as_str()) {
            (false, _) => Some(self.from.clone()),
            (true, "restore_snapshot") => Some("recovery-required".into()),
            _ => None,
        }
    }
}

fn read(key: &str) -> Result<Transfer> {
    id(key)?;
    let path = directory(key).join("transfer.json");
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "Transfer not found".into()));
    }
    persisted(store::read_secret(&path, &format!("transfer:{key}")))
}
fn write(t: &mut Transfer) -> Result<()> {
    t.updated_at = thelxinoe_core::now();
    persisted(store::write_secret(
        &directory(&t.id).join("transfer.json"),
        &format!("transfer:{}", t.id),
        t,
    ))
}
fn read_review(key: &str) -> Result<Review> {
    id(key)?;
    let path = directory(key).join("review.json");
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "Review not found".into()));
    }
    persisted(store::read_secret(&path, &format!("review:{key}")))
}

/// Unfinished transfers reserve their service kind.
pub(super) fn active_kind(kind: &str) -> Result<bool> {
    let root = store::root().join("transfers");
    if !root.is_dir() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(root).map_err(|_| unavailable())? {
        let name = entry.map_err(|_| unavailable())?.file_name();
        let key = name.to_string_lossy();
        if !directory(&key).join("transfer.json").exists() {
            continue;
        }
        let t = read(&key)?;
        if t.kind == kind && !terminal(&t.stage) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn public_review(r: &Review) -> Value {
    let layout = &r.layout;
    json!({
        "review_id":r.id,"revision":r.revision,"kind":r.kind,"service_id":r.service,"engine":r.engine,
        "source":{"container_id":r.original["Id"],"name":r.original["Name"].as_str().unwrap_or("").trim_start_matches('/'),
            "image":r.source_image,"version":r.source_version.text,"running":r.original["State"]["Running"],
            "restart_policy":r.original["HostConfig"]["RestartPolicy"]["Name"],
            "networks":r.original["NetworkSettings"]["Networks"].as_object().map(|n| n.keys().cloned().collect::<Vec<_>>()),
            "appdata":layout.appdata,"identity":{"uid":r.source_identity.0,"gid":r.source_identity.1}},
        "target":{"image":r.decision.image(),"version":r.target_version.text,"decision":r.decision,
            "appdata":format!("Thelxinoe deployment storage (services/{}/appdata)",r.id),"appdata_destination":"/config",
            "identity":{"uid":r.target_identity.0,"gid":r.target_identity.1},"aliases":r.aliases,"published_port":r.published,
            "mounts":layout.retained,"supplementary":layout.supplementary},
        "inspection":r.inspection,"access":r.access,"changes":r.changes,"settings":r.settings,"recyclarr":r.recyclarr,
        "automation":r.automation,"compose":r.compose,"blockers":r.blockers,"created_at":r.created_at,
    })
}

#[derive(Deserialize)]
pub(super) struct ReviewInput {
    kind: String,
    container_id: String,
    service_id: String,
}
pub(super) async fn review(
    State(runtime): State<Runtime>,
    Json(input): Json<ReviewInput>,
) -> Result<Json<Value>> {
    templates::find(&input.kind).ok_or_else(|| bad("Unknown supported service"))?;
    id(&input.service_id)?;
    if input.container_id.len() != 64 || !input.container_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad("Use the full Docker container ID"));
    }
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    ensure_kind_available(&d, &input.kind).await?;
    let review = inspect(&d, &input, thelxinoe_core::id()).await?;
    persisted(store::write_secret(
        &directory(&review.id).join("review.json"),
        &format!("review:{}", review.id),
        &review,
    ))?;
    Ok(Json(public_review(&review)))
}

/// Read-only validation without a persisted review, for Recyclarr's Connected stage.
pub(super) async fn check(
    State(runtime): State<Runtime>,
    Json(input): Json<ReviewInput>,
) -> Result<Json<Value>> {
    templates::find(&input.kind).ok_or_else(|| bad("Unknown supported service"))?;
    id(&input.service_id)?;
    if input.container_id.len() != 64 || !input.container_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad("Use the full Docker container ID"));
    }
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    let review = inspect(&d, &input, thelxinoe_core::id()).await?;
    Ok(Json(public_review(&review)))
}

/// Name the reviewed components that differ, for a stale start.
fn changed_parts(
    reviewed: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> String {
    let label = |name: &str| match name {
        "container" => "the original container's configuration",
        "image" => "its image or version",
        "storage" => "its storage or another container sharing it",
        "identity" => "its runtime identity",
        "network" => "its network aliases or published port",
        "automation" => "Docker-visible automation or Compose ownership",
        "settings" => "its application settings",
        "recyclarr" => "Recyclarr's targets",
        "blockers" => "the problems the review found",
        _ => "the deployment",
    };
    let changed: Vec<&str> = current
        .iter()
        .filter(|(name, value)| reviewed.get(*name) != Some(value))
        .map(|(name, _)| label(name))
        .collect();
    if changed.is_empty() {
        "the deployment".into()
    } else {
        changed.join(", ")
    }
}

/// Read-only review of the source. Every unsupported feature becomes a concrete blocker.
async fn inspect(d: &Deployment, input: &ReviewInput, key: String) -> Result<Review> {
    let kind = input.kind.as_str();
    let t = templates::find(kind).ok_or_else(unavailable)?;
    let raw = engine(&format!("/containers/{}/json", input.container_id)).await?;
    if raw["Id"] != input.container_id.as_str() {
        return Err(conflict("The reviewed container identity changed"));
    }
    let image = engine(&format!(
        "/images/{}/json",
        raw["Image"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    let mut blockers = Vec::new();
    if image["Os"] != "linux" || image["Architecture"] != "amd64" {
        blockers.push(format!(
            "Transfers support Linux x86-64 images; this container runs {}/{}",
            image["Os"].as_str().unwrap_or("unknown"),
            image["Architecture"].as_str().unwrap_or("unknown")
        ));
    }
    if let Err(reason) = policy::transfer_owner(&raw, true) {
        blockers.push(reason.into());
    }
    let source_image = match policy::source_image(&raw, &image, t) {
        Ok(image) => image,
        Err(reason) => {
            blockers.push(reason.into_owned());
            String::new()
        }
    };
    let source_version = match plan::image_version(&image) {
        Ok(version) => version,
        Err(reason) => {
            blockers.push(reason);
            plan::Version {
                numbers: vec![0],
                build: None,
                text: "unknown".into(),
            }
        }
    };
    let decision = match plan::qualify_image(kind, &source_image, &source_version) {
        Ok(decision) => decision,
        Err(reason) => {
            if blockers.is_empty() {
                blockers.push(reason);
            }
            plan::ImageDecision::Same {
                image: source_image.clone(),
            }
        }
    };
    let target_version = match &decision {
        plan::ImageDecision::Same { .. } => source_version.clone(),
        plan::ImageDecision::Upgrade { image, .. } => {
            let target = crate::docker::ensure_pinned_image(image).await?;
            plan::image_version(&target).map_err(conflict)?
        }
    };
    if target_version.release_cmp(&source_version) == std::cmp::Ordering::Less {
        blockers.push("The qualified image is older than the running application; downgrades are never applied".into());
    }
    let mut others = Vec::new();
    for row in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .take(500)
    {
        let id = row["Id"].as_str().ok_or_else(unavailable)?;
        // This deployment's transfer workers only ever mount a source read-only.
        let worker = row["Labels"]["app.thelxinoe.transfer"].is_string()
            && row["Labels"]["app.thelxinoe.managed-id"].is_null()
            && row["Labels"]["app.thelxinoe.deployment"] == d.id.as_str();
        if id != input.container_id
            && !worker
            && let Ok(other) = engine(&format!("/containers/{id}/json")).await
        {
            others.push(other);
        }
    }
    // A configuration folder must not contain any server media, including secondary mounts.
    let server_media: Vec<String> = d.server["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["Type"] == "bind"
                && m["Destination"]
                    .as_str()
                    .is_some_and(|p| p == "/media" || p.starts_with("/media/"))
        })
        .filter_map(|m| m["Source"].as_str().map(str::to_owned))
        .collect();
    let layout = plan::layout(
        kind,
        &raw,
        &plan::Context {
            media_source: &d.media_source,
            server_media: &server_media,
            appdata_source: &d.appdata_source,
            others: &others,
        },
    );
    let mut layout = match layout {
        Ok(layout) => layout,
        Err(reason) => {
            blockers.push(reason);
            plan::Layout {
                appdata: plan::Mount {
                    kind: "bind".into(),
                    source: String::new(),
                    destination: plan::appdata_destination(kind, &raw),
                    writable: true,
                    role: "appdata".into(),
                    file: false,
                },
                retained: vec![],
                supplementary: vec![],
            }
        }
    };
    if let Some(reason) = plan::boundary(kind, &raw, &image) {
        blockers.push(reason);
    }
    let source_identity = plan::source_identity(kind, &raw, &image).unwrap_or_else(|reason| {
        blockers.push(reason);
        (0, 0)
    });
    let target = Identity::from_user(&d.server["Config"])?;
    let controller = std::env::var("HOSTNAME").unwrap_or_default();
    let automation = plan::automation(&raw, &others, &controller);
    let compose = plan::compose(&raw, &others);
    let aliases = plan::aliases(kind, &raw, &d.network, &others);
    let (published, port_changes) = plan::ports(&raw, t.port);
    if let Some(port) = published
        && let Some(owner) = others.iter().find(|o| {
            o["State"]["Running"] == true
                && o["NetworkSettings"]["Ports"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .flat_map(|(_, b)| b.as_array().into_iter().flatten())
                    .any(|b| {
                        b["HostPort"].as_str().and_then(|p| p.parse::<u16>().ok()) == Some(port)
                    })
        })
    {
        blockers.push(format!(
            "Local port {port} is published by {}; free it or change the original's port before transferring",
            owner["Name"].as_str().unwrap_or("another container").trim_start_matches('/')
        ));
    }
    let mut changes = port_changes;
    let networks: Vec<String> = raw["NetworkSettings"]["Networks"]
        .as_object()
        .map(|n| n.keys().cloned().collect())
        .unwrap_or_default();
    if networks != [d.network.clone()] {
        changes.push(json!({"area":"network","before":networks.join(", "),"after":d.network,
            "reason":"Managed services join only Thelxinoe's media network; peers on other networks keep their own connection settings"}));
    }
    changes.push(json!({"area":"address","before":raw["Name"].as_str().unwrap_or("").trim_start_matches('/'),
        "after":aliases.join(", "),"reason":"The replacement answers on the original container name where no other container uses it"}));
    if let plan::ImageDecision::Upgrade { image, reason } = &decision {
        changes.push(
            json!({"area":"image","before":format!("{source_image} ({})",source_version.text),
            "after":format!("{image} ({})",target_version.text),"reason":reason}),
        );
    }
    if source_identity != (target.uid, target.gid) {
        changes.push(json!({"area":"identity","before":format!("{}:{}",source_identity.0,source_identity.1),
            "after":format!("{}:{}",target.uid,target.gid),
            "reason":"The copied configuration is owned by Thelxinoe's media identity; media files keep their ownership and are checked for access"}));
    }
    changes.push(json!({"area":"mounts","before":format!("{} {} at {}",layout.appdata.kind,layout.appdata.source,layout.appdata.destination),
        "after":format!("Managed copy at /config (services/{key}/appdata)"),
        "reason":"Configuration is copied while the original is stopped; the original folder or volume is preserved unchanged"}));
    for mount in &layout.supplementary {
        changes.push(json!({"area":"mounts","before":format!("{} {} at {}",mount.kind,mount.source,mount.destination),
            "after":format!("Read-only managed copy at {}",mount.destination),
            "reason":"Supplementary files are copied into managed state; the replacement never depends on the external folder"}));
    }
    changes.push(json!({"area":"restart","before":raw["HostConfig"]["RestartPolicy"]["Name"],"after":"Original stays stopped with restart disabled",
        "reason":"The original is kept for explicit recovery and must never run beside its replacement"}));
    // Read-only workers inspect the live configuration and media access.
    let inspection = if layout.appdata.source.is_empty() {
        json!({})
    } else {
        match run_worker(
            d,
            &key,
            "review-inspect",
            vec!["transfer-inspect".into(), kind.into(), "review".into()],
            vec![],
            None,
            &["DAC_READ_SEARCH"],
            vec![source_mount(&layout.appdata, "/state", true)],
            120,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                blockers.push(format!(
                    "The configuration couldn't be inspected: {}",
                    error.1
                ));
                json!({})
            }
        }
    };
    for problem in inspection["blockers"].as_array().into_iter().flatten() {
        blockers.extend(problem.as_str().map(str::to_owned));
    }
    // Supplementary mounts join the access namespace, so references into them resolve and
    // each is probed as a folder or a single file before anything stops.
    let access = if inspection["references"].is_array() || !layout.supplementary.is_empty() {
        let references = if inspection["references"].is_array() {
            inspection["references"].clone()
        } else {
            json!([])
        };
        let supplementary = layout
            .supplementary
            .iter()
            .map(|m| source_mount(m, &m.destination, true))
            .collect();
        match access_check(
            d,
            &key,
            kind,
            &layout,
            (target.uid, target.gid),
            &references,
            supplementary,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                blockers.push(error.1.into_owned());
                json!({})
            }
        }
    } else {
        json!({})
    };
    for problem in access["problems"].as_array().into_iter().flatten() {
        blockers.extend(problem.as_str().map(str::to_owned));
    }
    for mount in &mut layout.supplementary {
        let entry = access["supplementary"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e["destination"] == mount.destination.as_str())
            .map(|e| e["entry"].as_str().unwrap_or_default());
        match entry {
            // A failed access check already names why the mount couldn't be probed.
            None | Some("directory") => {}
            Some("file") => mount.file = true,
            Some(_) => blockers.push(format!(
                "The read-only {} mount is neither a folder nor a regular file; transfers copy only those",
                mount.destination
            )),
        }
    }
    let mut hosts = aliases.clone();
    hosts.push(format!("thelxinoe-{kind}-{key}"));
    let (transformations, settings) =
        settings(kind, &inspection, t, (target.uid, target.gid), &hosts);
    for automation in automation.iter().filter(|a| a.blocks) {
        blockers.push(format!(
            "{} updates containers automatically and could replace or restart the original. Exclude it from {} (label com.centurylinklabs.watchtower.enable=false) and review again",
            automation.name, automation.name
        ));
    }
    let mut recyclarr_targets = Value::Null;
    if blockers.is_empty() && kind == "recyclarr" {
        // The supported Recyclarr import subset is checked before anything stops.
        match recyclarr::review_config(d, &key, &layout.appdata).await {
            Ok(mut configuration) => {
                for target in configuration["targets"]
                    .as_array_mut()
                    .into_iter()
                    .flatten()
                {
                    let secret = target["secret"].as_str().unwrap_or_default().to_owned();
                    target["secret"] = json!(digest(&json!(secret)));
                }
                recyclarr_targets = configuration;
            }
            Err(error) => blockers.push(error.1.into_owned()),
        }
    }
    let evidence_revision = digest(&json!([automation, compose, layout]));
    let parts: BTreeMap<String, String> = [
        ("container", digest(&policy::fingerprint(&raw))),
        (
            "image",
            digest(&json!([source_image, decision, target_version])),
        ),
        ("storage", digest(&json!(layout))),
        (
            "identity",
            digest(&json!([source_identity, (target.uid, target.gid)])),
        ),
        ("network", digest(&json!([aliases, published]))),
        ("automation", evidence_revision.clone()),
        ("settings", digest(&transformations)),
        ("recyclarr", digest(&recyclarr_targets)),
        ("blockers", digest(&json!(blockers))),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect();
    let revision = digest(&json!([
        policy::fingerprint(&raw),
        source_image,
        decision,
        target_version,
        layout,
        source_identity,
        (target.uid, target.gid),
        aliases,
        published,
        evidence_revision,
        transformations,
        recyclarr_targets,
        blockers,
        d.id,
    ]));
    Ok(Review {
        id: key,
        kind: kind.into(),
        service: input.service_id.clone(),
        deployment: d.id.clone(),
        engine: crate::docker::engine_identity().await?,
        original: raw,
        source_image,
        source_version,
        decision,
        target_version,
        layout,
        source_identity,
        target_identity: (target.uid, target.gid),
        aliases,
        published,
        automation,
        compose,
        inspection,
        access,
        changes,
        settings,
        transformations,
        recyclarr: recyclarr_targets,
        blockers,
        evidence_revision,
        parts,
        revision,
        created_at: thelxinoe_core::now(),
    })
}

/// Application settings the transfer changes in the copied configuration only.
fn settings(
    kind: &str,
    inspection: &Value,
    t: templates::Template,
    identity: (u32, u32),
    hosts: &[String],
) -> (Value, Vec<Value>) {
    let config = &inspection["config"];
    let allowed_hosts = allowed_hosts(&config["allowed_hosts"], hosts);
    let mut changes = Vec::new();
    let mut change = |setting: &str, before: Value, after: Value, reason: &str| {
        if before != after {
            changes.push(json!({"setting":setting,"before":before,"after":after,"reason":reason}));
        }
    };
    let base = thelxinoe_core::service_url_base(kind);
    match kind {
        "radarr" | "sonarr" | "lidarr" | "prowlarr" => {
            change(
                "URL Base",
                config["url_base"].clone(),
                json!(base),
                "Thelxinoe opens the service at its managed route",
            );
            change(
                "Port",
                config["port"].clone(),
                json!(t.port),
                "Managed deployments use the service's standard port",
            );
            change(
                "Authentication",
                config["authentication"].clone(),
                json!("External"),
                "Thelxinoe's administrator session protects the managed route; the API key is preserved",
            );
            change(
                "HTTPS",
                config["ssl"].clone(),
                json!(false),
                "Thelxinoe or your reverse proxy terminates HTTPS",
            );
            change(
                "Automatic updates",
                config["update_automatically"].clone(),
                json!(false),
                "Thelxinoe qualifies image updates",
            );
            if let Some(after) = &allowed_hosts {
                change(
                    "Allowed hosts",
                    config["allowed_hosts"].clone(),
                    json!(after),
                    "Thelxinoe and linked services reach the replacement by its managed names",
                );
            }
        }
        "bazarr" => {
            change(
                "URL Base",
                config["url_base"].clone(),
                json!(base),
                "Thelxinoe opens Bazarr at its managed route",
            );
            change(
                "Port",
                config["port"].clone(),
                json!(t.port),
                "Managed deployments use Bazarr's standard port",
            );
            change(
                "Authentication",
                config["authentication"].clone(),
                json!("none"),
                "Thelxinoe's administrator session protects the managed route; the API key is preserved",
            );
        }
        "nzbget" => {
            change(
                "Control port",
                config["port"].clone(),
                json!(t.port),
                "Managed deployments use NZBGet's standard port",
            );
            change(
                "Control address",
                config["address"].clone(),
                json!("0.0.0.0"),
                "The replacement listens on Thelxinoe's private network",
            );
        }
        "seerr" => {
            change(
                "Sign-in",
                config["sign_in"].clone(),
                json!("Thelxinoe accounts only"),
                "Managed Seerr disables its local and media-server sign-in, as fresh installations do",
            );
            change(
                "Availability",
                config["availability"].clone(),
                json!("Radarr and Sonarr"),
                "Requests use Arr-derived availability instead of media-server scans",
            );
            change(
                "Configuration folder",
                config["directory"].clone(),
                json!("/config"),
                "Managed Seerr keeps its state in managed storage",
            );
        }
        "recyclarr" => {
            changes.push(json!({"setting":"Schedule","before":"Image cron (CRON_SCHEDULE)","after":"Thelxinoe schedule, paused until you enable it",
                "reason":"Synchronization never runs during takeover"}));
            changes.push(json!({"setting":"Targets","before":"URLs and API keys in recyclarr.yml","after":"Owned Radarr and Sonarr bindings",
                "reason":"Thelxinoe supplies each target's managed address and credential at synchronization time"}));
        }
        _ => {}
    }
    let plan = json!({"kind":kind,"url_base":base,"port":t.port,"identity":[identity.0,identity.1],
        "allowed_hosts":allowed_hosts});
    (plan, changes)
}

/// Servarr's host filter admits only listed names; keep them and add the managed ones.
fn allowed_hosts(configured: &Value, hosts: &[String]) -> Option<String> {
    let configured = configured
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let mut names: Vec<String> = configured
        .split([';', ','])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if names.iter().any(|name| name == "*") {
        return None;
    }
    for host in hosts {
        if !names.iter().any(|name| name.eq_ignore_ascii_case(host)) {
            names.push(host.clone());
        }
    }
    Some(names.join(";"))
}

fn source_mount(m: &plan::Mount, target: &str, read_only: bool) -> Value {
    let mut mount = json!({"Type":m.kind,"Source":m.source,"Target":target});
    // Docker omits a false ReadOnly when it reports the mount back.
    if read_only {
        mount["ReadOnly"] = json!(true);
    }
    mount
}

/// Run one isolated controller worker without network or Docker access.
#[allow(clippy::too_many_arguments)]
async fn run_worker(
    d: &Deployment,
    key: &str,
    label: &str,
    command: Vec<String>,
    env: Vec<String>,
    user: Option<String>,
    caps: &[&str],
    mounts: Vec<Value>,
    idle_seconds: u64,
) -> Result<Value> {
    run_job(
        d,
        key,
        Job {
            label,
            command,
            env,
            user,
            caps,
            mounts,
            idle_seconds,
            network: "none".into(),
            input: None,
        },
    )
    .await
}
/// One isolated worker run; `input` is written beside its progress report before it starts.
struct Job<'a> {
    label: &'a str,
    command: Vec<String>,
    env: Vec<String>,
    user: Option<String>,
    caps: &'a [&'a str],
    mounts: Vec<Value>,
    idle_seconds: u64,
    network: String,
    input: Option<Value>,
}
async fn run_job(d: &Deployment, key: &str, job: Job<'_>) -> Result<Value> {
    let Job {
        label,
        command,
        env,
        user,
        caps,
        mut mounts,
        idle_seconds,
        network,
        input,
    } = job;
    let name = format!("thelxinoe-transfer-{}-{label}", &key[..8]);
    remove_named(&name).await?;
    let progress = store::root().join("progress").join(&name);
    let _ = std::fs::remove_dir_all(&progress);
    persisted(std::fs::create_dir_all(&progress).map_err(Into::into))?;
    use std::os::unix::fs::PermissionsExt;
    persisted(
        std::fs::set_permissions(&progress, std::fs::Permissions::from_mode(0o777))
            .map_err(Into::into),
    )?;
    if let Some(input) = input {
        persisted(store::write_json(&progress.join("input.json"), &input))?;
        persisted(
            std::fs::set_permissions(
                progress.join("input.json"),
                std::fs::Permissions::from_mode(0o644),
            )
            .map_err(Into::into),
        )?;
    }
    mounts.push(json!({"Type":"bind","Source":format!("{}/progress/{name}",d.appdata_source),"Target":"/progress"}));
    let mut spec = json!({"Image":updates::current_image().await?,"Cmd":command,"Env":env,"Healthcheck":{"Test":["NONE"]},
        "Labels":{"app.thelxinoe.transfer":key,"app.thelxinoe.deployment":d.id},
        "HostConfig":{"NetworkMode":network,"ReadonlyRootfs":true,"CapDrop":["ALL"],"CapAdd":caps,"SecurityOpt":["no-new-privileges:true"],
            "Tmpfs":{"/tmp":"rw,nosuid,nodev,size=1g"},"Mounts":mounts}});
    crate::docker::limit(&mut spec, 1 << 30, 1, 64);
    if let Some(user) = user {
        spec["User"] = json!(user);
    }
    let container = request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(spec),
    )
    .await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let result = async {
        updates::start(&container).await?;
        wait_worker(&container, &progress.join("progress.json"), idle_seconds).await
    }
    .await;
    let report = std::fs::read(progress.join("report.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let removed = updates::remove(&container).await;
    let _ = std::fs::remove_dir_all(&progress);
    result?;
    removed?;
    Ok(report)
}
async fn remove_named(name: &str) -> Result<()> {
    match engine(&format!("/containers/{name}/json")).await {
        Ok(raw) => updates::remove(raw["Id"].as_str().ok_or_else(unavailable)?).await,
        Err((StatusCode::NOT_FOUND, _)) => Ok(()),
        Err(error) => Err(error),
    }
}
/// Workers report progress; only one that stops advancing fails.
async fn wait_worker(container: &str, report: &std::path::Path, idle_seconds: u64) -> Result<()> {
    let mut seen = None;
    let mut advanced = std::time::Instant::now();
    loop {
        let raw = engine(&format!("/containers/{container}/json")).await?;
        if raw["State"]["Running"] == false {
            if raw["State"]["ExitCode"] == 0 {
                return Ok(());
            }
            let reason = std::fs::read(report)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| value["error"].as_str().map(str::to_owned));
            return Err(conflict(reason.unwrap_or_else(|| {
                "Isolated transfer worker failed its contract".into()
            })));
        }
        let current = std::fs::read(report).ok();
        if current.is_some() && current != seen {
            seen = current;
            advanced = std::time::Instant::now();
        }
        if advanced.elapsed() > Duration::from_secs(idle_seconds) {
            return Err(conflict("Isolated transfer worker stopped making progress"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Target-identity access to retained media and every path the configuration references.
/// `supplementary` mounts the read-only supplementary files at their container paths.
async fn access_check(
    d: &Deployment,
    key: &str,
    kind: &str,
    layout: &plan::Layout,
    identity: (u32, u32),
    references: &Value,
    supplementary: Vec<Value>,
) -> Result<Value> {
    let mut mounts = layout
        .retained
        .iter()
        .filter(|m| m.role == "media")
        .map(|m| source_mount(m, &m.destination, true))
        .collect::<Vec<_>>();
    mounts.extend(supplementary);
    let destinations: Vec<&str> = layout
        .supplementary
        .iter()
        .map(|m| m.destination.as_str())
        .collect();
    let groups = if matches!(kind, "seerr" | "recyclarr") {
        vec![identity.1]
    } else {
        // LinuxServer images add their runtime user to the `users` group.
        vec![identity.1, 100]
    };
    run_job(
        d,
        key,
        Job {
            label: "access",
            command: vec!["transfer-access".into()],
            env: vec![],
            user: None,
            caps: &["DAC_READ_SEARCH"],
            mounts,
            idle_seconds: 300,
            network: "none".into(),
            input: Some(
                json!({"references":references,"uid":identity.0,"gid":identity.1,"groups":groups,
                    "supplementary":destinations}),
            ),
        },
    )
    .await
}

#[derive(Deserialize)]
pub(super) struct StartInput {
    revision: String,
    #[serde(default)]
    declarations: Value,
    #[serde(default)]
    compose_mode: Option<String>,
}
/// Persist transfer intent, recheck the reviewed source, then run the transfer in the background.
pub(super) async fn start(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<StartInput>,
) -> Result<Json<Value>> {
    id(&key)?;
    if directory(&key).join("transfer.json").exists() {
        let t = read(&key)?;
        if t.review.revision != input.revision {
            return Err(conflict(
                "This operation already belongs to another reviewed transfer",
            ));
        }
        let active = runtime.0.try_service(&t.kind).is_err();
        return Ok(Json(status(&t, active)));
    }
    let reviewed = read_review(&key)?;
    if reviewed.revision != input.revision {
        return Err(stale("The transfer review changed; review it again"));
    }
    if let Some(blocker) = reviewed.blockers.first() {
        return Err(conflict(blocker.clone()));
    }
    if reviewed.compose.is_some()
        && !matches!(input.compose_mode.as_deref(), Some("scoped" | "project"))
    {
        return Err(conflict(
            "Choose scoped Compose detachment or project-wide retirement",
        ));
    }
    let guard = runtime.0.try_service(&reviewed.kind).map_err(|_| busy())?;
    if !crate::lease::active() {
        return Err(conflict("The controller writer lease is unavailable"));
    }
    let d = bootstrap().await?;
    if reviewed.deployment != d.id {
        return Err(conflict("This review belongs to another deployment"));
    }
    ensure_kind_available(&d, &reviewed.kind).await?;
    // Recheck the reviewed configuration and observable automation immediately before mutation.
    let current = inspect(
        &d,
        &ReviewInput {
            kind: reviewed.kind.clone(),
            container_id: reviewed.original["Id"]
                .as_str()
                .ok_or_else(unavailable)?
                .into(),
            service_id: reviewed.service.clone(),
        },
        reviewed.id.clone(),
    )
    .await?;
    if current.engine != reviewed.engine {
        return Err(stale(
            "The Docker engine changed since the review; review the transfer again",
        ));
    }
    if current.revision != reviewed.revision {
        return Err(stale(format!(
            "The reviewed deployment changed ({}); review the transfer again",
            changed_parts(&reviewed.parts, &current.parts)
        )));
    }
    let now = thelxinoe_core::now();
    let mut t = Transfer {
        id: key.clone(),
        kind: reviewed.kind.clone(),
        source_running: current.original["State"]["Running"] == true,
        review: reviewed,
        stage: "approved".into(),
        error: None,
        restart_disabled: false,
        stopped: false,
        snapshot: None,
        source_state: None,
        qualification: None,
        production: None,
        intent: None,
        replacement: None,
        boundary_at: None,
        activations: vec![],
        activated: false,
        committed: false,
        completed: false,
        recovery: Value::Null,
        command: None,
        generations: vec![],
        original_removed: false,
        worker: None,
        declarations: input.declarations,
        compose_mode: input.compose_mode,
        created_at: now,
        updated_at: now,
    };
    write(&mut t)?;
    let result = status(&t, true);
    crate::recovery::spawn(async move {
        let _guard = guard;
        run(&d, &mut t).await;
    });
    Ok(Json(result))
}

/// Continue an interrupted transfer or recovery command from its journal. A transfer that
/// is already running is reported as active, so the caller keeps observing it.
pub(super) async fn resume(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let resumable = |t: &Transfer| {
        matches!(
            t.stage.as_str(),
            "approved"
                | "stopping"
                | "copying"
                | "validating"
                | "building"
                | "creating"
                | "activating"
                | "recovering"
        )
    };
    let probe = read(&key)?;
    let Ok(guard) = runtime.0.try_service(&probe.kind) else {
        return Ok(Json(status(&probe, true)));
    };
    let mut t = read(&key)?;
    if !resumable(&t) {
        return Ok(Json(status(&t, false)));
    }
    let d = bootstrap().await?;
    let result = status(&t, true);
    crate::recovery::spawn(async move {
        let _guard = guard;
        if t.stage == "recovering" {
            run_command(&d, &mut t).await;
        } else {
            run(&d, &mut t).await;
        }
    });
    Ok(Json(result))
}

async fn run(d: &Deployment, t: &mut Transfer) {
    if let Err(error) = execute(d, t).await {
        t.error = Some(error.1.to_string());
        if error.0 == StatusCode::PRECONDITION_FAILED && !t.restart_disabled {
            t.stage = "stale".into();
        } else if t.boundary_at.is_none() {
            // Before the production boundary the original returns automatically when safe.
            match restore_original(d, t).await {
                Ok(()) => t.stage = "returned".into(),
                Err(restore) => {
                    t.stage = "failed".into();
                    t.error = Some(format!(
                        "{}; restoring the original also failed: {}",
                        error.1, restore.1
                    ));
                }
            }
        } else {
            t.stage = "recovery-required".into();
        }
        t.worker = None;
        if let Err(write_error) = write(t) {
            eprintln!("Transfer journal failed: {}", write_error.1);
        }
    }
}

async fn execute(d: &Deployment, t: &mut Transfer) -> Result<()> {
    let kind = t.kind.clone();
    if t.stage == "approved" {
        let original = source(t).await?;
        if original["State"]["Running"] == true {
            t.source_running = true;
        }
        // Disable restart before stopping so a daemon restart cannot revive the old writer.
        request(
            Method::POST,
            &format!("/containers/{}/update", original_id(t)?),
            Some(json!({"RestartPolicy":{"Name":"no","MaximumRetryCount":0}})),
        )
        .await?;
        t.restart_disabled = true;
        t.stage = "stopping".into();
        write(t)?;
    }
    if t.stage == "stopping" {
        let original = source(t).await?;
        if original["State"]["Running"] == true || original["State"]["Paused"] == true {
            request(
                Method::POST,
                &format!("/containers/{}/stop?t=60", original_id(t)?),
                None,
            )
            .await?;
        }
        let stopped = source(t).await?;
        if stopped["State"]["Running"] != false {
            return Err(conflict("The original did not stop; nothing was copied"));
        }
        t.stopped = true;
        t.stage = "copying".into();
        write(t)?;
    }
    if t.stage == "copying" {
        snapshot(d, t).await?;
        t.stage = "validating".into();
        write(t)?;
    }
    if t.stage == "validating" {
        if kind == "recyclarr" {
            validate_recyclarr(d, t).await?;
        } else {
            validate(d, t).await?;
        }
        t.stage = "building".into();
        write(t)?;
    }
    if t.stage == "building" {
        build(d, t).await?;
        t.stage = "creating".into();
        write(t)?;
    }
    if t.stage == "creating" {
        // Validation can take minutes; anything may have started the original meanwhile.
        original_quiet(t).await?;
        create(d, t).await?;
        // The production boundary is durable before the replacement may reach peers or media.
        cross_boundary(t);
        t.stage = "activating".into();
        write(t)?;
    }
    if t.stage == "activating" {
        activate(d, t).await?;
        t.activated = true;
        t.stage = "activated".into();
        t.worker = None;
        write(t)?;
    }
    Ok(())
}

/// The replacement crosses the production boundary. The first crossing stays the transfer's
/// boundary; each crossing is recorded with the appdata generation the replacement runs.
fn cross_boundary(t: &mut Transfer) {
    let now = thelxinoe_core::now();
    t.boundary_at.get_or_insert(now);
    let replacement = json!(t.replacement);
    if t.activations
        .last()
        .is_none_or(|a| a["replacement"] != replacement)
    {
        t.activations
            .push(json!({"replacement":replacement,"generation":generation(t),"at":now}));
    }
}
/// The identity of the production appdata built from the snapshot. Setup and checks are
/// evidence about this generation; a rebuilt replacement gets a new one.
fn generation(t: &Transfer) -> Value {
    t.production
        .as_ref()
        .map(|p| p["generation"].clone())
        .unwrap_or(Value::Null)
}

fn original_id(t: &Transfer) -> Result<String> {
    Ok(t.review.original["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned())
}
fn frozen(original: &Value) -> Value {
    let mut value = original.clone();
    value["HostConfig"]["RestartPolicy"] = json!({"Name":"no","MaximumRetryCount":0});
    value
}
/// The original, still exactly the reviewed configuration apart from its disabled restart.
async fn source(t: &Transfer) -> Result<Value> {
    let raw = engine(&format!("/containers/{}/json", original_id(t)?)).await?;
    let actual = policy::fingerprint(&raw);
    if actual != policy::fingerprint(&t.review.original)
        && actual != policy::fingerprint(&frozen(&t.review.original))
    {
        return Err(conflict(
            "The original's Docker configuration changed during the transfer; restore its reviewed configuration",
        ));
    }
    Ok(raw)
}

async fn clear(d: &Deployment, t: &Transfer, leaf: &str) -> Result<()> {
    let path = directory(&t.id).join(leaf);
    if path.exists() {
        run_worker(
            d,
            &t.id,
            "clear",
            vec!["appdata-remove".into()],
            vec![],
            None,
            &["DAC_OVERRIDE", "FOWNER"],
            vec![json!({"Type":"bind","Source":host(d,&t.id,leaf),"Target":"/destination"})],
            600,
        )
        .await?;
    }
    persisted(std::fs::create_dir_all(&path).map_err(Into::into))
}

async fn copy(
    d: &Deployment,
    t: &mut Transfer,
    label: &str,
    source: Value,
    destination: String,
) -> Result<()> {
    t.worker = Some(format!("thelxinoe-transfer-{}-{label}", &t.id[..8]));
    write(t)?;
    run_worker(
        d,
        &t.id,
        label,
        vec!["snapshot-copy".into()],
        vec![],
        None,
        &["CHOWN", "FOWNER", "DAC_OVERRIDE"],
        vec![
            source,
            json!({"Type":"bind","Source":destination,"Target":"/destination"}),
        ],
        600,
    )
    .await
    .map(|_| ())
}

async fn tree_digest(d: &Deployment, t: &Transfer, label: &str, source: Value) -> Result<Value> {
    let mut source = source;
    source["Target"] = json!("/state");
    source["ReadOnly"] = json!(true);
    run_worker(
        d,
        &t.id,
        label,
        vec!["transfer-digest".into()],
        vec![],
        None,
        &["DAC_READ_SEARCH"],
        vec![source],
        600,
    )
    .await
}

/// Copy the stopped source into an untouched snapshot and prove the copy complete.
async fn snapshot(d: &Deployment, t: &mut Transfer) -> Result<()> {
    source(t).await.and_then(|raw| {
        if raw["State"]["Running"] == false {
            Ok(raw)
        } else {
            Err(conflict("The original restarted during the transfer"))
        }
    })?;
    let appdata = t.review.layout.appdata.clone();
    let measured = tree_digest(
        d,
        t,
        "source-digest",
        source_mount(&appdata, "/state", true),
    )
    .await?;
    let needed = measured["bytes"]
        .as_u64()
        .unwrap_or(0)
        .saturating_mul(3)
        .saturating_add(64 << 20);
    persisted(std::fs::create_dir_all(directory(&t.id)).map_err(Into::into))?;
    if let Ok(available) = fs2::available_space(directory(&t.id))
        && available < needed
    {
        return Err(conflict(format!(
            "The transfer needs {} MiB free in Thelxinoe's deployment storage for the snapshot, validation and managed copies; {} MiB are available",
            needed >> 20,
            available >> 20
        )));
    }
    clear(d, t, "snapshot").await?;
    copy(
        d,
        t,
        "snapshot",
        source_mount(&appdata, "/source", true),
        host(d, &t.id, "snapshot"),
    )
    .await?;
    let copied = tree_digest(
        d,
        t,
        "snapshot-digest",
        json!({"Type":"bind","Source":host(d,&t.id,"snapshot")}),
    )
    .await?;
    if copied["digest"] != measured["digest"] || copied["digest"].is_null() {
        return Err(conflict(
            "The configuration copy is incomplete; the original is unchanged",
        ));
    }
    let mut supplementary = Vec::new();
    for (index, mount) in t.review.layout.supplementary.clone().iter().enumerate() {
        let leaf = format!("supplementary/{index}");
        clear(d, t, &leaf).await?;
        // A single file is copied into a folder of its own, keeping its mode and owner.
        let source = if mount.file {
            source_mount(mount, &format!("/source/{SUPPLEMENTARY_FILE}"), true)
        } else {
            source_mount(mount, "/source", true)
        };
        copy(d, t, "supplementary", source, host(d, &t.id, &leaf)).await?;
        let copied = tree_digest(
            d,
            t,
            "supplementary-digest",
            json!({"Type":"bind","Source":host(d,&t.id,&leaf)}),
        )
        .await?;
        supplementary.push(json!({"destination":mount.destination,"digest":copied["digest"]}));
    }
    // Revalidate that the original stayed stopped through the copy.
    if source(t).await?["State"]["Running"] != false {
        return Err(conflict("The original restarted during the copy"));
    }
    t.snapshot = Some(
        json!({"digest":copied["digest"],"bytes":copied["bytes"],"files":copied["files"],
        "supplementary":supplementary,"at":thelxinoe_core::now()}),
    );
    Ok(())
}

fn target_env(r: &Review, identity: (u32, u32)) -> Vec<String> {
    let source = plan::env(&r.original);
    let keep = |name: &str| {
        source
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| format!("{name}={v}"))
    };
    let tz = keep("TZ").unwrap_or_else(|| "TZ=UTC".into());
    match r.kind.as_str() {
        "seerr" => [
            Some("CONFIG_DIRECTORY=/config".into()),
            Some(tz),
            keep("LOG_LEVEL"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        "recyclarr" => vec![tz, "TERM=dumb".into()],
        _ => [
            Some(format!("PUID={}", identity.0)),
            Some(format!("PGID={}", identity.1)),
            Some(tz),
            keep("UMASK"),
            keep("UMASK_SET"),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

/// Mounts at the replacement's container paths; `media` replaces retained media for validation.
fn mounts(d: &Deployment, t: &Transfer, appdata: &str, media: Option<&str>) -> Vec<Value> {
    let mut mounts = vec![json!({"Type":"bind","Source":appdata,"Target":"/config"})];
    for (index, m) in t.review.layout.retained.iter().enumerate() {
        match (media, m.role.as_str()) {
            // Validation never sees host time zone files or production media.
            (Some(_), "timezone") => {}
            (Some(fixture), _) => mounts.push(
                json!({"Type":"bind","Source":format!("{fixture}/{index}"),"Target":m.destination}),
            ),
            (None, _) => mounts.push(source_mount(m, &m.destination, !m.writable)),
        }
    }
    mounts.extend(supplementary_mounts(t, appdata));
    let _ = d;
    mounts
}
/// The managed copies of supplementary folders and files, read-only at their container paths.
fn supplementary_mounts(t: &Transfer, appdata: &str) -> Vec<Value> {
    t.review
        .layout
        .supplementary
        .iter()
        .enumerate()
        .map(|(index, m)| {
            let copy = format!("{appdata}/.thelxinoe-supplementary/{index}");
            let source = if m.file {
                format!("{copy}/{SUPPLEMENTARY_FILE}")
            } else {
                copy
            };
            json!({"Type":"bind","Source":source,"Target":m.destination,"ReadOnly":true})
        })
        .collect()
}

/// Copy supplementary files into an appdata tree and apply the reviewed transformations.
async fn prepare(
    d: &Deployment,
    t: &mut Transfer,
    appdata_host: &str,
    appdata_local: &std::path::Path,
    mode: &str,
) -> Result<Value> {
    prepare_with(d, t, appdata_host, appdata_local, mode, vec![]).await
}
async fn prepare_with(
    d: &Deployment,
    t: &mut Transfer,
    appdata_host: &str,
    appdata_local: &std::path::Path,
    mode: &str,
    env: Vec<String>,
) -> Result<Value> {
    let mut env = env;
    env.push(format!(
        "THELXINOE_TRANSFER_PLAN={}",
        t.review.transformations
    ));
    for index in 0..t.review.layout.supplementary.len() {
        persisted(
            std::fs::create_dir_all(
                appdata_local
                    .join(".thelxinoe-supplementary")
                    .join(index.to_string()),
            )
            .map_err(Into::into),
        )?;
        let leaf = format!("{appdata_host}/.thelxinoe-supplementary/{index}");
        copy(
            d,
            t,
            "supplementary-install",
            json!({"Type":"bind","Source":host(d,&t.id,&format!("supplementary/{index}")),"Target":"/source","ReadOnly":true}),
            leaf,
        )
        .await?;
    }
    run_worker(
        d,
        &t.id,
        "prepare",
        vec!["transfer-prepare".into(), t.kind.clone(), mode.into()],
        env,
        None,
        &["CHOWN", "FOWNER", "DAC_OVERRIDE"],
        vec![json!({"Type":"bind","Source":appdata_host,"Target":"/destination"})],
        600,
    )
    .await
}

async fn inspect_state(
    d: &Deployment,
    t: &Transfer,
    label: &str,
    appdata_host: &str,
) -> Result<Value> {
    run_worker(
        d,
        &t.id,
        label,
        vec!["transfer-inspect".into(), t.kind.clone(), "copy".into()],
        vec![],
        None,
        &["DAC_OVERRIDE", "FOWNER"],
        vec![json!({"Type":"bind","Source":appdata_host,"Target":"/state"})],
        600,
    )
    .await
}

async fn cleanup_containers(t: &Transfer, keep: Option<&str>) -> Result<()> {
    for row in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if row["Labels"]["app.thelxinoe.transfer"] == t.id.as_str()
            && row["Labels"]["app.thelxinoe.managed-id"].is_null()
            && keep.is_none_or(|k| row["Id"] != k)
        {
            updates::remove(row["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    Ok(())
}

/// Disposable validation: an independent copy, synthetic media and no route anywhere.
async fn validate(d: &Deployment, t: &mut Transfer) -> Result<()> {
    cleanup_containers(t, None).await?;
    clear(d, t, "validation").await?;
    let appdata = host(d, &t.id, "validation/appdata");
    persisted(
        std::fs::create_dir_all(directory(&t.id).join("validation/appdata")).map_err(Into::into),
    )?;
    copy(
        d,
        t,
        "validation-copy",
        json!({"Type":"bind","Source":host(d,&t.id,"snapshot"),"Target":"/source","ReadOnly":true}),
        appdata.clone(),
    )
    .await?;
    let before = inspect_state(d, t, "inspect-before", &appdata).await?;
    if let Some(problem) = before["blockers"].as_array().and_then(|b| b.first()) {
        return Err(conflict(
            problem
                .as_str()
                .unwrap_or("Unsupported copied configuration")
                .to_owned(),
        ));
    }
    t.source_state = Some(before.clone());
    write(t)?;
    prepare(
        d,
        t,
        &appdata,
        &directory(&t.id).join("validation/appdata"),
        "validation",
    )
    .await?;
    // Imported-file representations keep validation from mistaking missing media for deletions.
    let media = host(d, &t.id, "validation/media");
    persisted(
        std::fs::create_dir_all(directory(&t.id).join("validation/media")).map_err(Into::into),
    )?;
    let destinations: Vec<Value> = t
        .review
        .layout
        .retained
        .iter()
        .map(|m| json!(m.destination))
        .collect();
    let identity = t.review.target_identity;
    run_job(
        d,
        &t.id,
        Job {
            label: "fixture",
            command: vec!["transfer-fixture".into()],
            env: vec![],
            user: None,
            caps: &["CHOWN", "FOWNER", "DAC_OVERRIDE"],
            mounts: vec![json!({"Type":"bind","Source":media,"Target":"/fixture"})],
            idle_seconds: 600,
            network: "none".into(),
            input: Some(json!({"destinations":destinations,"files":before["files"],"uid":identity.0,"gid":identity.1})),
        },
    )
    .await?;
    let image = t.review.decision.image().to_owned();
    crate::docker::ensure_pinned_image(&image).await?;
    let mut spec = json!({"Image":image,"Env":target_env(&t.review, identity),
        "Labels":{"app.thelxinoe.transfer":t.id,"app.thelxinoe.deployment":d.id,"app.thelxinoe.role":"candidate"},
        "Healthcheck":{"Test":["NONE"]},
        "HostConfig":{"NetworkMode":"none","CapDrop":["ALL"],"CapAdd":["CHOWN","DAC_OVERRIDE","FOWNER","SETUID","SETGID","KILL"],
            "SecurityOpt":["no-new-privileges:true"],
            "ExtraHosts":peer_hosts(&t.review),"Mounts":mounts(d,t,&appdata,Some(&media))}});
    crate::docker::limit(&mut spec, 2 << 30, 2, 256);
    if matches!(t.kind.as_str(), "seerr" | "recyclarr") {
        spec["User"] = json!(format!("{}:{}", identity.0, identity.1));
    }
    let name = format!("thelxinoe-transfer-{}-candidate", &t.id[..8]);
    remove_named(&name).await?;
    let candidate = request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(spec),
    )
    .await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let outcome = async {
        let mut checks = Vec::new();
        for pass in ["first-start", "restart"] {
            updates::start(&candidate).await?;
            let report = run_job(
                d,
                &t.id,
                Job {
                    label: "contract",
                    command: vec!["transfer-contract".into(), t.kind.clone(), pass.into()],
                    env: vec![],
                    user: Some(format!("{}:{}", identity.0, identity.1)),
                    caps: &[],
                    mounts: vec![
                        json!({"Type":"bind","Source":appdata,"Target":"/config","ReadOnly":true}),
                    ],
                    idle_seconds: 400,
                    network: format!("container:{candidate}"),
                    input: Some(
                        json!({"remotes":before["remotes"],"peers":t.review.inspection["peers"]}),
                    ),
                },
            )
            .await
            .map_err(|error| conflict(format!("Isolated validation failed: {}", error.1)))?;
            checks.push(json!({"pass":pass,"report":report}));
            updates::stop(&candidate).await?;
        }
        Ok::<_, crate::docker::Failure>(checks)
    };
    let outcome = outcome.await;
    let removed = updates::remove(&candidate).await;
    let checks = outcome?;
    removed?;
    let after = inspect_state(d, t, "inspect-after", &appdata).await?;
    let preservation = compare(&before, &after)?;
    cleanup_containers(t, None).await?;
    clear(d, t, "validation").await?;
    let _ = std::fs::remove_dir(directory(&t.id).join("validation"));
    t.qualification = Some(json!({
        "snapshot_digest":t.snapshot.as_ref().and_then(|s| s.get("digest")).cloned(),
        "image":image,"transformations":digest(&t.review.transformations),
        "versions":{"source":t.review.source_version.text,"target":t.review.target_version.text},
        "formats":{"source":before["format"],"target":after["format"]},
        "checks":checks,"preservation":preservation,"passed":true,"at":thelxinoe_core::now(),
    }));
    Ok(())
}

/// Known peer names resolve only to loopback fixtures inside the isolated namespace.
fn peer_hosts(r: &Review) -> Vec<String> {
    let mut hosts: Vec<String> = [
        "radarr", "sonarr", "lidarr", "nzbget", "prowlarr", "bazarr", "seerr",
    ]
    .iter()
    .flat_map(|kind| [kind.to_string(), format!("thelxinoe-{kind}")])
    .map(|name| format!("{name}:127.0.0.1"))
    .collect();
    for name in r.inspection["peers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && name.parse::<std::net::IpAddr>().is_err()
        {
            hosts.push(format!("{name}:127.0.0.1"));
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

/// Preserved record identities must survive validation; application-owned changes are named.
fn compare(before: &Value, after: &Value) -> Result<Value> {
    let mut tables = Vec::new();
    let schema_changed = before["format"] != after["format"];
    for (name, table) in before["records"].as_object().into_iter().flatten() {
        let later = &after["records"][name];
        if later.is_null() {
            return Err(conflict(format!("Validation lost the {name} records")));
        }
        let missing = table["ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|id| !later["ids"].as_array().is_some_and(|ids| ids.contains(id)))
            .count();
        if missing > 0 {
            return Err(conflict(format!(
                "Validation removed {missing} preserved {name} records; the copied state isn't preserved"
            )));
        }
        let changed = table["digest"] != later["digest"];
        if changed && table["strict"] == true && !schema_changed {
            return Err(conflict(format!(
                "Validation changed preserved {name} records without an application upgrade"
            )));
        }
        tables.push(json!({"table":name,"count":table["count"],"preserved":true,
            "changed":changed,"reason":if changed {json!(if schema_changed {"Application-owned schema upgrade"} else {"Application runtime state"})} else {Value::Null}}));
    }
    Ok(json!({"tables":tables,"schema_changed":schema_changed}))
}

/// Recyclarr qualification: its copied configuration synchronizes only synthetic targets.
async fn validate_recyclarr(d: &Deployment, t: &mut Transfer) -> Result<()> {
    cleanup_containers(t, None).await?;
    clear(d, t, "validation").await?;
    let appdata = host(d, &t.id, "validation/appdata");
    persisted(
        std::fs::create_dir_all(directory(&t.id).join("validation/appdata")).map_err(Into::into),
    )?;
    copy(
        d,
        t,
        "validation-copy",
        json!({"Type":"bind","Source":host(d,&t.id,"snapshot"),"Target":"/source","ReadOnly":true}),
        appdata.clone(),
    )
    .await?;
    let before = inspect_state(d, t, "inspect-before", &appdata).await?;
    if let Some(problem) = before["blockers"].as_array().and_then(|b| b.first()) {
        return Err(conflict(
            problem
                .as_str()
                .unwrap_or("Unsupported Recyclarr configuration")
                .to_owned(),
        ));
    }
    t.source_state = Some(before.clone());
    write(t)?;
    let image = t.review.decision.image().to_owned();
    crate::docker::ensure_pinned_image(&image).await?;
    let secret = thelxinoe_core::id().replace('-', "");
    let prepared = prepare_with(
        d,
        t,
        &appdata,
        &directory(&t.id).join("validation/appdata"),
        "validation",
        vec![
            format!("THELXINOE_TRANSFER_FIXTURE_KEY={secret}"),
            // Copied guide resources replace any network fetch during validation.
            format!(
                "THELXINOE_RECYCLARR_SETTINGS={}",
                recyclarr::local_settings()
            ),
        ],
    )
    .await?;
    let fixtures = prepared["fixtures"].as_array().cloned().unwrap_or_default();
    // Preparation points the copy at fixtures; synchronization must not change it further.
    let prepared_state = inspect_state(d, t, "inspect-prepared", &appdata).await?;
    let evidence = recyclarr::qualify_transfer(
        d,
        &t.id,
        &image,
        &directory(&t.id).join("validation"),
        &host(d, &t.id, "validation"),
        &fixtures,
        &secret,
        t.review.target_identity,
    )
    .await?;
    let after = inspect_state(d, t, "inspect-after", &appdata).await?;
    let preservation = compare(&prepared_state, &after)?;
    cleanup_containers(t, None).await?;
    clear(d, t, "validation").await?;
    let _ = std::fs::remove_dir(directory(&t.id).join("validation"));
    t.qualification = Some(json!({
        "snapshot_digest":t.snapshot.as_ref().and_then(|s| s.get("digest")).cloned(),
        "image":image,"transformations":digest(&t.review.transformations),
        "versions":{"source":t.review.source_version.text,"target":t.review.target_version.text},
        "formats":{"source":before["format"],"target":after["format"]},
        "checks":[evidence],"preservation":preservation,"passed":true,"at":thelxinoe_core::now(),
    }));
    Ok(())
}

/// Production appdata comes from the untouched snapshot plus approved transformations only.
async fn build(d: &Deployment, t: &mut Transfer) -> Result<()> {
    let qualification = t
        .qualification
        .clone()
        .ok_or_else(|| conflict("The transfer has no qualification"))?;
    let snapshot = tree_digest(
        d,
        t,
        "snapshot-verify",
        json!({"Type":"bind","Source":host(d,&t.id,"snapshot")}),
    )
    .await?;
    if snapshot["digest"] != qualification["snapshot_digest"]
        || qualification["image"] != t.review.decision.image()
        || qualification["transformations"] != digest(&t.review.transformations)
    {
        return Err(conflict(
            "The snapshot, image or transformations changed after validation; qualification is invalid",
        ));
    }
    let appdata = format!("{}/services/{}/appdata", d.appdata_source, t.id);
    let local = service_path(&t.id).with_file_name("appdata");
    if local.exists() {
        // Only a build interrupted before creation can leave appdata here.
        run_worker(
            d,
            &t.id,
            "clear",
            vec!["appdata-remove".into()],
            vec![],
            None,
            &["DAC_OVERRIDE", "FOWNER"],
            vec![json!({"Type":"bind","Source":appdata,"Target":"/destination"})],
            600,
        )
        .await?;
    }
    persisted(std::fs::create_dir_all(&local).map_err(Into::into))?;
    copy(
        d,
        t,
        "production-copy",
        json!({"Type":"bind","Source":host(d,&t.id,"snapshot"),"Target":"/source","ReadOnly":true}),
        appdata.clone(),
    )
    .await?;
    let prepared = prepare(d, t, &appdata, &local, "production").await?;
    let produced = tree_digest(
        d,
        t,
        "production-digest",
        json!({"Type":"bind","Source":appdata}),
    )
    .await?;
    let references = t
        .source_state
        .as_ref()
        .map(|s| s["references"].clone())
        .unwrap_or(json!([]));
    // The replacement reaches its managed supplementary copies, not the external originals.
    let access = access_check(
        d,
        &t.id,
        &t.kind,
        &t.review.layout,
        t.review.target_identity,
        &references,
        supplementary_mounts(t, &appdata),
    )
    .await?;
    if let Some(problem) = access["problems"].as_array().and_then(|p| p.first()) {
        return Err(conflict(
            problem
                .as_str()
                .unwrap_or("Media access check failed")
                .to_owned(),
        ));
    }
    t.production = Some(
        json!({"digest":produced["digest"],"generation":thelxinoe_core::id(),"prepared":prepared,
            "access":access,"at":thelxinoe_core::now()}),
    );
    Ok(())
}

fn target_spec(d: &Deployment, t: &Transfer, name: &str) -> Result<Value> {
    let template = templates::find(&t.kind).ok_or_else(unavailable)?;
    let identity = t.review.target_identity;
    let appdata = format!("{}/services/{}/appdata", d.appdata_source, t.id);
    let mut spec = json!({"Image":t.review.decision.image(),"Env":target_env(&t.review, identity),
        "Labels":{"app.thelxinoe.managed-id":t.id,"app.thelxinoe.deployment":d.id,"app.thelxinoe.kind":t.kind,"app.thelxinoe.transfer":t.id},
        "HostConfig":{"Mounts":mounts(d,t,&appdata,None),"NetworkMode":d.network,"RestartPolicy":{"Name":"unless-stopped"}},
        "NetworkingConfig":{"EndpointsConfig":{d.network.clone():{"Aliases":t.review.aliases}}}});
    if let (Some(port), Some(published)) = (template.port, t.review.published) {
        spec["HostConfig"]["PortBindings"] = json!({format!("{port}/tcp"):[{"HostIp":"127.0.0.1","HostPort":published.to_string()}]});
    }
    if matches!(t.kind.as_str(), "seerr" | "recyclarr") {
        spec["User"] = json!(format!("{}:{}", identity.0, identity.1));
    }
    if t.kind == "recyclarr" {
        spec["Cmd"] = json!(["--version"]);
        spec["HostConfig"]["RestartPolicy"] = json!({"Name":"no"});
        spec["HostConfig"]["CapDrop"] = json!(["ALL"]);
        spec["HostConfig"]["SecurityOpt"] = json!(["no-new-privileges:true"]);
    }
    let _ = name;
    Ok(spec)
}

/// Persist creation intent before Docker create; never create a second replacement.
async fn create(d: &Deployment, t: &mut Transfer) -> Result<()> {
    let containers = engine("/containers/json?all=true").await?;
    let intent =
        match &t.intent {
            Some(intent) => intent.clone(),
            None => {
                let name = format!("thelxinoe-{}-{}", t.kind, t.id);
                if containers.as_array().into_iter().flatten().any(|row| {
                    row["Names"].as_array().into_iter().flatten().any(|n| {
                        n.as_str().map(|n| n.trim_start_matches('/')) == Some(name.as_str())
                    }) && row["Labels"]["app.thelxinoe.managed-id"] != t.id.as_str()
                }) {
                    return Err(conflict("Managed container name is already in use"));
                }
                let mut s = Managed {
                    id: t.id.clone(),
                    kind: t.kind.clone(),
                    container: String::new(),
                    name: name.clone(),
                    image: t.review.decision.image().to_owned(),
                    phase: "transferring".into(),
                    active_update: None,
                    spec: target_spec(d, t, &name)?,
                    expected: Value::Null,
                    hold: Some("transfer".into()),
                    origin: "transferred".into(),
                    transfer: Some(t.id.clone()),
                    error: None,
                };
                s.spec["Labels"][creation::ATTEMPT_LABEL] = json!(thelxinoe_core::id());
                let intent = creation::Intent::recorded(d, &s);
                t.intent = Some(intent.clone());
                write(t)?;
                save(&s)?;
                intent
            }
        };
    crate::docker::ensure_pinned_image(t.review.decision.image()).await?;
    let container = intent.create(d).await?;
    let raw = engine(&format!("/containers/{container}/json")).await?;
    verify_recorded(d, &intent.service, &raw).await?;
    let mut s = load(&t.id)?;
    s.container = container.clone();
    save(&s)?;
    t.replacement = Some(container);
    write(t)
}

/// Start the replacement after the boundary and prove authenticated API readiness only.
/// Every path that starts a replacement comes through here, so the single-writer check does too.
async fn activate(d: &Deployment, t: &mut Transfer) -> Result<()> {
    original_quiet(t).await?;
    let mut s = load(&t.id)?;
    let container = t.replacement.clone().ok_or_else(unavailable)?;
    let raw = engine(&format!("/containers/{container}/json")).await?;
    verify_ownership(d, &s, &raw).await?;
    if t.kind != "recyclarr" {
        if raw["State"]["Running"] != true {
            updates::start(&container).await?;
        }
        health(d, t, &container).await?;
    }
    let raw = engine(&format!("/containers/{container}/json")).await?;
    s.expected = policy::fingerprint(&raw);
    s.phase = "active".into();
    s.hold = Some("transfer".into());
    s.error = None;
    save(&s)
}

/// Read-only authenticated health of the exact replacement, from inside its namespace.
async fn health(d: &Deployment, t: &Transfer, container: &str) -> Result<()> {
    let identity = t.review.target_identity;
    let appdata = format!("{}/services/{}/appdata", d.appdata_source, t.id);
    let mut checker = json!({"Image":updates::current_image().await?,"User":format!("{}:{}",identity.0,identity.1),
        "Cmd":["adapter-health",t.kind],"Healthcheck":{"Test":["NONE"]},
        "Labels":{"app.thelxinoe.transfer":t.id,"app.thelxinoe.deployment":d.id},
        "HostConfig":{"NetworkMode":format!("container:{container}"),"ReadonlyRootfs":true,"CapDrop":["ALL"],
            "SecurityOpt":["no-new-privileges:true"],
            "Mounts":[{"Type":"bind","Source":appdata,"Target":"/config","ReadOnly":true}]}});
    crate::docker::limit(&mut checker, 256 << 20, 1, 32);
    let name = format!("thelxinoe-transfer-{}-health", &t.id[..8]);
    remove_named(&name).await?;
    let checker = request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(checker),
    )
    .await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let outcome = async {
        updates::start(&checker).await?;
        updates::wait(&checker, 300).await
    }
    .await;
    let removed = updates::remove(&checker).await;
    outcome.map_err(|_| {
        conflict("The replacement did not become ready with its preserved credentials")
    })?;
    removed
}

/// Undo a transfer that never crossed the production boundary.
async fn restore_original(d: &Deployment, t: &mut Transfer) -> Result<()> {
    if t.boundary_at.is_some() {
        return Err(conflict(
            "Production activation requires an explicit recovery plan",
        ));
    }
    cleanup_containers(t, None).await?;
    // Find our own creation attempt by its labels, even when its spec failed verification.
    let attempt = t
        .intent
        .as_ref()
        .and_then(|intent| intent.service.spec["Labels"][creation::ATTEMPT_LABEL].as_str())
        .map(str::to_owned);
    let rows = engine("/containers/json?all=true").await?;
    for row in rows.as_array().into_iter().flatten().filter(|row| {
        attempt.is_some()
            && row["Labels"][creation::ATTEMPT_LABEL].as_str() == attempt.as_deref()
            && row["Labels"]["app.thelxinoe.deployment"] == d.id
            && row["Labels"]["app.thelxinoe.managed-id"] == t.id.as_str()
    }) {
        let container = row["Id"].as_str().ok_or_else(unavailable)?.to_owned();
        let raw = engine(&format!("/containers/{container}/json")).await?;
        if raw["State"]["Running"] != false
            || raw["State"]["StartedAt"]
                .as_str()
                .is_none_or(|time| !time.starts_with("0001-01-01"))
        {
            return Err(conflict(
                "The replacement may have run; explicit recovery is required",
            ));
        }
        updates::remove(&container).await?;
    }
    if let Ok(mut s) = load(&t.id) {
        s.phase = "returned".into();
        s.hold = None;
        save(&s)?;
    }
    if t.restart_disabled {
        let raw = source(t).await?;
        request(
            Method::POST,
            &format!("/containers/{}/update", original_id(t)?),
            Some(json!({"RestartPolicy":t.review.original["HostConfig"]["RestartPolicy"]})),
        )
        .await?;
        if t.source_running && raw["State"]["Running"] != true {
            updates::start(&original_id(t)?).await?;
        }
    }
    Ok(())
}

pub(super) fn status(t: &Transfer, active: bool) -> Value {
    let progress = t.worker.as_ref().and_then(|name| {
        std::fs::read(
            store::root()
                .join("progress")
                .join(name)
                .join("progress.json"),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    });
    json!({"id":t.id,"kind":t.kind,"stage":t.stage,"error":t.error,"revision":t.review.revision,
        "source":{"container_id":t.review.original["Id"],"running_before":t.source_running,"restart_disabled":t.restart_disabled,"stopped":t.stopped,
            "appdata":t.review.layout.appdata,"removed":t.original_removed},
        "snapshot":t.snapshot,"qualification":t.qualification,
        "production":t.production.as_ref().map(|p| json!({"digest":p["digest"],"generation":p["generation"],"access":p["access"],"at":p["at"]})),
        "replacement":t.replacement,"boundary_at":t.boundary_at,"activations":t.activations,"activated":t.activated,"committed":t.committed,"completed":t.completed,
        "recovery":t.recovery,"command":t.command,"generations":t.generations,"progress":progress,"operation_active":active,"updated_at":t.updated_at,
        // Preserved identities are needed only for post-activation checks.
        "source_records":if t.boundary_at.is_some() {t.source_state.as_ref().map(|s| s["records"].clone())} else {None}})
}

pub(super) async fn get(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let t = read(&key)?;
    let active = runtime.0.try_service(&t.kind).is_err();
    Ok(Json(status(&t, active)))
}

#[derive(Deserialize)]
pub(super) struct Commit {
    container_id: String,
}
/// The server committed ownership of this exact replacement; record agreement.
pub(super) async fn commit(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Commit>,
) -> Result<Json<Value>> {
    let probe = read(&key)?;
    let _guard = runtime.0.service(&probe.kind).await;
    let mut t = read(&key)?;
    if terminal(&t.stage) || t.stage == "recovering" {
        return Err(conflict(
            "This transfer can't commit a replacement in its current stage",
        ));
    }
    if !t.activated || t.replacement.as_deref() != Some(input.container_id.as_str()) {
        return Err(conflict("Only the activated replacement can be committed"));
    }
    if !t.committed {
        t.committed = true;
        if t.stage == "activated" {
            t.stage = "committed".into();
        }
        write(&mut t)?;
    }
    Ok(Json(status(&t, false)))
}

/// Transfer completion: the controller releases its operational hold. Repeating it after a
/// lost reply reports the recorded completion.
pub(super) async fn complete(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let probe = read(&key)?;
    let _guard = runtime.0.service(&probe.kind).await;
    let mut t = read(&key)?;
    if t.completed {
        return Ok(Json(status(&t, false)));
    }
    if terminal(&t.stage) || t.stage == "recovering" {
        return Err(conflict(
            "This transfer can't complete in its current stage",
        ));
    }
    if !t.committed {
        return Err(conflict(
            "Ownership must be committed before the transfer completes",
        ));
    }
    let mut s = load(&t.id)?;
    let d = bootstrap().await?;
    let raw = engine(&format!("/containers/{}/json", s.container)).await?;
    verify_ownership(&d, &s, &raw).await?;
    s.hold = None;
    s.phase = "active".into();
    save(&s)?;
    t.completed = true;
    t.stage = "complete".into();
    t.recovery = Value::Null;
    write(&mut t)?;
    Ok(Json(status(&t, false)))
}

/// Cancel only before any source mutation.
pub(super) async fn cancel(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    if !directory(&key).join("transfer.json").exists() {
        read_review(&key)?;
        persisted(std::fs::remove_dir_all(directory(&key)).map_err(Into::into))?;
        return Ok(Json(json!({"id":key,"stage":"cancelled"})));
    }
    let probe = read(&key)?;
    let _guard = runtime.0.try_service(&probe.kind).map_err(|_| {
        conflict("The transfer is running; it can be cancelled only before the original changes")
    })?;
    let mut t = read(&key)?;
    if t.stage == "cancelled" {
        return Ok(Json(status(&t, false)));
    }
    if t.stage != "approved" || t.restart_disabled {
        return Err(conflict(
            "The original already changed; use Restore original or recovery instead",
        ));
    }
    t.stage = "cancelled".into();
    write(&mut t)?;
    Ok(Json(status(&t, false)))
}

/// Restore the original after a failed transfer that never reached production.
pub(super) async fn restore(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let probe = read(&key)?;
    let _guard = runtime.0.try_service(&probe.kind).map_err(|_| busy())?;
    let d = bootstrap().await?;
    let mut t = read(&key)?;
    if t.stage == "returned" {
        return Ok(Json(status(&t, false)));
    }
    if t.boundary_at.is_some() || !matches!(t.stage.as_str(), "failed" | "stale") {
        return Err(conflict(
            "Only a failed transfer before production activation restores automatically",
        ));
    }
    restore_original(&d, &mut t).await?;
    t.stage = "returned".into();
    write(&mut t)?;
    Ok(Json(status(&t, false)))
}

/// Recovery applies after the production boundary to a transfer that hasn't ended and
/// isn't already running a recovery command.
fn plannable(t: &Transfer) -> Result<()> {
    if t.boundary_at.is_none() || t.completed {
        return Err(conflict(
            "Recovery plans apply to activated transfers that are not complete",
        ));
    }
    match t.stage.as_str() {
        "returned" => Err(conflict(
            "This transfer already returned to the original; nothing remains to recover",
        )),
        "recovering" => Err(conflict(
            "A recovery is already running; it continues from its journal",
        )),
        stage if terminal(stage) => Err(conflict(
            "This transfer already ended; nothing remains to recover",
        )),
        _ => Ok(()),
    }
}

/// A replacement may start only while the original can't write: stopped with its restart
/// disabled, or no longer present.
async fn original_quiet(t: &Transfer) -> Result<()> {
    match engine(&format!("/containers/{}/json", original_id(t)?)).await {
        Ok(raw) => {
            if raw["State"]["Running"] != false || raw["State"]["Restarting"] == true {
                return Err(conflict(
                    "The original is running; a replacement never starts beside it",
                ));
            }
            if raw["HostConfig"]["RestartPolicy"]["Name"]
                .as_str()
                .is_some_and(|name| !matches!(name, "" | "no"))
            {
                return Err(conflict(
                    "The original's restart policy is enabled again; disable it before a replacement starts",
                ));
            }
            Ok(())
        }
        Err((StatusCode::NOT_FOUND, _)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Evidence and options for a transfer that crossed the production boundary.
pub(super) async fn recovery_plan(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let probe = read(&key)?;
    let _guard = runtime.0.try_service(&probe.kind).map_err(|_| busy())?;
    let d = bootstrap().await?;
    let mut t = read(&key)?;
    plannable(&t)?;
    let s = load(&t.id)?;
    let replacement = if s.container.is_empty() {
        json!({"present":false})
    } else {
        match engine(&format!("/containers/{}/json", s.container)).await {
            Ok(raw) => {
                json!({"id":s.container,"present":true,"running":raw["State"]["Running"],"started_at":raw["State"]["StartedAt"],"verified":verify_ownership(&d,&s,&raw).await.is_ok()})
            }
            Err((StatusCode::NOT_FOUND, _)) => json!({"id":s.container,"present":false}),
            Err(error) => return Err(error),
        }
    };
    let original = match engine(&format!("/containers/{}/json", original_id(&t)?)).await {
        Ok(raw) => json!({"id":raw["Id"],"present":true,"running":raw["State"]["Running"],
            "unchanged":policy::fingerprint(&raw)==policy::fingerprint(&frozen(&t.review.original))}),
        Err((StatusCode::NOT_FOUND, _)) => json!({"present":false}),
        Err(error) => return Err(error),
    };
    // Single writer: nothing that starts a replacement is offered while the original can write.
    let quiet = original_quiet(&t).await.err().map(|error| error.1);
    let appdata = format!("{}/services/{}/appdata", d.appdata_source, t.id);
    let effects = run_worker(
        &d,
        &t.id,
        "effects",
        vec!["transfer-effects".into(), t.kind.clone()],
        vec![format!("THELXINOE_TRANSFER_BOUNDARY={}", t.boundary_at.unwrap_or_default())],
        None,
        &["DAC_READ_SEARCH"],
        vec![json!({"Type":"bind","Source":appdata,"Target":"/state","ReadOnly":true})],
        300,
    )
    .await
    .unwrap_or_else(|error| json!({"known":[],"uncertain":[format!("Live effects couldn't be inventoried: {}",error.1)]}));
    let current_evidence = digest(&json!([
        t.review.automation,
        t.review.compose,
        t.review.layout
    ]));
    let evidence = json!({"images":{"source":t.review.source_image,"replacement":t.review.decision.image()},
        "versions":{"source":t.review.source_version.text,"replacement":t.review.target_version.text},
        "generations":{"source_appdata":t.review.layout.appdata,"snapshot":t.snapshot.as_ref().map(|s| &s["digest"]),
            "activated":t.production.as_ref().map(|p| &p["digest"]),"preserved":t.generations,"activations":t.activations},
        "boundary_at":t.boundary_at,"containers":{"replacement":replacement,"original":original},"effects":effects,
        "automation_revision":current_evidence});
    let cannot_undo = [
        "Downloads, imports and file changes the replacement made",
        "Notifications or webhooks it sent",
        "Changes it or Thelxinoe made to peer services",
        "Requests submitted since activation",
    ];
    let starts = |available: bool, missing: &str| match (&quiet, available) {
        (_, false) => (false, Some(missing.to_owned())),
        (Some(reason), true) => (false, Some(reason.to_string())),
        (None, true) => (true, None),
    };
    let (repair, repair_reason) = starts(
        replacement["present"] == true,
        "The replacement container no longer exists",
    );
    let (snapshot, snapshot_reason) = starts(t.snapshot.is_some(), "The transfer has no snapshot");
    let back = original["present"] == true && original["unchanged"] == true;
    let options = json!([
        {"action":"repair","label":"Repair and continue","available":repair,"unavailable":repair_reason,
            "description":"Keep the replacement's latest appdata, repeat the required checks and finish the transfer when healthy"},
        {"action":"restore_snapshot","label":"Restore the transfer snapshot into a new candidate","available":snapshot,"unavailable":snapshot_reason,
            "description":"Preserve the current appdata as a generation and activate an independent replacement built from the snapshot taken when the original stopped. Reviewed setup runs again for the new appdata",
            "requires":["state_loss","reconciliation"]},
        {"action":"return_original","label":"Return to the original","available":back,
            "unavailable":if back {Value::Null} else {json!("The original container is missing or its configuration changed")},
            "description":"Stop and remove the replacement, keeping its appdata as a generation, then restart the original. The service ends Connected with this failed transfer recorded",
            "requires":["state_loss","reconciliation","automation"]},
        {"action":"suspend","label":"Suspend for manual resolution","available":true,
            "description":"Keep normal operations blocked and preserve all data and evidence"}
    ]);
    let revision = digest(&json!([evidence, options]));
    t.recovery = json!({"revision":revision,"evidence":evidence,"options":options,"cannot_undo":cannot_undo,"planned_at":thelxinoe_core::now()});
    write(&mut t)?;
    Ok(Json(t.recovery.clone()))
}

#[derive(Deserialize)]
pub(super) struct Recover {
    /// The server's identity for this command; a repeat with it continues the same command.
    operation: String,
    action: String,
    revision: String,
    #[serde(default)]
    approval: Value,
}
/// Accept one approved recovery command and run it in the background from its journal.
/// Acknowledgements never bypass identity or single-writer checks.
pub(super) async fn recover(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Recover>,
) -> Result<Json<Value>> {
    id(&input.operation)?;
    let probe = read(&key)?;
    if probe
        .command
        .as_ref()
        .is_some_and(|c| c.operation == input.operation)
    {
        // A repeated submission after a lost reply: report or continue the same command.
        return resume(State(runtime), Path(key)).await;
    }
    let guard = runtime.0.try_service(&probe.kind).map_err(|_| busy())?;
    if !crate::lease::active() {
        return Err(conflict("The controller writer lease is unavailable"));
    }
    let d = bootstrap().await?;
    let mut t = read(&key)?;
    plannable(&t)?;
    if t.recovery["revision"] != input.revision.as_str() {
        return Err(stale(
            "The recovery evidence changed; review the plan again",
        ));
    }
    let option = t.recovery["options"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["action"] == input.action.as_str())
        .cloned()
        .ok_or_else(|| bad("Unknown recovery action"))?;
    if option["available"] != true {
        return Err(conflict(
            "This recovery action isn't available for the recorded state",
        ));
    }
    for required in option["requires"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if input.approval[required] != true {
            return Err(conflict(
                "Approve the listed consequences before this recovery",
            ));
        }
    }
    // The command is durable before any effect; from here on it only continues.
    t.command = Some(Command {
        operation: input.operation,
        action: input.action,
        approval: input.approval,
        from: t.stage.clone(),
        phase: "accepted".into(),
        state: "running".into(),
        error: None,
        generation: None,
        started_at: thelxinoe_core::now(),
        finished_at: None,
    });
    t.stage = "recovering".into();
    t.error = None;
    write(&mut t)?;
    let result = status(&t, true);
    crate::recovery::spawn(async move {
        let _guard = guard;
        run_command(&d, &mut t).await;
    });
    Ok(Json(result))
}

/// Run the journaled command to its outcome. An error after a return to the original began
/// leaves it running, to be resumed; otherwise it fails and the transfer stage says why.
async fn run_command(d: &Deployment, t: &mut Transfer) {
    let result = execute_command(d, t).await;
    let Some(c) = t.command.as_mut() else {
        return;
    };
    let now = thelxinoe_core::now();
    match result {
        Ok(()) => {
            c.state = "applied".into();
            c.error = None;
            c.finished_at = Some(now);
            t.error = None;
        }
        Err(error) => {
            let message = error.1.to_string();
            c.error = Some(message.clone());
            if let Some(stage) = c.failure_stage() {
                c.state = "failed".into();
                c.finished_at = Some(now);
                t.stage = stage;
                t.error = Some(message);
            }
        }
    }
    t.worker = None;
    if let Err(error) = write(t) {
        eprintln!("Transfer journal failed: {}", error.1);
    }
}

fn phase(t: &Transfer) -> String {
    t.command
        .as_ref()
        .map(|c| c.phase.clone())
        .unwrap_or_default()
}
/// Journal the next phase before its effect.
fn advance(t: &mut Transfer, phase: &str) -> Result<()> {
    t.command.as_mut().ok_or_else(unavailable)?.phase = phase.into();
    write(t)
}

async fn execute_command(d: &Deployment, t: &mut Transfer) -> Result<()> {
    let action = t
        .command
        .as_ref()
        .map(|c| c.action.clone())
        .ok_or_else(unavailable)?;
    match action.as_str() {
        "suspend" => {
            t.stage = "suspended".into();
            Ok(())
        }
        "repair" => repair(d, t).await,
        "return_original" => return_original(d, t).await,
        "restore_snapshot" => restore_snapshot(d, t).await,
        _ => Err(bad("Unknown recovery action")),
    }
}

/// Start the retained replacement again over its latest appdata.
async fn repair(d: &Deployment, t: &mut Transfer) -> Result<()> {
    if phase(t) == "accepted" {
        advance(t, "starting")?;
    }
    original_quiet(t).await?;
    let mut s = load(&t.id)?;
    let raw = engine(&format!("/containers/{}/json", s.container)).await?;
    verify_ownership(d, &s, &raw).await?;
    if t.kind != "recyclarr" {
        if raw["State"]["Running"] != true {
            updates::start(&s.container).await?;
        }
        health(d, t, &s.container).await?;
    }
    s.expected = policy::fingerprint(&engine(&format!("/containers/{}/json", s.container)).await?);
    s.phase = "active".into();
    save(&s)?;
    t.activated = true;
    t.stage = if t.committed {
        "committed"
    } else {
        "activated"
    }
    .into();
    Ok(())
}

/// Single writer: the replacement stops and is retired before the original starts again.
async fn return_original(d: &Deployment, t: &mut Transfer) -> Result<()> {
    if phase(t) == "accepted" {
        advance(t, "stopping-replacement")?;
    }
    if phase(t) == "stopping-replacement" {
        stop_replacement(d, &load(&t.id)?).await?;
        advance(t, "checking-original")?;
    }
    if phase(t) == "checking-original" {
        let raw = source(t).await?;
        if raw["State"]["Running"] != false {
            return Err(conflict(
                "The original is running beside the replacement; stop it before recovery",
            ));
        }
        let current = inspect(
            d,
            &ReviewInput {
                kind: t.kind.clone(),
                container_id: original_id(t)?,
                service_id: t.review.service.clone(),
            },
            t.id.clone(),
        )
        .await?;
        if current.evidence_revision != t.review.evidence_revision {
            return Err(conflict(
                "Automation around the original changed; review its evidence before restarting it",
            ));
        }
        advance(t, "retiring-replacement")?;
    }
    if phase(t) == "retiring-replacement" {
        remove_replacements(d, t).await?;
        preserve_appdata(t, "returned")?;
        advance(t, "releasing")?;
    }
    if phase(t) == "releasing" {
        let mut s = load(&t.id)?;
        s.phase = "returned".into();
        s.hold = None;
        save(&s)?;
        advance(t, "starting-original")?;
    }
    request(
        Method::POST,
        &format!("/containers/{}/update", original_id(t)?),
        Some(json!({"RestartPolicy":t.review.original["HostConfig"]["RestartPolicy"]})),
    )
    .await?;
    if t.source_running && source(t).await?["State"]["Running"] != true {
        updates::start(&original_id(t)?).await?;
    }
    t.stage = "returned".into();
    Ok(())
}

/// Build an independent replacement from the untouched snapshot. The discarded appdata is
/// kept as a generation and the new activation is recorded beside the first boundary.
async fn restore_snapshot(d: &Deployment, t: &mut Transfer) -> Result<()> {
    if phase(t) == "accepted" {
        advance(t, "checking-original")?;
    }
    if phase(t) == "checking-original" {
        original_quiet(t).await?;
        advance(t, "stopping-replacement")?;
    }
    if phase(t) == "stopping-replacement" {
        stop_replacement(d, &load(&t.id)?).await?;
        advance(t, "removing-replacement")?;
    }
    if phase(t) == "removing-replacement" {
        remove_replacements(d, t).await?;
        advance(t, "preserving-appdata")?;
    }
    if phase(t) == "preserving-appdata" {
        preserve_appdata(t, "replacement")?;
        t.intent = None;
        t.replacement = None;
        t.production = None;
        t.activated = false;
        t.committed = false;
        advance(t, "building")?;
    }
    if phase(t) == "building" {
        build(d, t).await?;
        advance(t, "creating")?;
    }
    if phase(t) == "creating" {
        create(d, t).await?;
        cross_boundary(t);
        advance(t, "activating")?;
    }
    activate(d, t).await?;
    t.activated = true;
    t.stage = "activated".into();
    Ok(())
}

/// Remove every stopped container this transfer created, after verifying it is ours.
async fn remove_replacements(d: &Deployment, t: &Transfer) -> Result<()> {
    for row in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if row["Labels"]["app.thelxinoe.deployment"] != d.id.as_str()
            || row["Labels"]["app.thelxinoe.managed-id"] != t.id.as_str()
        {
            continue;
        }
        let container = row["Id"].as_str().ok_or_else(unavailable)?;
        let raw = engine(&format!("/containers/{container}/json")).await?;
        if raw["State"]["Running"] != false {
            return Err(conflict("A replacement container is running; it was kept"));
        }
        updates::remove(container).await?;
    }
    Ok(())
}

/// Keep the replacement's appdata as a named generation. The name is journaled before the
/// appdata moves, so a resumed command neither loses nor duplicates it.
fn preserve_appdata(t: &mut Transfer, from: &str) -> Result<()> {
    let local = service_path(&t.id).with_file_name("appdata");
    let name = match t.command.as_ref().and_then(|c| c.generation.clone()) {
        Some(name) => name,
        None => {
            let name = (t.generations.len() + 1..)
                .map(|n| format!("generation-{n}"))
                .find(|name| {
                    !directory(&t.id).join(name).exists()
                        && !t.generations.iter().any(|g| g["name"] == name.as_str())
                })
                .ok_or_else(unavailable)?;
            t.command.as_mut().ok_or_else(unavailable)?.generation = Some(name.clone());
            write(t)?;
            name
        }
    };
    let target = directory(&t.id).join(&name);
    if local.exists() {
        persisted(std::fs::rename(&local, &target).map_err(Into::into))?;
    }
    if target.exists() && !t.generations.iter().any(|g| g["name"] == name.as_str()) {
        t.generations.push(json!({"name":name,"from":from,"replacement":t.replacement,
            "generation":generation(t),"digest":t.production.as_ref().map(|p| p["digest"].clone()),"at":thelxinoe_core::now()}));
        write(t)?;
    }
    Ok(())
}

async fn stop_replacement(d: &Deployment, s: &Managed) -> Result<()> {
    if s.container.is_empty() {
        return Ok(());
    }
    match engine(&format!("/containers/{}/json", s.container)).await {
        Ok(raw) => {
            verify_ownership(d, s, &raw).await?;
            if raw["State"]["Running"] != false {
                updates::stop(&s.container).await?;
            }
            if engine(&format!("/containers/{}/json", s.container)).await?["State"]["Running"]
                != false
            {
                return Err(conflict("The replacement did not stop"));
            }
            Ok(())
        }
        Err((StatusCode::NOT_FOUND, _)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Remove the retained original container after success; never its appdata or volumes.
pub(super) async fn remove_original(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let probe = read(&key)?;
    let _guard = runtime.0.service(&probe.kind).await;
    let mut t = read(&key)?;
    if !t.completed {
        return Err(conflict(
            "Remove the original only after the transfer completes",
        ));
    }
    if !t.original_removed {
        match engine(&format!("/containers/{}/json", original_id(&t)?)).await {
            Ok(raw) => {
                if raw["State"]["Running"] != false
                    || policy::fingerprint(&raw) != policy::fingerprint(&frozen(&t.review.original))
                {
                    return Err(conflict(
                        "The original is running or changed since the transfer; it was kept",
                    ));
                }
                request(
                    Method::DELETE,
                    &format!("/containers/{}?v=false", original_id(&t)?),
                    None,
                )
                .await?;
            }
            Err((StatusCode::NOT_FOUND, _)) => {}
            Err(error) => return Err(error),
        }
        t.original_removed = true;
        write(&mut t)?;
    }
    Ok(Json(status(&t, false)))
}

pub(super) async fn list(State(runtime): State<Runtime>) -> Result<Json<Value>> {
    let root = store::root().join("transfers");
    let mut items = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(root).map_err(|_| unavailable())? {
            let key = entry
                .map_err(|_| unavailable())?
                .file_name()
                .to_string_lossy()
                .into_owned();
            if directory(&key).join("transfer.json").exists() {
                let t = read(&key)?;
                let active = runtime.0.try_service(&t.kind).is_err();
                items.push(status(&t, active));
            }
        }
    }
    Ok(Json(json!({"items":items})))
}

#[cfg(test)]
#[path = "transfer_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_start_names_the_changed_review_components() {
        let reviewed: BTreeMap<String, String> = [("container", "a"), ("blockers", "b")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let mut current = reviewed.clone();
        current.insert("blockers".into(), "c".into());
        assert_eq!(
            changed_parts(&reviewed, &current),
            "the problems the review found"
        );
        assert_eq!(changed_parts(&reviewed, &reviewed), "the deployment");
    }

    /// A journal after the production boundary, in `stage`.
    fn journal(stage: &str) -> Transfer {
        let version = json!({"numbers":[1],"build":null,"text":"1"});
        let mount = json!({"kind":"bind","source":"/srv/config","destination":"/config","writable":true,"role":"appdata"});
        serde_json::from_value(json!({
            "id":"transfer","kind":"radarr","stage":stage,"error":null,
            "review":{"id":"transfer","kind":"radarr","service":"service","deployment":"deployment","engine":"engine",
                "original":{"Id":"original"},"source_image":"image","source_version":version,
                "decision":{"decision":"same","image":"image"},"target_version":version,
                "layout":{"appdata":mount,"retained":[],"supplementary":[]},"source_identity":[1000,1000],
                "target_identity":[10001,10001],"aliases":[],"published":null,"automation":[],"compose":null,
                "inspection":{},"access":{},"changes":[],"settings":[],"transformations":{},"blockers":[],
                "evidence_revision":"evidence","revision":"revision","created_at":0},
            "source_running":true,"restart_disabled":true,"stopped":true,"snapshot":{"digest":"snapshot"},
            "source_state":null,"qualification":null,"production":{"digest":"first","generation":"one"},
            "intent":null,"replacement":"first","boundary_at":100,"activated":true,"committed":true,
            "completed":false,"declarations":{},"compose_mode":null,"created_at":0,"updated_at":0
        }))
        .unwrap()
    }
    fn command(action: &str, phase: &str) -> Command {
        Command {
            operation: "operation".into(),
            action: action.into(),
            approval: json!({}),
            from: "recovery-required".into(),
            phase: phase.into(),
            state: "running".into(),
            error: None,
            generation: None,
            started_at: 0,
            finished_at: None,
        }
    }

    #[test]
    fn commands_fail_back_before_irreversible_steps_and_only_resume_a_return_under_way() {
        for (action, phase) in [
            ("repair", "starting"),
            ("return_original", "stopping-replacement"),
            ("return_original", "checking-original"),
            ("restore_snapshot", "checking-original"),
            ("restore_snapshot", "stopping-replacement"),
        ] {
            assert_eq!(
                command(action, phase).failure_stage().as_deref(),
                Some("recovery-required"),
                "{action} {phase}"
            );
        }
        // Without its replacement, a restored transfer needs a new decision.
        for phase in [
            "removing-replacement",
            "preserving-appdata",
            "building",
            "activating",
        ] {
            assert_eq!(
                command("restore_snapshot", phase)
                    .failure_stage()
                    .as_deref(),
                Some("recovery-required")
            );
        }
        let mut from_suspended = command("restore_snapshot", "accepted");
        from_suspended.from = "suspended".into();
        assert_eq!(from_suspended.failure_stage().as_deref(), Some("suspended"));
        // Once the replacement is retired, a return can only continue.
        for phase in ["retiring-replacement", "releasing", "starting-original"] {
            assert_eq!(command("return_original", phase).failure_stage(), None);
        }
    }

    #[test]
    fn recovery_is_planned_only_for_an_unfinished_transfer_after_the_boundary() {
        assert!(plannable(&journal("recovery-required")).is_ok());
        assert!(plannable(&journal("suspended")).is_ok());
        // A lost reply after a return must never offer starting the replacement again.
        for stage in ["returned", "recovering", "complete", "cancelled", "stale"] {
            assert!(plannable(&journal(stage)).is_err(), "{stage}");
        }
        let mut before = journal("failed");
        before.boundary_at = None;
        assert!(plannable(&before).is_err());
    }

    #[tokio::test]
    async fn a_replacement_never_starts_while_the_original_can_write() {
        for (state, restart) in [
            (json!({"Running":true}), json!({"Name":"no"})),
            (
                json!({"Running":false,"Restarting":true}),
                json!({"Name":"no"}),
            ),
            (json!({"Running":false}), json!({"Name":"unless-stopped"})),
        ] {
            let docker = crate::test_support::Docker::new();
            docker.insert(
                "original",
                json!({"Id":"original","Name":"/service","State":state,
                    "Config":{"Labels":{}},"HostConfig":{"RestartPolicy":restart}}),
            );
            let (d, _): (Deployment, Value) = docker.update();
            let mut t = journal("activating");
            t.replacement = Some("replacement".into());
            let error = docker
                .scope(activate(&d, &mut t))
                .await
                .expect_err("activation refused");
            assert_eq!(error.0, StatusCode::CONFLICT);
        }
    }

    #[test]
    fn the_first_boundary_stays_while_each_activation_is_recorded_once() {
        let mut t = journal("activating");
        cross_boundary(&mut t);
        cross_boundary(&mut t);
        assert_eq!(t.boundary_at, Some(100));
        assert_eq!(t.activations.len(), 1);
        // A restored candidate is a new activation with its own appdata generation.
        t.replacement = Some("second".into());
        t.production = Some(json!({"digest":"second","generation":"two"}));
        cross_boundary(&mut t);
        assert_eq!(t.boundary_at, Some(100));
        assert_eq!(t.activations[1]["replacement"], "second");
        assert_eq!(t.activations[1]["generation"], "two");
    }

    #[test]
    fn supplementary_files_mount_from_their_managed_copy() {
        let mut t = journal("building");
        t.review.layout.supplementary = vec![
            plan::Mount {
                kind: "bind".into(),
                source: "/srv/certs".into(),
                destination: "/certs".into(),
                writable: false,
                role: "supplementary".into(),
                file: false,
            },
            plan::Mount {
                kind: "bind".into(),
                source: "/srv/hook.sh".into(),
                destination: "/custom/hook.sh".into(),
                writable: false,
                role: "supplementary".into(),
                file: true,
            },
        ];
        let mounts = supplementary_mounts(&t, "/appdata");
        assert_eq!(mounts[0]["Source"], "/appdata/.thelxinoe-supplementary/0");
        assert_eq!(
            mounts[1]["Source"],
            format!("/appdata/.thelxinoe-supplementary/1/{SUPPLEMENTARY_FILE}")
        );
        assert_eq!(mounts[1]["Target"], "/custom/hook.sh");
        assert!(mounts.iter().all(|m| m["ReadOnly"] == true));
        // Folder mounts keep their reviewed shape; only files carry the flag.
        assert!(serde_json::to_value(&t.review.layout.supplementary[0]).unwrap()["file"].is_null());
    }

    #[test]
    fn restricted_hosts_keep_their_names_and_admit_the_replacement() {
        let hosts = ["radarr".to_owned(), "thelxinoe-radarr".to_owned()];
        assert_eq!(
            allowed_hosts(&json!("radarr;custom.example"), &hosts).as_deref(),
            Some("radarr;custom.example;thelxinoe-radarr")
        );
        // Unrestricted hosts stay unrestricted.
        assert_eq!(allowed_hosts(&json!(""), &hosts), None);
        assert_eq!(allowed_hosts(&json!("*"), &hosts), None);
        assert_eq!(allowed_hosts(&Value::Null, &hosts), None);
    }
}
