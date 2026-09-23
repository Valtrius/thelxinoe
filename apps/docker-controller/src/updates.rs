//! Journaled, isolated service updates. Production activation is an explicit one-way boundary.
use super::*;
use reqwest::Method;
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
struct Update {
    id: String,
    service: String,
    candidate: String,
    stage: String,
    classification: String,
    error: Option<String>,
    old: Managed,
    was_running: bool,
    snapshot_complete: bool,
    recovery_complete: bool,
    activation_crossed: bool,
    candidate_container: Option<String>,
    replacement: Option<String>,
}
fn path(key: &str) -> PathBuf {
    store::root().join("updates").join(key)
}
fn read(key: &str) -> Result<Update> {
    id(key)?;
    persisted(store::read(&path(key).join("update.json")))
}
fn read_listed(key: &str) -> Result<Option<Update>> {
    id(key)?;
    // A different service can remove its update copies after directory discovery.
    let bytes = match std::fs::read(path(key).join("update.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable()),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| unavailable())
}
fn write(u: &Update) -> Result<()> {
    persisted(store::write_json(&path(&u.id).join("update.json"), u))
}
fn public(u: &Update) -> Value {
    json!({"id":u.id,"service_id":u.service,"candidate":u.candidate,"stage":u.stage,"classification":u.classification,"error":u.error,"activation_crossed":u.activation_crossed,"container_id":u.replacement})
}
pub(super) async fn list() -> Result<Json<Value>> {
    let root = store::root().join("updates");
    let mut items = Vec::new();
    if root.exists() {
        for entry in std::fs::read_dir(root).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            if entry.path().join("update.json").is_file()
                && let Some(update) = read_listed(&entry.file_name().to_string_lossy())?
            {
                items.push(public(&update));
            }
        }
    }
    Ok(Json(json!({"items":items})))
}
pub(super) async fn verified(s: &Managed, d: &Deployment) -> Result<Value> {
    let raw = engine(&format!("/containers/{}/json", s.container)).await?;
    if policy::fingerprint(&raw) != s.expected
        || raw["Config"]["Labels"]["app.thelxinoe.deployment"] != d.id
        || raw["Config"]["Labels"]["app.thelxinoe.managed-id"] != s.id
    {
        return Err(conflict("Configuration drift blocks update work"));
    }
    Ok(raw)
}

pub(super) async fn retained_originals(d: &Deployment, service: &Managed) -> Result<Vec<String>> {
    let root = store::root().join("updates");
    let mut retained = Vec::new();
    if !root.exists() {
        return Ok(retained);
    }
    for entry in std::fs::read_dir(root).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        if !entry.path().join("update.json").is_file() {
            continue;
        }
        let Some(update) = read_listed(&entry.file_name().to_string_lossy())? else {
            continue;
        };
        if update.service != service.id
            || update.old.container == service.container
            || !update.activation_crossed
            || !matches!(update.stage.as_str(), "committed" | "runtime-failure")
        {
            continue;
        }
        match verified(&update.old, d).await {
            Ok(raw) if raw["State"]["Running"] == false => retained.push(update.old.container),
            Err((StatusCode::NOT_FOUND, _)) => {}
            _ => {
                return Err(conflict(
                    "A retained update container is running or changed; stop and reconcile it before recovery",
                ));
            }
        }
    }
    Ok(retained)
}
pub(super) fn removable_updates(service: &str) -> Result<Vec<String>> {
    let root = store::root().join("updates");
    let mut keys = Vec::new();
    if root.exists() {
        for entry in std::fs::read_dir(root).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            if !entry.path().join("update.json").is_file() {
                continue;
            }
            let Some(update) = read_listed(&entry.file_name().to_string_lossy())? else {
                continue;
            };
            if update.service == service {
                if !matches!(
                    update.stage.as_str(),
                    "committed" | "rolled-back" | "blocked"
                ) {
                    return Err(conflict(
                        "Finish or recover the service update before removal",
                    ));
                }
                keys.push(update.id);
            }
        }
    }
    Ok(keys)
}
#[derive(Deserialize)]
pub(super) struct Preflight {
    operation_id: String,
}
pub(super) async fn preflight(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Preflight>,
) -> Result<Json<Value>> {
    id(&input.operation_id)?;
    if path(&input.operation_id).join("update.json").exists() {
        let existing = read(&input.operation_id)?;
        if existing.service != key {
            return Err(conflict("Update identity belongs to another service"));
        }
        return Ok(Json(public(&existing)));
    }
    let guard = runtime
        .0
        .try_service(&load(&key)?.kind)
        .map_err(|_| conflict("This service or the deployment has an active operation"))?;
    let d = bootstrap().await?;
    let mut s = load(&key)?;
    if s.phase != "active" {
        return Err(conflict("Service requires recovery before an update"));
    }
    let raw = verified(&s, &d).await?;
    let t = templates::find(&s.kind).ok_or_else(unavailable)?;
    let discovered = engine(&format!("/distribution/{}:latest/json", t.repository)).await?;
    let digest = discovered["Descriptor"]["digest"]
        .as_str()
        .ok_or_else(unavailable)?;
    if digest.len() != 71
        || !digest.starts_with("sha256:")
        || !digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(unavailable());
    }
    let mut u = Update {
        id: input.operation_id,
        service: s.id.clone(),
        candidate: format!("{}@{digest}", t.repository),
        stage: "preparing".into(),
        classification: "pending".into(),
        error: None,
        old: s.clone(),
        was_running: raw["State"]["Running"] == true,
        snapshot_complete: false,
        recovery_complete: false,
        activation_crossed: false,
        candidate_container: None,
        replacement: None,
    };
    write(&u)?;
    s.phase = "updating".into();
    s.active_update = Some(u.id.clone());
    save(&s)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        if let Err(error) = check(&d, &mut u).await {
            u.stage = "blocked".into();
            u.classification = "unable-to-verify".into();
            u.error = Some(error.1.into());
            let _ = write(&u);
            // The old container has not changed its spec or appdata during preflight.
            if cleanup_candidate(&u).await.is_ok()
                && verified(&u.old, &d).await.is_ok()
                && restart_old(&u).await.is_ok()
            {
                let _ = save(&u.old);
            }
        }
    });
    Ok(Json(result))
}
pub(super) async fn current_image() -> Result<String> {
    let self_id = std::env::var("HOSTNAME").map_err(|_| unavailable())?;
    let c = engine(&format!("/containers/{self_id}/json")).await?;
    let image = immutable(&c)?;
    // Keep the trusted worker image addressable when the bootstrap tag advances.
    request(
        Method::POST,
        &format!(
            "/images/{image}/tag?repo=thelxinoe-controller-worker&tag={}",
            &image[7..]
        ),
        None,
    )
    .await?;
    Ok(image)
}
pub(super) async fn start(container: &str) -> Result<()> {
    request(
        Method::POST,
        &format!("/containers/{container}/start"),
        None,
    )
    .await
    .map(|_| ())
}
pub(super) async fn stop(container: &str) -> Result<()> {
    request(
        Method::POST,
        &format!("/containers/{container}/stop?t=30"),
        None,
    )
    .await
    .map(|_| ())
}
pub(super) async fn remove(container: &str) -> Result<()> {
    match request(
        Method::DELETE,
        &format!("/containers/{container}?force=true&v=false"),
        None,
    )
    .await
    {
        Ok(_) | Err((StatusCode::NOT_FOUND, _)) => Ok(()),
        Err(e) => Err(e),
    }
}
async fn restart_old(u: &Update) -> Result<()> {
    if u.was_running {
        start(&u.old.container).await?;
    }
    Ok(())
}
fn host_path(d: &Deployment, u: &Update, leaf: &str) -> String {
    format!("{}/updates/{}/{leaf}", d.appdata_source, u.id)
}
async fn worker(
    d: &Deployment,
    u: &Update,
    source: &str,
    destination: &str,
    restore: bool,
    label: &str,
) -> Result<()> {
    copy_state(d, &u.id, source, destination, restore, label).await
}
pub(super) async fn copy_state(
    d: &Deployment,
    operation: &str,
    source: &str,
    destination: &str,
    restore: bool,
    label: &str,
) -> Result<()> {
    let name = format!("thelxinoe-state-{}-{label}", &operation[..8]);
    let spec = json!({"Image":current_image().await?,"Cmd":[if restore {"snapshot-restore"} else {"snapshot-copy"}],"Healthcheck":{"Test":["NONE"]},"Labels":{"app.thelxinoe.update":operation,"app.thelxinoe.deployment":d.id},"HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,"CapDrop":["ALL"],"CapAdd":["CHOWN","FOWNER","DAC_OVERRIDE"],"SecurityOpt":["no-new-privileges:true"],"Memory":536870912,"NanoCpus":1000000000u64,"PidsLimit":32,"Mounts":[{"Type":"bind","Source":source,"Target":"/source","ReadOnly":true},{"Type":"bind","Source":destination,"Target":"/destination"}]}});
    let value = request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(spec),
    )
    .await?;
    let container = value["Id"].as_str().ok_or_else(unavailable)?;
    let result = async {
        start(container).await?;
        wait(container, 600).await
    }
    .await;
    let cleaned = remove(container).await;
    result?;
    cleaned
}
pub(super) async fn wait(container: &str, seconds: u64) -> Result<()> {
    for _ in 0..seconds {
        let raw = engine(&format!("/containers/{container}/json")).await?;
        if raw["State"]["Running"] == false {
            return if raw["State"]["ExitCode"] == 0 {
                Ok(())
            } else {
                Err(conflict("Isolated worker failed its contract"))
            };
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(conflict("Isolated worker exceeded its time limit"))
}
async fn snapshot(d: &Deployment, u: &Update, leaf: &str) -> Result<()> {
    let raw = verified(&u.old, d).await?;
    let config = mount(&raw, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?;
    // Installed appdata is inside our dedicated deployment mount; adoption
    // additionally excludes host system roots and media-overlapping trees.
    let installed = std::path::Path::new(config).starts_with(
        std::path::Path::new(&d.appdata_source)
            .join("services")
            .join(&u.service)
            .join("appdata"),
    );
    if !installed && !policy::appdata_isolated(config, &d.media_source) {
        return Err(conflict(
            "Appdata overlaps media or a host system directory",
        ));
    }
    if raw["State"]["Running"] == true {
        stop(&u.old.container).await?;
    }
    let stopped = engine(&format!("/containers/{}/json", u.old.container)).await?;
    if stopped["State"]["Running"] != false {
        return Err(conflict("Service did not stop for a consistent snapshot"));
    }
    let source = mount(&raw, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?;
    persisted(std::fs::create_dir_all(path(&u.id).join(leaf)).map_err(Into::into))?;
    worker(d, u, source, &host_path(d, u, leaf), false, leaf).await
}
async fn check(d: &Deployment, u: &mut Update) -> Result<()> {
    request(
        Method::POST,
        &format!("/images/create?fromImage={}", u.candidate),
        None,
    )
    .await?;
    let image = engine(&format!("/images/{}/json", u.candidate)).await?;
    if image["Architecture"] != "amd64" || image["Os"] != "linux" {
        return Err(conflict("Candidate platform is unsupported"));
    }
    u.stage = "snapshotting".into();
    write(u)?;
    snapshot(d, u, "snapshot").await?;
    u.snapshot_complete = true;
    write(u)?;
    restart_old(u).await?;
    persisted(std::fs::create_dir_all(path(&u.id).join("clone")).map_err(Into::into))?;
    worker(
        d,
        u,
        &host_path(d, u, "snapshot"),
        &host_path(d, u, "clone"),
        false,
        "clone",
    )
    .await?;
    u.stage = "preflight".into();
    write(u)?;
    match candidate(d, u, &host_path(d, u, "clone")).await {
        Ok(()) => {
            u.classification = "compatible".into();
            u.stage = "ready".into();
        }
        Err(error) => {
            u.classification = "incompatible".into();
            u.stage = "blocked".into();
            u.error = Some(error.1.into());
        }
    }
    cleanup_candidate(u).await?;
    save(&u.old)?;
    write(u)
}
async fn candidate(d: &Deployment, u: &mut Update, config: &str) -> Result<()> {
    let data = path(&u.id).join("scratch-data");
    for sub in ["movies", "tv", "music", "downloads"] {
        persisted(std::fs::create_dir_all(data.join(sub)).map_err(Into::into))?;
    }
    let mut spec = json!({"Image":u.candidate,"Env":["PUID=10001","PGID=10001","TZ=UTC"],"Labels":{"app.thelxinoe.update":u.id,"app.thelxinoe.deployment":d.id},"HostConfig":{"NetworkMode":"none","CapDrop":["ALL"],"CapAdd":["CHOWN","DAC_OVERRIDE","FOWNER","SETUID","SETGID","KILL"],"SecurityOpt":["no-new-privileges:true"],"Memory":2147483648u64,"NanoCpus":2000000000u64,"PidsLimit":256,"Mounts":[{"Type":"bind","Source":config,"Target":"/config"},{"Type":"bind","Source":host_path(d,u,"scratch-data"),"Target":"/media"}]}});
    // Never inherit host ports, sockets, extra mounts, production networks or commands.
    spec["Healthcheck"] = json!({"Test":["NONE"]});
    spec["HostConfig"]["ExtraHosts"] = json!([
        "thelxinoe-radarr:127.0.0.1",
        "thelxinoe-sonarr:127.0.0.1",
        "thelxinoe-lidarr:127.0.0.1",
        "thelxinoe-nzbget:127.0.0.1",
        "thelxinoe-prowlarr:127.0.0.1"
    ]);
    let raw = request(
        Method::POST,
        &format!("/containers/create?name=thelxinoe-candidate-{}", &u.id[..8]),
        Some(spec),
    )
    .await?;
    let container = raw["Id"].as_str().ok_or_else(unavailable)?.to_owned();
    u.candidate_container = Some(container.clone());
    write(u)?;
    start(&container).await?;
    contract(d, u, config, &container, true).await
}
async fn contract(
    d: &Deployment,
    u: &Update,
    config: &str,
    container: &str,
    isolated: bool,
) -> Result<()> {
    let checker = json!({"Image":current_image().await?,"User":"10001:10001","Cmd":[if isolated {"adapter-contract"} else {"adapter-health"},u.old.kind],"Healthcheck":{"Test":["NONE"]},"Labels":{"app.thelxinoe.update":u.id,"app.thelxinoe.deployment":d.id},"HostConfig":{"NetworkMode":format!("container:{container}"),"ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Memory":268435456,"NanoCpus":1000000000u64,"PidsLimit":32,"Mounts":[{"Type":"bind","Source":config,"Target":"/config","ReadOnly":true}]}});
    let raw = request(
        Method::POST,
        &format!("/containers/create?name=thelxinoe-contract-{}", &u.id[..8]),
        Some(checker),
    )
    .await?;
    let checker = raw["Id"].as_str().ok_or_else(unavailable)?;
    let outcome = async {
        start(checker).await?;
        wait(checker, 200).await
    }
    .await;
    let removed = remove(checker).await;
    outcome?;
    removed
}
async fn cleanup_candidate(u: &Update) -> Result<()> {
    // Covers a crash between Docker creation and persistence of its returned ID.
    let containers = engine("/containers/json?all=true").await?;
    for c in containers.as_array().ok_or_else(unavailable)? {
        if c["Labels"]["app.thelxinoe.update"] == u.id {
            remove(c["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    Ok(())
}

pub(super) async fn activate(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let guard = runtime
        .0
        .try_service(&load(&read(&key)?.service)?.kind)
        .map_err(|_| conflict("This service or the deployment has an active operation"))?;
    let d = bootstrap().await?;
    let mut u = read(&key)?;
    let mut s = load(&u.service)?;
    if u.stage != "ready"
        || u.classification != "compatible"
        || s.phase != "active"
        || s.container != u.old.container
        || s.image != u.old.image
    {
        return Err(conflict("A current compatible preflight is required"));
    }
    let observed = verified(&s, &d).await?;
    // Preflight may have happened hours ago. Preserve the running state at
    // activation, including a deliberate stop since the earlier snapshot.
    u.was_running = observed["State"]["Running"]
        .as_bool()
        .ok_or_else(unavailable)?;
    u.stage = "recovery-snapshot".into();
    write(&u)?;
    s.phase = "updating".into();
    s.active_update = Some(u.id.clone());
    save(&s)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        if let Err(error) = replace(&d, &mut u).await {
            u.error = Some(error.1.into());
            if u.activation_crossed {
                u.stage = "runtime-failure".into();
            } else if rollback(&d, &mut u).await.is_ok() {
                u.stage = "rolled-back".into();
            } else {
                u.stage = "recovery-required".into();
            }
            let _ = write(&u);
        }
    });
    Ok(Json(result))
}
async fn replace(d: &Deployment, u: &mut Update) -> Result<()> {
    snapshot(d, u, "recovery").await?;
    u.recovery_complete = true;
    u.stage = "isolated-live-validation".into();
    write(u)?;
    let raw = verified(&u.old, d).await?;
    let source = mount(&raw, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    candidate(d, u, &source).await?;
    cleanup_candidate(u).await?;
    // Stopped original remains available until the accepted replacement is recorded.
    request(
        Method::POST,
        &format!(
            "/containers/{}/rename?name={}-prior-{}",
            u.old.container,
            u.old.name,
            &u.id[..8]
        ),
        None,
    )
    .await?;
    let mut spec = u.old.spec.clone();
    spec["Image"] = json!(u.candidate);
    let replacement = request(
        Method::POST,
        &format!("/containers/create?name={}", u.old.name),
        Some(spec.clone()),
    )
    .await?;
    let container = replacement["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    u.replacement = Some(container.clone());
    u.stage = "activating".into();
    // Commit the boundary before any request that permits production side effects.
    u.activation_crossed = true;
    write(u)?;
    let started = if u.was_running {
        start(&container).await
    } else {
        Ok(())
    };
    let raw = engine(&format!("/containers/{container}/json")).await?;
    let mut accepted = u.old.clone();
    accepted.container = container.clone();
    accepted.image = u.candidate.clone();
    accepted.spec = spec;
    accepted.expected = policy::fingerprint(&raw);
    accepted.phase = "active".into();
    save(&accepted)?;
    started?;
    if u.was_running {
        contract(d, u, &source, &container, false).await?;
    }
    u.stage = "committed".into();
    write(u)?;
    // Keep the stopped original and recovery snapshot for explicit recovery, never automatic post-activation rollback.
    Ok(())
}
async fn rollback(d: &Deployment, u: &mut Update) -> Result<()> {
    if u.activation_crossed {
        return Err(conflict(
            "Production activation requires explicit recovery planning",
        ));
    }
    cleanup_candidate(u).await?;
    if let Some(c) = &u.replacement {
        remove(c).await?;
    }
    let raw = verified(&u.old, d).await?;
    stop(&u.old.container).await?;
    if u.recovery_complete {
        let source = mount(&raw, "/config")?["Source"]
            .as_str()
            .ok_or_else(unavailable)?;
        worker(d, u, &host_path(d, u, "recovery"), source, true, "restore").await?;
    }
    if raw["Name"].as_str().unwrap_or("").trim_start_matches('/') != u.old.name {
        request(
            Method::POST,
            &format!("/containers/{}/rename?name={}", u.old.container, u.old.name),
            None,
        )
        .await?;
    }
    restart_old(u).await?;
    save(&u.old)
}
pub(super) async fn recover(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let _guard = runtime
        .0
        .try_service(&load(&read(&key)?.service)?.kind)
        .map_err(|_| conflict("This service or the deployment has an active operation"))?;
    let d = bootstrap().await?;
    let mut u = read(&key)?;
    let current = load(&u.service)?;
    if current.container != u.old.container
        || current.image != u.old.image
        || current.active_update.as_ref().is_some_and(|id| id != &u.id)
    {
        return Err(conflict(
            "A newer accepted service prevents recovery of this old attempt",
        ));
    }
    if u.activation_crossed || ["committed", "rolled-back", "ready"].contains(&u.stage.as_str()) {
        return Err(conflict("This update cannot use pre-activation recovery"));
    }
    rollback(&d, &mut u).await?;
    u.stage = "rolled-back".into();
    u.error = None;
    write(&u)?;
    Ok(Json(public(&u)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn activation_boundary_rejects_automatic_rollback_before_any_docker_call() {
        let mut update:Update=serde_json::from_value(json!({"id":"test","service":"service","candidate":"candidate","stage":"runtime-failure","classification":"compatible","error":null,"old":{"id":"service","kind":"radarr","container":"old","name":"old","image":"old","phase":"active","spec":{},"expected":{}},"was_running":true,"snapshot_complete":true,"recovery_complete":true,"activation_crossed":true,"candidate_container":null,"replacement":"new"})).unwrap();
        let deployment:Deployment=serde_json::from_value(json!({"id":"deployment","generation":1,"version":"test","server":{},"controller":{},"network":"test","media_source":"/media","appdata_source":"/state"})).unwrap();
        assert_eq!(
            rollback(&deployment, &mut update).await.unwrap_err().0,
            StatusCode::CONFLICT
        );
    }
}
