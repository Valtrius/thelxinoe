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
        return Err(conflict(
            "Ownership requires a supported Linux x86-64 service",
        ));
    }
    policy::transfer_owner(&raw, true).map_err(conflict)?;
    let host = &raw["HostConfig"];
    if host["Privileged"] == true
        || ["PidMode", "IpcMode", "NetworkMode", "UsernsMode"]
            .iter()
            .any(|key| host[key] == "host")
        || host["CapAdd"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|cap| cap == "SYS_ADMIN" || cap == "ALL")
        || host["Devices"]
            .as_array()
            .is_some_and(|devices| !devices.is_empty())
        || host["NetworkMode"]
            .as_str()
            .is_some_and(|mode| mode.starts_with("container:"))
    {
        return Err(conflict(
            "This deployment exceeds the supported controller security boundary",
        ));
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
        if kind == "bind"
            && (!policy::appdata_isolated(source, &d.appdata_source)
                || destination == "/var/run/docker.sock"
                || destination == "/run/docker.sock")
        {
            return Err(conflict(
                "Host system and controller storage mounts cannot be adopted",
            ));
        }
        storage.push(StorageReference {
            kind: kind.into(),
            source: source.into(),
            destination: destination.into(),
            writable: m["RW"] == true,
            borrowed: true,
        });
    }
    let config = storage.iter().find(|m| m.destination == "/config");
    let backup = if config.is_some_and(|m| {
        m.kind == "bind" && m.writable && policy::appdata_isolated(&m.source, &d.media_source)
    }) && storage.iter().all(|m| {
        m.destination == "/config"
            || !m.writable
            || ["/media", "/movies", "/tv", "/music", "/downloads"]
                .iter()
                .any(|root| {
                    m.destination == *root || m.destination.starts_with(&format!("{root}/"))
                })
    }) {
        Capability::available()
    } else {
        Capability::blocked(
            "Recovery coverage is incomplete: the existing persistent storage needs a supported snapshot adapter",
        )
    };
    let capabilities = Capabilities {
        lifecycle: Capability::available(),
        backup,
        update: Capability::blocked(
            "Faithful recreation and recovery must be verified before updating this imported deployment",
        ),
        recreate: Capability::blocked(
            "Review a replacement deployment before changing the registered container identity",
        ),
        release: Capability::available(),
    };
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
    if s.phase != "active" || s.active_update.is_some() {
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
