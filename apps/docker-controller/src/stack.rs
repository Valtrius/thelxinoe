//! Constrained managed-service lifecycle over the private controller socket.
use crate::{
    docker::{Result, engine, request, unavailable},
    policy, store, templates,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[path = "adoption.rs"]
mod adoption;
#[path = "backups.rs"]
mod backups;
#[path = "product.rs"]
pub(crate) mod product;
#[path = "service_recovery.rs"]
mod recovery;
#[path = "service_removal.rs"]
mod removal;
#[path = "updates.rs"]
mod updates;
pub async fn retain_worker_image() -> anyhow::Result<()> {
    updates::current_image()
        .await
        .map(|_| ())
        .map_err(|(_, message)| anyhow::anyhow!(message))
}
pub async fn recover_backups() -> anyhow::Result<()> {
    backups::recover_interrupted()
        .await
        .map_err(|(_, message)| anyhow::anyhow!(message))
}
#[derive(Clone)]
struct Runtime(thelxinoe_core::operation_locks::OperationLocks);
#[derive(Clone, Serialize, Deserialize)]
struct Managed {
    id: String,
    kind: String,
    container: String,
    name: String,
    image: String,
    phase: String,
    #[serde(default)]
    active_update: Option<String>,
    spec: Value,
    expected: Value,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Deployment {
    id: String,
    generation: u64,
    version: String,
    server: Value,
    controller: Value,
    network: String,
    media_source: String,
    appdata_source: String,
}
fn conflict(message: &'static str) -> (StatusCode, &'static str) {
    (StatusCode::CONFLICT, message)
}
fn bad(message: &'static str) -> (StatusCode, &'static str) {
    (StatusCode::BAD_REQUEST, message)
}
fn persisted<T>(result: anyhow::Result<T>) -> Result<T> {
    result.map_err(|_| unavailable())
}
fn id(value: &str) -> Result<()> {
    if value.len() == 36 && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        Ok(())
    } else {
        Err(bad("Invalid managed service ID"))
    }
}
fn service_path(key: &str) -> std::path::PathBuf {
    store::root()
        .join("services")
        .join(key)
        .join("service.json")
}
fn choose_service_name(
    kind: &str,
    key: &str,
    containers: &Value,
    replacing: Option<&str>,
) -> Result<String> {
    let rows = containers.as_array().ok_or_else(unavailable)?;
    let occupied = |name: &str| {
        rows.iter().any(|row| {
            replacing != row["Id"].as_str()
                && row["Names"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|n| n.trim_start_matches('/') == name)
        })
    };
    let primary = format!("thelxinoe-{kind}");
    if !occupied(&primary) {
        return Ok(primary);
    }
    // Separate deployments can share a Docker daemon without taking each other's names.
    let scoped = format!("{primary}-{}", &key[..8]);
    if occupied(&scoped) {
        return Err(conflict("Managed container name is already in use"));
    }
    Ok(scoped)
}
fn load(key: &str) -> Result<Managed> {
    id(key)?;
    persisted(store::read(&service_path(key)))
}
fn save(value: &Managed) -> Result<()> {
    persisted(store::write_json(&service_path(&value.id), value))
}
fn services() -> Result<Vec<Managed>> {
    let path = store::root().join("services");
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut rows: Vec<Managed> = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        id(&name)?;
        if entry.path().join("service.json").exists() {
            let service = load(&name)?;
            if !matches!(service.phase.as_str(), "returned" | "retired" | "removed") {
                if rows.iter().any(|existing| existing.kind == service.kind) {
                    return Err(conflict("Managed state contains duplicate service kinds"));
                }
                rows.push(service);
            }
        }
    }
    Ok(rows)
}
fn deployment_has_kind(containers: &Value, deployment: &str, kind: &str) -> Result<bool> {
    Ok(containers
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .any(|container| {
            container["Labels"]["app.thelxinoe.deployment"] == deployment
                && container["Labels"]["app.thelxinoe.kind"] == kind
        }))
}
async fn ensure_kind_available(d: &Deployment, kind: &str) -> Result<Value> {
    if services()?.iter().any(|service| service.kind == kind) {
        return Err(conflict("This service already has a managed installation"));
    }
    let containers = engine("/containers/json?all=true").await?;
    let mut active = containers.clone();
    let mut retained = Vec::new();
    for container in containers.as_array().ok_or_else(unavailable)? {
        if container["Labels"]["app.thelxinoe.deployment"] != d.id
            || container["Labels"]["app.thelxinoe.kind"] != kind
        {
            continue;
        }
        if let Some(key) = container["Labels"]["app.thelxinoe.managed-id"].as_str()
            && let Ok(service) = load(key)
            && service.phase == "retired"
        {
            retained.extend(updates::retained_originals(d, &service).await?);
        }
    }
    active
        .as_array_mut()
        .ok_or_else(unavailable)?
        .retain(|row| {
            !row["Id"]
                .as_str()
                .is_some_and(|id| retained.iter().any(|old| old == id))
        });
    if deployment_has_kind(&active, &d.id, kind)? {
        return Err(conflict("This service already has a managed installation"));
    }
    Ok(containers)
}
fn mount<'a>(c: &'a Value, destination: &str) -> Result<&'a Value> {
    c["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["Destination"] == destination)
        .ok_or_else(|| conflict("Required first-party mount is missing"))
}
fn bootstrap_server_candidate(container: &Value, runtime: &Value) -> bool {
    container["State"]["Running"] == true
        && mount(container, "/run/thelxinoe").is_ok_and(|m| m["Source"] == *runtime)
}
fn immutable(c: &Value) -> Result<String> {
    let value = c["Image"].as_str().ok_or_else(unavailable)?;
    if value.len() != 71 || !value.starts_with("sha256:") {
        return Err(conflict("First-party image identity is not immutable"));
    }
    Ok(value.into())
}
fn compose_service(c: &Value) -> Result<Value> {
    let config = &c["Config"];
    let host = &c["HostConfig"];
    let volumes=c["Mounts"].as_array().ok_or_else(unavailable)?.iter().map(|m|json!({"type":"bind","source":m["Source"],"target":m["Destination"],"read_only":m["RW"]==false})).collect::<Vec<_>>();
    let ports = host["PortBindings"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(port, bindings)| {
            bindings.as_array().into_iter().flatten().map(move |b| {
                let (target, protocol) = port.split_once('/').unwrap_or((port, "tcp"));
                json!({"target":target.parse::<u16>().unwrap_or(0),"published":b["HostPort"],"host_ip":b["HostIp"],"protocol":protocol})
            })
        })
        .collect::<Vec<_>>();
    let mut value = json!({"image":immutable(c)?,"container_name":c["Name"].as_str().unwrap_or("").trim_start_matches('/'),"environment":config["Env"],"labels":config["Labels"],"volumes":volumes,"ports":ports,"restart":host["RestartPolicy"]["Name"],"read_only":host["ReadonlyRootfs"],"cap_drop":host["CapDrop"],"security_opt":host["SecurityOpt"],"user":config["User"]});
    for (source, target) in [
        ("Cmd", "command"),
        ("Entrypoint", "entrypoint"),
        ("WorkingDir", "working_dir"),
    ] {
        if !config[source].is_null() {
            value[target] = config[source].clone();
        }
    }
    if let Some(health) = config["Healthcheck"].as_object() {
        let mut converted = json!({"test":health.get("Test")});
        for (source, target) in [
            ("Interval", "interval"),
            ("Timeout", "timeout"),
            ("StartPeriod", "start_period"),
            ("StartInterval", "start_interval"),
        ] {
            if let Some(nanos) = health
                .get(source)
                .and_then(Value::as_u64)
                .filter(|v| *v > 0)
            {
                converted[target] = json!(format!("{nanos}ns"));
            }
        }
        if let Some(retries) = health.get("Retries") {
            converted["retries"] = retries.clone();
        }
        value["healthcheck"] = converted;
    }
    for (source, target) in [
        ("CapAdd", "cap_add"),
        ("Init", "init"),
        ("PidsLimit", "pids_limit"),
    ] {
        if !host[source].is_null() {
            value[target] = host[source].clone();
        }
    }
    for (source, target) in [
        ("Memory", "mem_limit"),
        ("MemorySwap", "memswap_limit"),
        ("CpuShares", "cpu_shares"),
    ] {
        if host[source].as_i64().is_some_and(|v| v != 0) {
            value[target] = host[source].clone();
        }
    }
    if let Some(nanos) = host["NanoCpus"].as_u64().filter(|v| *v > 0) {
        value["cpus"] = json!(nanos as f64 / 1_000_000_000.0);
    }
    if host["NetworkMode"] == "none" {
        value["network_mode"] = json!("none");
    } else {
        value["networks"] = json!(["media"]);
    }
    if let Some(tmpfs) = host["Tmpfs"].as_object() {
        value["tmpfs"] = json!(
            tmpfs
                .iter()
                .map(|(path, options)| format!("{path}:{}", options.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
        );
    }
    value.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    escape_compose(&mut value);
    Ok(value)
}
fn escape_compose(value: &mut Value) {
    match value {
        Value::String(text) => *text = text.replace('$', "$$"),
        Value::Array(items) => items.iter_mut().for_each(escape_compose),
        Value::Object(items) => items.values_mut().for_each(escape_compose),
        _ => (),
    }
}
async fn bootstrap() -> Result<Deployment> {
    static INITIALIZE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _initializing = INITIALIZE.lock().await;
    let root = store::root();
    if root.join("desired-state.json").exists() {
        return persisted(store::read(&root.join("desired-state.json")));
    }
    let self_id = std::env::var("HOSTNAME").map_err(|_| unavailable())?;
    if !self_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(conflict("Controller container identity unavailable"));
    }
    let controller = engine(&format!("/containers/{self_id}/json")).await?;
    let runtime = mount(&controller, "/run/thelxinoe")?["Source"].clone();
    let containers = engine("/containers/json?all=true").await?;
    let mut candidates = Vec::new();
    for c in containers.as_array().ok_or_else(unavailable)? {
        if c["Labels"]["app.thelxinoe.component"] != "server" {
            continue;
        }
        let raw = engine(&format!(
            "/containers/{}/json",
            c["Id"].as_str().ok_or_else(unavailable)?
        ))
        .await?;
        if bootstrap_server_candidate(&raw, &runtime) {
            candidates.push(raw);
        }
    }
    if candidates.len() != 1 {
        return Err(conflict(
            "Exactly one labeled server must share this controller runtime",
        ));
    }
    let server = candidates.remove(0);
    let networks = server["NetworkSettings"]["Networks"]
        .as_object()
        .ok_or_else(unavailable)?;
    if networks.len() != 1 {
        return Err(conflict(
            "Managed stack requires a single server media network",
        ));
    }
    let network = networks.keys().next().unwrap().clone();
    let generation = std::fs::read_dir(root.join("generations"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let d = Deployment {
        id: thelxinoe_core::id(),
        generation,
        version: thelxinoe_core::VERSION.into(),
        server: server.clone(),
        controller: controller.clone(),
        network: network.clone(),
        media_source: policy::media_source(&server).map_err(conflict)?.into(),
        appdata_source: mount(&controller, "/var/lib/thelxinoe/deployment")?["Source"]
            .as_str()
            .ok_or_else(unavailable)?
            .into(),
    };
    if !policy::media_disjoint(&d.appdata_source, &d.media_source) {
        return Err(conflict(
            "Deployment state must be outside every media mount",
        ));
    }
    let project = controller["Config"]["Labels"]["com.docker.compose.project"]
        .as_str()
        .unwrap_or("thelxinoe");
    let compose = json!({"name":project,"services":{"server":compose_service(&server)?,"controller":compose_service(&controller)?},"networks":{"media":{"external":true,"name":network}}});
    persisted(store::write_json(
        &root.join("compose.yaml"),
        &json!({"name":project,"services":{}}),
    ))?;
    persisted(store::commit_generation(
        &root,
        generation,
        &serde_json::to_value(&d).map_err(|_| unavailable())?,
        &compose,
    ))?;
    Ok(d)
}
pub fn router() -> Router {
    Router::new()
        .route(
            "/stack/templates",
            get(|| async { Json(json!({"items":templates::TEMPLATES})) }),
        )
        .route("/stack", get(list))
        .route("/stack/install", post(install))
        .route("/stack/updates", get(updates::list))
        .route("/stack/backups", get(backups::list).post(backups::create))
        .route("/stack/backups/{id}/restore", post(backups::restore))
        .route("/stack/product", get(product::list))
        .route("/stack/product/preflight", post(product::preflight))
        .route("/stack/product/{id}/activate", post(product::activate))
        .route("/stack/product/{id}/recover", post(product::recover))
        .route("/stack/{id}/preflight", post(updates::preflight))
        .route("/stack/updates/{id}/activate", post(updates::activate))
        .route("/stack/updates/{id}/recover", post(updates::recover))
        .route("/stack/releases", get(releases))
        .route("/stack/adopt/preview", post(adoption::preview))
        .route("/stack/adopt/check", post(adoption::check))
        .route("/stack/adopt", post(adoption::adopt))
        .route("/stack/{id}/action", post(action))
        .with_state(Runtime(Default::default()))
}
async fn list(State(runtime): State<Runtime>) -> Result<Json<Value>> {
    let d = if store::root().join("desired-state.json").exists() {
        bootstrap().await?
    } else {
        let _guard = runtime.0.lock().await;
        bootstrap().await?
    };
    let mut result = Vec::new();
    for s in services()? {
        let identity = if s.container.is_empty() {
            &s.name
        } else {
            &s.container
        };
        let live = engine(&format!("/containers/{identity}/json")).await;
        result.push(recovery::observation(&s, &live));
    }
    Ok(Json(
        json!({"deployment_id":d.id,"generation":d.generation,"items":result}),
    ))
}
#[derive(Deserialize)]
struct Install {
    operation_id: String,
    kind: String,
    host_port: u16,
    secret: String,
    #[serde(default)]
    username: String,
}
fn app_config(t: templates::Template, input: &Install, directory: &std::path::Path) -> Result<()> {
    if !(24..=128).contains(&input.secret.len())
        || !input.secret.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(bad(
            "Provisioning requires a generated alphanumeric credential",
        ));
    }
    let text = match t.kind {
        "nzbget" => {
            if input.username != "thelxinoe" {
                return Err(bad("Use the managed NZBGet account"));
            }
            // A preseeded config bypasses the image's defaults. Keep the
            // download, logging, and unpack settings useful from first boot.
            // The paths also serve the web interface and settings editor.
            format!(
                "MainDir=/media/downloads\nDestDir=/media/downloads/completed\nInterDir=/media/downloads/intermediate\nNzbDir=/config/nzb\nQueueDir=/config/queue\nTempDir=/config/tmp\nScriptDir=/config/scripts\nLogFile=/config/nzbget.log\nWriteLog=rotate\nRotateLog=3\nArticleCache=128\nDirectWrite=yes\nWriteBuffer=1024\nFileNaming=auto\nPostStrategy=balanced\nNzbCleanupDisk=yes\nParCheck=auto\nParRepair=yes\nUnpack=yes\nWebDir=${{AppDir}}/webui\nConfigTemplate=${{AppDir}}/webui/nzbget.conf.template\nControlIP=0.0.0.0\nControlPort=6789\nControlUsername=thelxinoe\nControlPassword={}\n",
                input.secret
            )
        }
        "bazarr" => format!(
            "auth:\n  apikey: {}\ngeneral:\n  ip: 0.0.0.0\n  port: 6767\n  use_sonarr: false\n  use_radarr: false\n",
            input.secret
        ),
        _ => format!(
            "<Config><BindAddress>*</BindAddress><Port>{}</Port><EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>{}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><Branch>master</Branch><LogLevel>info</LogLevel><UpdateAutomatically>False</UpdateAutomatically></Config>",
            t.port, input.secret
        ),
    };
    let file = match t.kind {
        "nzbget" => "nzbget.conf",
        "bazarr" => "config/config.yaml",
        _ => "config.xml",
    };
    persisted(store::write(&directory.join(file), text.as_bytes()))?;
    // LSIO initializes its appdata ownership on first boot as root.
    Ok(())
}
async fn install(
    State(runtime): State<Runtime>,
    Json(input): Json<Install>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service(&input.kind).await;
    let t = templates::find(&input.kind).ok_or_else(|| bad("Unknown curated service"))?;
    if input.host_port < 1024 {
        return Err(bad("Choose a nonprivileged local port"));
    }
    let d = bootstrap().await?;
    let containers = ensure_kind_available(&d, &input.kind).await?;
    if t.media {
        // Compose edits must not silently provision with a stale stored layout.
        let server = engine(&format!(
            "/containers/{}/json",
            d.server["Name"]
                .as_str()
                .or(d.server["Id"].as_str())
                .ok_or_else(unavailable)?
                .trim_start_matches('/')
        ))
        .await?;
        if policy::media_source(&server).map_err(conflict)? != d.media_source {
            return Err(conflict(
                "Server media mounts changed after deployment initialization; reconcile the deployment layout before installing managed services",
            ));
        }
    }
    id(&input.operation_id)?;
    let key = input.operation_id.clone();
    if service_path(&key).exists() {
        return Err(conflict("Provisioning identity already exists"));
    }
    let directory = store::root().join("services").join(&key).join("appdata");
    app_config(t, &input, &directory)?;
    let image = format!("{}@{}", t.repository, t.digest);
    request(
        reqwest::Method::POST,
        &format!("/images/create?fromImage={}@{}", t.repository, t.digest),
        None,
    )
    .await?;
    let name = choose_service_name(t.kind, &key, &containers, None)?;
    let mut mounts = vec![
        json!({"Type":"bind","Source":format!("{}/services/{key}/appdata",d.appdata_source),"Target":"/config"}),
    ];
    if t.media {
        mounts.push(json!({"Type":"bind","Source":d.media_source,"Target":"/media"}));
    }
    let port = format!("{}/tcp", t.port);
    let spec = json!({"Image":image,"Env":["PUID=10001","PGID=10001","TZ=UTC"],"Labels":{"app.thelxinoe.managed-id":key,"app.thelxinoe.deployment":d.id,"app.thelxinoe.kind":t.kind},"HostConfig":{"Mounts":mounts,"NetworkMode":d.network,"RestartPolicy":{"Name":"unless-stopped"},"PortBindings":{port:[{"HostIp":"127.0.0.1","HostPort":input.host_port.to_string()}]}},"NetworkingConfig":{"EndpointsConfig":{d.network.clone():{"Aliases":[format!("thelxinoe-{}",t.kind)]}}}});
    let mut s = Managed {
        id: key,
        kind: input.kind,
        container: String::new(),
        name: name.clone(),
        image,
        phase: "creating".into(),
        active_update: None,
        spec: spec.clone(),
        expected: Value::Null,
        error: None,
    };
    save(&s)?;
    let _ = recovery::resume_creation(&d, &mut s).await?;
    Ok(Json(
        json!({"id":s.id,"kind":s.kind,"container_id":s.container,"port":t.port,"host_port":input.host_port}),
    ))
}
#[derive(Deserialize)]
struct Action {
    action: String,
}
async fn action(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    id(&key)?;
    let kind = if service_path(&key).exists() {
        load(&key)?.kind
    } else {
        key.clone()
    };
    let _guard = runtime.0.service(&kind).await;
    let d = bootstrap().await?;
    if input.action == "retire" && !service_path(&key).exists() {
        let rows = engine("/containers/json?all=true").await?;
        if rows.as_array().ok_or_else(unavailable)?.iter().any(|row| {
            row["Labels"]["app.thelxinoe.deployment"] == d.id
                && row["Labels"]["app.thelxinoe.managed-id"] == key
        }) {
            return Err(conflict(
                "An unrecorded container still owns this installation",
            ));
        }
        return Ok(Json(
            json!({"accepted":true,"retired":true,"appdata_preserved":true}),
        ));
    }
    if input.action == "restore_original" && !service_path(&key).exists() {
        return adoption::cancel_unsubmitted(&d, &key).await;
    }
    let mut s = load(&key)?;
    if input.action == "remove" {
        return removal::remove(&d, &mut s).await;
    }
    if input.action == "retire" {
        return recovery::retire(&d, &mut s).await;
    }
    if input.action == "recreate" {
        return recovery::recreate(&d, &mut s).await;
    }
    if input.action == "complete_adoption" {
        return adoption::complete(&d, &mut s).await;
    }
    if input.action == "restore_original" {
        return adoption::restore(&d, &mut s).await;
    }
    if input.action == "reconcile" {
        if adoption::pending(&s) {
            return adoption::reconcile(&d, &mut s).await;
        }
        return reconcile(&d, &mut s).await;
    }
    if s.phase != "active" {
        return Err(conflict(
            "Interrupted service operation requires reconciliation",
        ));
    }
    let raw = engine(&format!("/containers/{}/json", s.container)).await?;
    if raw["Config"]["Labels"]["app.thelxinoe.deployment"] != d.id
        || raw["Config"]["Labels"]["app.thelxinoe.managed-id"] != s.id
        || policy::fingerprint(&raw) != s.expected
    {
        return Err(conflict("Docker configuration drift blocks this action"));
    }
    let endpoint = match input.action.as_str() {
        "start" => "start",
        "stop" => "stop?t=30",
        "restart" => "restart?t=30",
        _ => return Err(bad("Unsupported managed lifecycle action")),
    };
    s.phase = "changing".into();
    save(&s)?;
    request(
        reqwest::Method::POST,
        &format!("/containers/{}/{endpoint}", s.container),
        None,
    )
    .await?;
    let after = engine(&format!("/containers/{}/json", s.container)).await?;
    let fingerprint = policy::fingerprint(&after);
    if fingerprint != s.expected && !policy::first_start_matches(&s.expected, &fingerprint) {
        return Err(conflict(
            "Docker configuration changed during the lifecycle action",
        ));
    }
    s.expected = fingerprint;
    s.phase = "active".into();
    save(&s)?;
    Ok(Json(json!({"accepted":true})))
}

async fn releases() -> Result<Json<Value>> {
    let mut results = Vec::new();
    for t in templates::TEMPLATES {
        let result = engine(&format!("/distribution/{}:latest/json", t.repository)).await;
        let digest = result
            .ok()
            .and_then(|v| v["Descriptor"]["digest"].as_str().map(str::to_owned))
            .filter(|v| {
                v.starts_with("sha256:")
                    && v.len() == 71
                    && v[7..].bytes().all(|b| b.is_ascii_hexdigit())
            });
        results.push(json!({"kind":t.kind,"channel":"stable","image":digest.map(|d|format!("{}@{d}",t.repository)),"tested_image":format!("{}@{}",t.repository,t.digest)}));
    }
    Ok(Json(json!({"items":results})))
}

fn subset(expected: &Value, actual: &Value) -> bool {
    match expected {
        Value::Object(fields) => actual.as_object().is_some_and(|object| {
            fields
                .iter()
                .all(|(key, value)| object.get(key).is_some_and(|a| subset(value, a)))
        }),
        Value::Array(items) => actual.as_array().is_some_and(|a| {
            items.len() == a.len() && items.iter().zip(a).all(|(e, a)| subset(e, a))
        }),
        value => value == actual,
    }
}
async fn reconcile(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if s.phase == "updating" {
        return Err(conflict("Use update recovery for an interrupted update"));
    }
    if matches!(s.phase.as_str(), "creating" | "recreating") {
        return recovery::resume_creation(d, s).await;
    }
    let container = recovery::find_container(d, s)
        .await?
        .ok_or_else(|| conflict("Container is missing; recreate it or retire the installation"))?;
    let raw = engine(&format!("/containers/{container}/json")).await?;
    verify_recorded(d, s, &raw).await?;
    s.expected = policy::fingerprint(&raw);
    s.container = raw["Id"].as_str().ok_or_else(unavailable)?.into();
    s.phase = "active".into();
    s.error = None;
    save(s)?;
    Ok(Json(
        json!({"accepted":true,"container_id":s.container,"running":raw["State"]["Running"]}),
    ))
}

fn verify_ownership(d: &Deployment, s: &Managed, raw: &Value) -> Result<()> {
    if raw["Config"]["Labels"]["app.thelxinoe.deployment"] != d.id
        || raw["Config"]["Labels"]["app.thelxinoe.managed-id"] != s.id
    {
        return Err(conflict("Container name belongs to another owner"));
    }
    Ok(())
}

fn verify_fingerprint(d: &Deployment, s: &Managed, raw: &Value) -> Result<()> {
    verify_ownership(d, s, raw)?;
    let actual = policy::fingerprint(raw);
    if actual != s.expected
        && !(s.phase == "changing" && policy::first_start_matches(&s.expected, &actual))
    {
        return Err(conflict("Docker configuration drift blocks reconciliation"));
    }
    Ok(())
}

async fn verify_recorded(d: &Deployment, s: &Managed, raw: &Value) -> Result<()> {
    verify_ownership(d, s, raw)?;
    if !s.expected.is_null() {
        verify_fingerprint(d, s, raw)?;
    } else {
        if raw["Config"]["Image"] != s.spec["Image"]
            || !subset(&s.spec["HostConfig"], &raw["HostConfig"])
            || !subset(&s.spec["Labels"], &raw["Config"]["Labels"])
        {
            return Err(conflict(
                "Interrupted container no longer matches its recorded creation spec",
            ));
        }
        let image = engine(&format!(
            "/images/{}/json",
            raw["Image"].as_str().ok_or_else(unavailable)?
        ))
        .await?;
        let env_map = |value: &Value| {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|v| v.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let mut env = env_map(&image["Config"]["Env"]);
        env.extend(env_map(&s.spec["Env"]));
        if env != env_map(&raw["Config"]["Env"]) {
            return Err(conflict(
                "Container environment changed during provisioning",
            ));
        }
        for field in ["Cmd", "Entrypoint", "User", "WorkingDir", "Healthcheck"] {
            let expected = s.spec.get(field).unwrap_or(&image["Config"][field]);
            if !policy::same_default(expected, &raw["Config"][field]) {
                return Err(conflict(
                    "Container command configuration changed during provisioning",
                ));
            }
        }
        let networks = raw["NetworkSettings"]["Networks"]
            .as_object()
            .ok_or_else(unavailable)?;
        if networks.len() != 1 || !networks.contains_key(&d.network) {
            return Err(conflict("Container network changed during provisioning"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod naming_tests {
    use super::*;
    #[test]
    fn names_are_prefixed_without_claiming_another_deployments_container() {
        let key = "01234567-89ab-cdef-0123-456789abcdef";
        let occupied = json!([{"Id":"other","Names":["/thelxinoe-radarr"]}]);
        assert_eq!(
            choose_service_name("radarr", key, &json!([]), None).unwrap(),
            "thelxinoe-radarr"
        );
        assert_eq!(
            choose_service_name("radarr", key, &occupied, Some("other")).unwrap(),
            "thelxinoe-radarr"
        );
        assert_eq!(
            choose_service_name("radarr", key, &occupied, None).unwrap(),
            "thelxinoe-radarr-01234567"
        );
        let collision =
            json!([{"Id":"other","Names":["/thelxinoe-radarr", "/thelxinoe-radarr-01234567"]}]);
        assert!(choose_service_name("radarr", key, &collision, None).is_err());
    }

    #[test]
    fn managed_kind_detection_is_scoped_to_the_current_deployment() {
        let containers = json!([
            {"Labels":{"app.thelxinoe.deployment":"current","app.thelxinoe.kind":"radarr"}},
            {"Labels":{"app.thelxinoe.deployment":"other","app.thelxinoe.kind":"sonarr"}}
        ]);
        assert!(deployment_has_kind(&containers, "current", "radarr").unwrap());
        assert!(!deployment_has_kind(&containers, "current", "sonarr").unwrap());
    }

    #[test]
    fn bootstrap_ignores_stopped_servers_sharing_the_controller_runtime() {
        let runtime = json!("controller-runtime");
        let server = |running: bool, source: &str| {
            json!({
                "State":{"Running":running},
                "Mounts":[{"Source":source,"Destination":"/run/thelxinoe"}]
            })
        };
        assert!(bootstrap_server_candidate(
            &server(true, "controller-runtime"),
            &runtime
        ));
        assert!(!bootstrap_server_candidate(
            &server(false, "controller-runtime"),
            &runtime
        ));
        assert!(!bootstrap_server_candidate(
            &server(true, "other-runtime"),
            &runtime
        ));
    }
}
