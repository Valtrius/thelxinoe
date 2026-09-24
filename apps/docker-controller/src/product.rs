//! First-party releases. Journals, image pins and recovery live outside server SQLite.
use super::*;
use reqwest::Method;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use thelxinoe_releases::{Envelope, Manifest};

#[derive(Clone, Serialize, Deserialize)]
struct Update {
    id: String,
    stage: String,
    error: Option<String>,
    manifest: Option<Manifest>,
    envelope: Option<Envelope>,
    old: Deployment,
    source: String,
    snapshot_ready: bool,
    recovery_tested: bool,
    activation_crossed: bool,
    next_server: Option<String>,
    next_controller: Option<String>,
    generation: Option<u64>,
    #[serde(default)]
    restoring_release: bool,
    #[serde(default)]
    retiring: Option<Deployment>,
    #[serde(default)]
    archive: Option<String>,
}
fn root() -> PathBuf {
    store::root().join("product-updates")
}
fn dir(key: &str) -> PathBuf {
    root().join(key)
}
fn read(key: &str) -> Result<Update> {
    id(key)?;
    persisted(store::read(&dir(key).join("update.json")))
}
fn write(u: &Update) -> Result<()> {
    persisted(store::write_json(&dir(&u.id).join("update.json"), u))
}
fn public(u: &Update) -> Value {
    json!({"id":u.id,"stage":u.stage,"error":u.error,"version":u.manifest.as_ref().map(|m|m.version.as_str()).unwrap_or(&u.old.version),"previous_version":u.old.version,"recovery":"full-state-restore","snapshot_ready":u.snapshot_ready,"recovery_tested":u.recovery_tested,"activation_crossed":u.activation_crossed,"generation":u.generation,"archive":u.archive})
}
fn all() -> Result<Vec<Update>> {
    let mut items = vec![];
    if root().is_dir() {
        for e in std::fs::read_dir(root()).map_err(|_| unavailable())? {
            let e = e.map_err(|_| unavailable())?;
            if e.path().join("update.json").is_file() {
                items.push(read(&e.file_name().to_string_lossy())?);
            }
        }
    }
    Ok(items)
}
pub(super) fn recovery_stage(key: &str) -> Option<String> {
    read(key).ok().map(|u| u.stage)
}
/// Only backups checked against a locally accepted historical descriptor reach here.
pub(super) async fn restore_archive(
    archive: String,
    operation: String,
    desired: Deployment,
    current: Deployment,
    source: String,
) -> Result<()> {
    let mut u = Update {
        id: operation,
        stage: "restoring-release".into(),
        error: None,
        manifest: None,
        envelope: None,
        old: desired,
        source,
        snapshot_ready: false,
        recovery_tested: false,
        activation_crossed: false,
        next_server: None,
        next_controller: None,
        generation: None,
        restoring_release: true,
        retiring: Some(current),
        archive: Some(archive),
    };
    private(&dir(&u.id))?;
    // Capture the authenticated archive state, never the migrated database.
    copy(&u, &u.source, "rollback", false).await?;
    let schema = verify_state(&u, "rollback").await?;
    copy(&u, &host(&u, "rollback"), "clone", false).await?;
    server_check(
        &u,
        "clone",
        &immutable(&u.old.server)?,
        &u.old.version,
        schema,
    )
    .await?;
    u.snapshot_ready = true;
    u.recovery_tested = true;
    write(&u)?;
    if let Err(e) = handoff(&mut u).await {
        u.stage = "recovery-required".into();
        u.error = Some(e.1.into());
        write(&u)?;
        return Err(e);
    }
    Ok(())
}
fn host(u: &Update, leaf: &str) -> String {
    format!("{}/product-updates/{}/{leaf}", u.old.appdata_source, u.id)
}
fn private(path: &std::path::Path) -> Result<()> {
    persisted(std::fs::create_dir_all(path).map_err(Into::into))?;
    persisted(
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(Into::into),
    )
}
fn trust() -> Result<String> {
    let path =
        std::env::var("THELXINOE_RELEASE_KEY_FILE").unwrap_or("/etc/thelxinoe/release.pub".into());
    std::fs::read_to_string(path)
        .map_err(|_| conflict("Configure the release signing public key on the controller"))
}
pub(super) fn deployment_matches(expected: &Value, actual: &Value) -> bool {
    let normalized = |value: &Value| {
        let mut value = value.clone();
        // Docker reports an omitted OOM option as either null or false across
        // create/start/recreate. Both keep the kernel OOM killer enabled.
        if value["HostConfig"]["OomKillDisable"].is_null() {
            value["HostConfig"]["OomKillDisable"] = json!(false);
        }
        policy::fingerprint(&value)
    };
    normalized(expected) == normalized(actual)
}
pub(super) async fn list() -> Result<Json<Value>> {
    Ok(Json(
        json!({"items":all()?.iter().map(public).collect::<Vec<_>>(),"configured":trust().is_ok()}),
    ))
}
#[derive(Deserialize)]
pub(super) struct Prepare {
    envelope: Envelope,
}
pub(super) async fn preflight(
    State(runtime): State<Runtime>,
    Json(input): Json<Prepare>,
) -> Result<Json<Value>> {
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let manifest = thelxinoe_releases::verify(&input.envelope, &trust()?)
        .map_err(|_| bad("Release signature or manifest is invalid"))?;
    let d = bootstrap().await?;
    let current = engine(&format!(
        "/containers/{}/json",
        d.server["Id"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    if !deployment_matches(&d.server, &current) {
        return Err(conflict("First-party drift blocks release preparation"));
    }
    if all()?.iter().any(|u| {
        !matches!(
            u.stage.as_str(),
            "ready" | "blocked" | "recovered" | "committed" | "restored" | "runtime-failure"
        )
    }) {
        return Err(conflict("Recover the unfinished product update first"));
    }
    let source = policy::host_path(
        mount(&current, "/var/lib/thelxinoe")?["Source"]
            .as_str()
            .ok_or_else(unavailable)?,
    )
    .ok_or_else(unavailable)?
    .to_string_lossy()
    .into_owned();
    if !policy::appdata_isolated(&source, &d.media_source) {
        return Err(conflict(
            "Server state must be isolated from media and system files",
        ));
    }
    let mut u = Update {
        id: thelxinoe_core::id(),
        stage: "preparing".into(),
        error: None,
        manifest: Some(manifest),
        envelope: Some(input.envelope),
        old: d,
        source,
        snapshot_ready: false,
        recovery_tested: false,
        activation_crossed: false,
        next_server: None,
        next_controller: None,
        generation: None,
        restoring_release: false,
        retiring: None,
        archive: None,
    };
    private(&dir(&u.id))?;
    write(&u)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = prepare(&mut u).await {
            u.stage = "blocked".into();
            u.error = Some(e.1.into());
            let _ = write(&u);
            let _ = updates::start(u.old.server["Id"].as_str().unwrap_or("")).await;
        }
    });
    Ok(Json(result))
}
async fn pull(image: &thelxinoe_releases::Image) -> Result<()> {
    // Cached immutable images also support offline recovery and release fixtures.
    let raw = match engine(&format!("/images/{}/json", image.reference)).await {
        Ok(v) => v,
        Err((StatusCode::NOT_FOUND, _)) => {
            request(
                Method::POST,
                &format!("/images/create?fromImage={}", image.reference),
                None,
            )
            .await?;
            engine(&format!("/images/{}/json", image.reference)).await?
        }
        Err(e) => return Err(e),
    };
    if raw["Id"] != image.config_digest || raw["Architecture"] != "amd64" || raw["Os"] != "linux" {
        return Err(conflict(
            "Release image does not match the signed Linux x86-64 identity",
        ));
    }
    Ok(())
}
async fn worker(u: &Update, label: &str, spec: Value) -> Result<()> {
    let name = format!("thelxinoe-product-{}-{label}", &u.id[..8]);
    let container = request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(spec),
    )
    .await?;
    let key = container["Id"].as_str().ok_or_else(unavailable)?;
    let result = async {
        updates::start(key).await?;
        updates::wait(key, 180).await
    }
    .await;
    let cleaned = updates::remove(key).await;
    result?;
    cleaned
}
fn worker_spec(u: &Update, image: &str, command: &str, mounts: Value) -> Value {
    json!({"Image":image,"Cmd":[command],"Labels":{"app.thelxinoe.product-update":u.id,"app.thelxinoe.deployment":u.old.id},"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Memory":1073741824u64,"NanoCpus":2000000000u64,"PidsLimit":64,"Tmpfs":{"/tmp":"rw,noexec,nosuid,size=64m","/var/cache/thelxinoe":"rw,nosuid,size=64m,uid=10001,gid=10001"},"Mounts":mounts}})
}
async fn verify_state(u: &Update, leaf: &str) -> Result<u32> {
    let mut spec = worker_spec(
        u,
        &updates::current_image().await?,
        "verify-state",
        json!([{"Type":"bind","Source":host(u,leaf),"Target":"/state"}]),
    );
    // SQLite may create WAL/SHM even for a read-only integrity connection. Use the
    // server UID so the later isolated migration can write those sidecars.
    spec["User"] = u.old.server["Config"]["User"].clone();
    worker(u, "verify", spec).await?;
    let report: Value = persisted(store::read(
        &dir(&u.id).join(leaf).join(".snapshot-validation.json"),
    ))?;
    report["schema"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(unavailable)
}
async fn server_check(
    u: &Update,
    leaf: &str,
    image: &str,
    version: &str,
    schema: u32,
) -> Result<()> {
    let report = dir(&u.id).join(leaf).join(".release-validation.json");
    if report.exists() {
        persisted(std::fs::remove_file(&report).map_err(Into::into))?;
    }
    let spec = worker_spec(
        u,
        image,
        "validate-state",
        json!([{"Type":"bind","Source":host(u,leaf),"Target":"/var/lib/thelxinoe"}]),
    );
    worker(u, "validate", spec).await?;
    let value: Value = persisted(store::read(&report))?;
    if value["version"] != version
        || value["schema"] != schema
        || u.manifest.as_ref().is_some_and(|manifest| {
            version == manifest.version
                && value["api"]
                    .as_u64()
                    .is_none_or(|n| !manifest.api.contains(n as u32))
        })
    {
        return Err(conflict(
            "Server migration contract does not match the signed release",
        ));
    }
    Ok(())
}
async fn copy(u: &Update, source: &str, leaf: &str, restore: bool) -> Result<()> {
    private(&dir(&u.id).join(leaf))?;
    updates::copy_state(&u.old, &u.id, source, &host(u, leaf), restore, leaf).await
}
async fn prepare(u: &mut Update) -> Result<()> {
    let manifest = u.manifest.clone().ok_or_else(unavailable)?;
    pull(&manifest.server).await?;
    pull(&manifest.controller).await?;
    // Retain old images independently of mutable bootstrap tags.
    for (component, raw) in [("server", &u.old.server), ("controller", &u.old.controller)] {
        let image = immutable(raw)?;
        request(
            Method::POST,
            &format!(
                "/images/{image}/tag?repo=thelxinoe-recovery-{component}&tag={}",
                &image[7..]
            ),
            None,
        )
        .await?;
    }
    u.stage = "snapshotting".into();
    write(u)?;
    updates::stop(u.old.server["Id"].as_str().ok_or_else(unavailable)?).await?;
    copy(u, &u.source, "preflight-snapshot", false).await?;
    updates::start(u.old.server["Id"].as_str().ok_or_else(unavailable)?).await?;
    let schema = verify_state(u, "preflight-snapshot").await?;
    manifest
        .candidate(&u.old.version, schema, thelxinoe_core::now())
        .map_err(|_| {
            conflict("Release version, validity, schema or recovery protocol is incompatible")
        })?;
    u.stage = "validating".into();
    write(u)?;
    copy(u, &host(u, "preflight-snapshot"), "clone", false).await?;
    server_check(
        u,
        "clone",
        &manifest.server.config_digest,
        &manifest.version,
        manifest.migration.target,
    )
    .await?;
    // Exercise the real restore worker against migrated state, then run the old binary.
    copy(u, &host(u, "preflight-snapshot"), "clone", true).await?;
    server_check(
        u,
        "clone",
        &immutable(&u.old.server)?,
        &u.old.version,
        schema,
    )
    .await?;
    private(&dir(&u.id).join("probe"))?;
    let spec = worker_spec(
        u,
        &manifest.controller.config_digest,
        "controller-probe",
        json!([{"Type":"bind","Source":host(u,"probe"),"Target":"/probe"}]),
    );
    worker(u, "probe", spec).await?;
    let probe: Value = persisted(store::read(&dir(&u.id).join("probe/controller.json")))?;
    if probe["version"] != manifest.version || probe["recovery_protocol"] != 1 {
        return Err(conflict("Successor controller failed its release contract"));
    }
    u.recovery_tested = true;
    u.stage = "ready".into();
    write(u)
}
fn recreate(raw: &Value, image: &str) -> Result<Value> {
    let mut spec = raw["Config"].clone();
    spec["Image"] = json!(image);
    spec["Hostname"] = json!("");
    spec["HostConfig"] = raw["HostConfig"].clone();
    let networks = raw["NetworkSettings"]["Networks"]
        .as_object()
        .ok_or_else(unavailable)?
        .iter()
        .map(|(name, n)| {
            (
                name.clone(),
                json!({"Aliases":n["Aliases"],"IPAMConfig":n["IPAMConfig"]}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    spec["NetworkingConfig"] = json!({"EndpointsConfig":networks});
    Ok(spec)
}
async fn create(name: &str, spec: Value) -> Result<String> {
    request(
        Method::POST,
        &format!("/containers/create?name={name}"),
        Some(spec),
    )
    .await?["Id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(unavailable)
}
async fn rename(container: &str, name: &str) -> Result<()> {
    if engine(&format!("/containers/{container}/json")).await?["Name"] == format!("/{name}") {
        return Ok(());
    }
    request(
        Method::POST,
        &format!("/containers/{container}/rename?name={name}"),
        None,
    )
    .await
    .map(|_| ())
}
fn name(raw: &Value) -> Result<&str> {
    raw["Name"]
        .as_str()
        .map(|n| n.trim_start_matches('/'))
        .ok_or_else(unavailable)
}
fn next_generation() -> Result<u64> {
    Ok(std::fs::read_dir(store::root().join("generations"))
        .map_err(|_| unavailable())?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1)
}
fn commit(d: &Deployment) -> Result<()> {
    let project = d.controller["Config"]["Labels"]["com.docker.compose.project"]
        .as_str()
        .unwrap_or("thelxinoe");
    let compose = json!({"name":project,"services":{"server":compose_service(&d.server)?,"controller":compose_service(&d.controller)?},"networks":{"media":{"external":true,"name":d.network}}});
    persisted(store::commit_generation(
        &store::root(),
        d.generation,
        &serde_json::to_value(d).map_err(|_| unavailable())?,
        &compose,
    ))
}
pub(super) async fn activate(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let mut u = read(&key)?;
    if u.stage != "ready" || !u.recovery_tested {
        return Err(conflict(
            "A tested recovery path and compatible preflight are required",
        ));
    }
    let d = bootstrap().await?;
    if d.generation != u.old.generation {
        return Err(conflict("Deployment changed since preflight"));
    }
    let current = engine(&format!(
        "/containers/{}/json",
        u.old.server["Id"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    if !deployment_matches(&u.old.server, &current) {
        return Err(conflict("First-party drift blocks activation"));
    }
    u.stage = "preparing-activation".into();
    write(&u)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = replace(&mut u).await {
            u.error = Some(e.1.into());
            if recover_old(&mut u).await.is_err() {
                u.stage = "recovery-required".into();
            }
            let _ = write(&u);
        }
    });
    Ok(Json(result))
}
async fn replace(u: &mut Update) -> Result<()> {
    let manifest = u.manifest.clone().ok_or_else(unavailable)?;
    updates::stop(u.old.server["Id"].as_str().ok_or_else(unavailable)?).await?;
    copy(u, &u.source, "rollback", false).await?;
    let schema = verify_state(u, "rollback").await?;
    manifest
        .candidate(&u.old.version, schema, thelxinoe_core::now())
        .map_err(|_| conflict("Release is no longer eligible"))?;
    u.snapshot_ready = true;
    u.stage = "isolated-migration".into();
    write(u)?;
    // Migrate the real state while entirely disconnected, after the verified rollback bundle exists.
    let mut spec = worker_spec(
        u,
        &manifest.server.config_digest,
        "validate-state",
        json!([{"Type":"bind","Source":u.source,"Target":"/var/lib/thelxinoe"}]),
    );
    spec["Labels"]["app.thelxinoe.validation"] = json!("true");
    worker(u, "live-validation", spec).await?;
    // Copy the report with trusted code, then inspect the version/schema contract.
    copy(u, &u.source, "validated", false).await?;
    let report: Value = persisted(store::read(
        &dir(&u.id).join("validated/.release-validation.json"),
    ))?;
    if report["version"] != manifest.version || report["schema"] != manifest.migration.target {
        return Err(conflict(
            "Isolated live-state validation returned an unexpected contract",
        ));
    }
    handoff(u).await
}
async fn handoff(u: &mut Update) -> Result<()> {
    let retiring = u.retiring.clone().unwrap_or_else(|| u.old.clone());
    let server_image = if u.restoring_release {
        immutable(&u.old.server)?
    } else {
        u.manifest
            .as_ref()
            .ok_or_else(unavailable)?
            .server
            .config_digest
            .clone()
    };
    let controller_image = if u.restoring_release {
        immutable(&u.old.controller)?
    } else {
        u.manifest
            .as_ref()
            .ok_or_else(unavailable)?
            .controller
            .config_digest
            .clone()
    };
    let version = if u.restoring_release {
        u.old.version.clone()
    } else {
        u.manifest.as_ref().ok_or_else(unavailable)?.version.clone()
    };
    for leaf in ["standby.json", "writer.json"] {
        let path = dir(&u.id).join(leaf);
        if path.exists() {
            persisted(std::fs::remove_file(path).map_err(Into::into))?;
        }
    }
    u.stage = "creating-successor".into();
    write(u)?;
    let server_name = name(&u.old.server)?.to_owned();
    let controller_name = name(&u.old.controller)?.to_owned();
    rename(
        retiring.server["Id"].as_str().ok_or_else(unavailable)?,
        &format!("{server_name}-retired-{}", &u.id[..8]),
    )
    .await?;
    let mut spec = recreate(&u.old.server, &server_image)?;
    spec["Labels"]["app.thelxinoe.product-update"] = json!(u.id);
    spec["Labels"]["app.thelxinoe.deployment"] = json!(u.old.id);
    u.next_server = Some(create(&server_name, spec).await?);
    write(u)?;
    let mut spec = recreate(&u.old.controller, &controller_image)?;
    spec["Labels"]["app.thelxinoe.product-update"] = json!(u.id);
    spec["Labels"]["app.thelxinoe.deployment"] = json!(u.old.id);
    spec["Env"]
        .as_array_mut()
        .ok_or_else(unavailable)?
        .retain(|v| !v.as_str().unwrap_or("").starts_with("THELXINOE_HANDOFF="));
    spec["Env"]
        .as_array_mut()
        .unwrap()
        .push(json!(format!("THELXINOE_HANDOFF={}", u.id)));
    spec["HostConfig"]["RestartPolicy"] = json!({"Name":"unless-stopped"});
    rename(
        retiring.controller["Id"].as_str().ok_or_else(unavailable)?,
        &format!("{controller_name}-retired-{}", &u.id[..8]),
    )
    .await?;
    u.next_controller = Some(create(&controller_name, spec).await?);
    write(u)?;
    updates::start(u.next_controller.as_deref().unwrap()).await?;
    for n in 0..60 {
        if dir(&u.id).join("standby.json").is_file() {
            break;
        }
        if n == 59 {
            return Err(conflict("Successor controller did not enter standby"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let ready: Value = persisted(store::read(&dir(&u.id).join("standby.json")))?;
    if ready["version"] != version
        || !ready["container"].as_str().is_some_and(|short| {
            short.len() >= 12
                && u.next_controller
                    .as_deref()
                    .is_some_and(|full| full.starts_with(short))
        })
    {
        return Err(conflict("Successor standby identity did not match"));
    }
    let mut d = u.old.clone();
    d.generation = next_generation()?;
    d.version = version;
    d.server = engine(&format!(
        "/containers/{}/json",
        u.next_server.as_deref().unwrap()
    ))
    .await?;
    normalize_stopped_server(&mut d.server).await?;
    d.controller = engine(&format!(
        "/containers/{}/json",
        u.next_controller.as_deref().unwrap()
    ))
    .await?;
    u.generation = Some(d.generation);
    u.stage = "handoff".into();
    write(u)?;
    commit(&d)?;
    // No further Docker call is allowed by this task. Main drops both leases before waiting.
    crate::lease::HANDOFF.notify_one();
    Ok(())
}
async fn clean_workers(u: &Update) -> Result<()> {
    for c in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if (c["Labels"]["app.thelxinoe.product-update"] == u.id
            || c["Labels"]["app.thelxinoe.update"] == u.id)
            && c["Labels"]["app.thelxinoe.deployment"] == u.old.id
            && c["Labels"]["app.thelxinoe.component"].is_null()
        {
            updates::remove(c["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    Ok(())
}
async fn recover_old(u: &mut Update) -> Result<()> {
    if u.restoring_release {
        return Err(conflict(
            "An interrupted explicit release restore requires recovery continuation",
        ));
    }
    if u.activation_crossed {
        return Err(conflict("Production activation requires explicit recovery"));
    }
    clean_workers(u).await?;
    clean_successors(u, &u.old).await?;
    if u.snapshot_ready {
        updates::stop(u.old.server["Id"].as_str().ok_or_else(unavailable)?).await?;
        updates::copy_state(
            &u.old,
            &u.id,
            &host(u, "rollback"),
            &u.source,
            true,
            "offline-recovery",
        )
        .await?;
    }
    rename(
        u.old.server["Id"].as_str().ok_or_else(unavailable)?,
        name(&u.old.server)?,
    )
    .await?;
    rename(
        u.old.controller["Id"].as_str().ok_or_else(unavailable)?,
        name(&u.old.controller)?,
    )
    .await?;
    let current = bootstrap().await?;
    if current.generation != u.old.generation {
        let mut restored = u.old.clone();
        restored.generation = next_generation()?;
        commit(&restored)?;
    }
    updates::start(u.old.server["Id"].as_str().ok_or_else(unavailable)?).await?;
    u.stage = "recovered".into();
    write(u)
}
#[derive(Deserialize)]
pub(super) struct Recover {
    confirm: bool,
}
pub(super) async fn recover(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Recover>,
) -> Result<Json<Value>> {
    if !input.confirm {
        return Err(bad(
            "Confirm restoring the complete pre-update server state",
        ));
    }
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let mut u = read(&key)?;
    if !u.activation_crossed && !u.restoring_release {
        recover_old(&mut u).await?;
        return Ok(Json(public(&u)));
    }
    let d = bootstrap().await?;
    if !u.snapshot_ready || (!u.restoring_release && Some(d.generation) != u.generation) {
        return Err(conflict(
            "This snapshot does not belong to the accepted release generation",
        ));
    }
    if !u.restoring_release {
        u.retiring = Some(d);
    }
    u.restoring_release = true;
    u.stage = "restoring-release".into();
    write(&u)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = restore_release(&mut u).await {
            u.stage = "recovery-required".into();
            u.error = Some(e.1.into());
            let _ = write(&u);
        }
    });
    Ok(Json(result))
}
async fn restore_release(u: &mut Update) -> Result<()> {
    // All inputs are retained controller state; the migrated SQLite database is never opened.
    let active = u.retiring.clone().ok_or_else(unavailable)?;
    clean_workers(u).await?;
    clean_successors(u, &active).await?;
    updates::stop(active.server["Id"].as_str().ok_or_else(unavailable)?).await?;
    updates::copy_state(
        &u.old,
        &u.id,
        &host(u, "rollback"),
        &u.source,
        true,
        "explicit-recovery",
    )
    .await?;
    u.activation_crossed = false;
    u.next_server = None;
    u.next_controller = None;
    write(u)?;
    handoff(u).await
}
async fn clean_successors(u: &Update, active: &Deployment) -> Result<()> {
    // Include creations whose Docker response was lost before their ID was journaled.
    for c in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if c["Labels"]["app.thelxinoe.product-update"] == u.id
            && c["Labels"]["app.thelxinoe.deployment"] == u.old.id
            && c["Id"] != active.server["Id"]
            && c["Id"] != active.controller["Id"]
        {
            updates::remove(c["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    Ok(())
}
/// Called before acquiring the lease. Standby only writes its own readiness record.
pub(crate) async fn standby() -> anyhow::Result<()> {
    let Ok(key) = std::env::var("THELXINOE_HANDOFF") else {
        return Ok(());
    };
    id(&key).map_err(|(_, e)| anyhow::anyhow!(e))?;
    let self_id = std::env::var("HOSTNAME")?;
    store::write_json(
        &dir(&key).join("standby.json"),
        &json!({"container":self_id,"version":thelxinoe_core::VERSION}),
    )?;
    Ok(())
}
/// A stale generation must never take the writer role, even after a normal Docker restart.
pub(crate) async fn accepted_writer() -> anyhow::Result<bool> {
    let file = store::root().join("desired-state.json");
    if !file.exists() {
        return Ok(std::env::var("THELXINOE_HANDOFF").is_err());
    }
    let mut d: Deployment = store::read(&file)?;
    let own = std::env::var("HOSTNAME")?;
    if d.controller["Id"]
        .as_str()
        .is_some_and(|id| id.starts_with(&own))
    {
        return Ok(true);
    }
    // Compose can recreate a pinned generation with new container IDs. Only accept its
    // original names, exact images and equivalent security/mount configuration.
    let controller = engine(&format!("/containers/{own}/json"))
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    let allow_rebuilt_image =
        std::env::var("THELXINOE_DEV_COMPOSE_REBUILD").is_ok_and(|value| value == "1");
    if !equivalent(&d.controller, &controller, allow_rebuilt_image) {
        return Ok(false);
    }
    let server_name = name(&d.server).map_err(|(_, e)| anyhow::anyhow!(e))?;
    let mut server = engine(&format!("/containers/{server_name}/json"))
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    if !equivalent(&d.server, &server, allow_rebuilt_image) {
        return Ok(false);
    }
    normalize_stopped_server(&mut server)
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    d.controller = controller;
    d.server = server;
    d.generation = next_generation().map_err(|(_, e)| anyhow::anyhow!(e))?;
    commit(&d).map_err(|(_, e)| anyhow::anyhow!(e))?;
    Ok(true)
}
async fn normalize_stopped_server(server: &mut Value) -> Result<()> {
    if server["State"]["Running"] == true {
        return Ok(());
    }
    // Docker fills these defaults only when a created container first starts.
    // Resolve them before accepting the stopped successor, so runtime startup
    // does not look like an outside configuration change.
    normalize_stopped_server_defaults(server);
    let network_names = server["NetworkSettings"]["Networks"]
        .as_object()
        .ok_or_else(unavailable)?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for network in network_names {
        let raw = engine(&format!("/networks/{network}")).await?;
        server["NetworkSettings"]["Networks"][&network]["NetworkID"] = raw["Id"].clone();
    }
    Ok(())
}
fn normalize_stopped_server_defaults(server: &mut Value) {
    if server["HostConfig"]["OomKillDisable"].is_null() {
        server["HostConfig"]["OomKillDisable"] = json!(false);
    }
}
fn same_image_identity(expected: &Value, actual: &Value) -> bool {
    match (
        expected["ImageManifestDescriptor"]["digest"].as_str(),
        actual["ImageManifestDescriptor"]["digest"].as_str(),
    ) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => {
            expected["Image"].as_str().is_some()
                && expected["Image"].as_str() == actual["Image"].as_str()
        }
    }
}
fn equivalent(expected: &Value, actual: &Value, allow_rebuilt_image: bool) -> bool {
    let same_image = same_image_identity(expected, actual);
    let same_configured_image = expected["Config"]["Image"].is_string()
        && expected["Config"]["Image"] == actual["Config"]["Image"];
    if (!same_image && !(allow_rebuilt_image && same_configured_image))
        || expected["Name"] != actual["Name"]
    {
        return false;
    }
    let env = |c: &Value| -> Option<std::collections::BTreeMap<String, String>> {
        let mut result = std::collections::BTreeMap::new();
        for item in c["Config"]["Env"].as_array()? {
            let (key, value) = item.as_str()?.split_once('=')?;
            if allow_rebuilt_image && key == "THELXINOE_DEV_COMPOSE_REBUILD" {
                continue;
            }
            if result.insert(key.into(), value.into()).is_some() {
                return None;
            }
        }
        Some(result)
    };
    if env(expected).is_none() || env(expected) != env(actual) {
        return false;
    }
    for key in [
        "Cmd",
        "Entrypoint",
        "User",
        "WorkingDir",
        "ExposedPorts",
        "Healthcheck",
    ] {
        if expected["Config"][key] != actual["Config"][key] {
            return false;
        }
    }
    for key in [
        "Privileged",
        "ReadonlyRootfs",
        "CapAdd",
        "CapDrop",
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
        "IpcMode",
        "SecurityOpt",
        "PortBindings",
    ] {
        if !policy::same_default(&expected["HostConfig"][key], &actual["HostConfig"][key]) {
            return false;
        }
    }
    let mount_set = |c: &Value| -> Option<std::collections::BTreeMap<String, (PathBuf, bool)>> {
        c["Mounts"]
            .as_array()?
            .iter()
            .map(|m| {
                Some((
                    m["Destination"].as_str()?.into(),
                    (
                        policy::host_path(m["Source"].as_str()?)?,
                        m["RW"].as_bool()?,
                    ),
                ))
            })
            .collect()
    };
    if mount_set(expected).is_none() || mount_set(expected) != mount_set(actual) {
        return false;
    }
    let networks = |c: &Value| {
        c["NetworkSettings"]["Networks"]
            .as_object()
            .map(|n| n.keys().cloned().collect::<Vec<_>>())
    };
    networks(expected) == networks(actual)
}
pub(crate) async fn startup() -> anyhow::Result<()> {
    for mut u in all().map_err(|(_, e)| anyhow::anyhow!(e))? {
        if matches!(
            u.stage.as_str(),
            "ready"
                | "blocked"
                | "recovered"
                | "committed"
                | "restored"
                | "runtime-failure"
                | "recovery-required"
        ) {
            continue;
        }
        let d = bootstrap().await.map_err(|(_, e)| anyhow::anyhow!(e))?;
        if u.stage == "handoff"
            && d.generation == u.generation.unwrap_or(0)
            && d.controller["Id"].as_str() == u.next_controller.as_deref()
        {
            store::write_json(
                &dir(&u.id).join("writer.json"),
                &json!({"generation":d.generation,"version":thelxinoe_core::VERSION}),
            )?;
            u.activation_crossed = true;
            u.stage = "activating".into();
            write(&u).map_err(|(_, e)| anyhow::anyhow!(e))?;
            let activated = async {
                let key = u.next_server.as_deref().ok_or_else(unavailable)?;
                updates::start(key).await?;
                for _ in 0..90 {
                    let raw = engine(&format!("/containers/{key}/json")).await?;
                    if raw["State"]["Health"]["Status"] == "healthy" {
                        return Ok(());
                    }
                    if raw["State"]["Running"] == false {
                        return Err(conflict("Accepted server exited"));
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(conflict("Accepted server health check timed out"))
            }
            .await;
            u.stage = if activated.is_ok() {
                if u.restoring_release {
                    "restored"
                } else {
                    "committed"
                }
            } else {
                "runtime-failure"
            }
            .into();
            if activated.is_err() {
                u.error =
                    Some("Accepted server could not start; explicit recovery is required".into());
            }
            write(&u).map_err(|(_, e)| anyhow::anyhow!(e))?;
            // The accepted descriptor is sufficient to recreate old images offline; retired
            // Compose containers must not compete with the accepted service names.
            let retired = u.retiring.as_ref().unwrap_or(&u.old);
            let _ = updates::remove(retired.server["Id"].as_str().unwrap_or("")).await;
            let _ = updates::remove(retired.controller["Id"].as_str().unwrap_or("")).await;
        } else if u.activation_crossed {
            u.stage = "runtime-failure".into();
            u.error = Some(
                "Server activation was interrupted; inspect health before explicit recovery".into(),
            );
            write(&u).map_err(|(_, e)| anyhow::anyhow!(e))?;
        } else if u.restoring_release {
            restore_release(&mut u)
                .await
                .map_err(|(_, e)| anyhow::anyhow!(e))?;
        } else {
            recover_old(&mut u)
                .await
                .map_err(|(_, e)| anyhow::anyhow!(e))?;
        }
    }
    Ok(())
}
/// The predecessor can recover an unacknowledged handoff only after reacquiring both leases.
pub(crate) async fn watchdog(runtime: &std::path::Path) -> anyhow::Result<bool> {
    for _ in 0..60 {
        if all()
            .map_err(|(_, e)| anyhow::anyhow!(e))?
            .iter()
            .filter(|u| u.stage == "handoff")
            .all(|u| dir(&u.id).join("writer.json").is_file())
        {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let _lease = crate::lease::Lease::acquire(runtime)?;
    for mut u in all().map_err(|(_, e)| anyhow::anyhow!(e))? {
        if u.stage == "handoff" && !dir(&u.id).join("writer.json").exists() {
            if u.restoring_release {
                // Keep the existing controller available for explicit continuation;
                // never start its newer server against the restored older database.
                let result = async {
                    let mut active = u.retiring.clone().ok_or_else(unavailable)?;
                    clean_successors(&u, &active).await?;
                    rename(active.server["Id"].as_str().ok_or_else(unavailable)?, name(&u.old.server)?).await?;
                    rename(active.controller["Id"].as_str().ok_or_else(unavailable)?, name(&u.old.controller)?).await?;
                    active.generation = next_generation()?;
                    commit(&active)?;
                    u.stage = "recovery-required".into();
                    u.error = Some("Recovery controller did not take over; resume recovery through the private controller API".into());
                    write(&u)
                }.await;
                result.map_err(|(_, e)| anyhow::anyhow!(e))?;
            } else {
                recover_old(&mut u)
                    .await
                    .map_err(|(_, e)| anyhow::anyhow!(e))?;
            }
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_oom_reporting_is_stable_but_disabling_it_is_drift() {
        let before = json!({"HostConfig":{"OomKillDisable":null}});
        let mut after = before.clone();
        after["HostConfig"]["OomKillDisable"] = json!(false);
        assert!(deployment_matches(&before, &after));
        after["HostConfig"]["OomKillDisable"] = json!(true);
        assert!(!deployment_matches(&before, &after));
    }
    #[test]
    fn recreated_generation_requires_exact_image_and_host_access() {
        let expected = json!({"Image":"sha256:accepted","ImageManifestDescriptor":{"digest":"sha256:manifest"},"Name":"/controller","Config":{"Image":"thelxinoe-controller:dev","Env":["SAFE=1"],"User":"0:10001"},"HostConfig":{"Privileged":false,"ReadonlyRootfs":true,"CapDrop":["ALL"]},"Mounts":[{"Destination":"/state","Source":"/srv/thelxinoe","RW":true}],"NetworkSettings":{"Networks":{"none":{}}}});
        assert!(equivalent(&expected, &expected, false));
        let mut platform_alias = expected.clone();
        platform_alias["Image"] = json!("sha256:platform-image");
        assert!(equivalent(&expected, &platform_alias, false));
        let mut no_manifest = expected.clone();
        no_manifest
            .as_object_mut()
            .unwrap()
            .remove("ImageManifestDescriptor");
        assert!(equivalent(&expected, &no_manifest, false));
        let mut different_image_without_manifest = no_manifest.clone();
        different_image_without_manifest["Image"] = json!("sha256:different-image");
        assert!(!equivalent(
            &expected,
            &different_image_without_manifest,
            false
        ));
        let mut renamed = expected.clone();
        renamed["Name"] = json!("/retired");
        assert!(!equivalent(&expected, &renamed, false));
        let mut changed = expected.clone();
        changed["ImageManifestDescriptor"]["digest"] = json!("sha256:new-manifest");
        changed["Image"] = json!("sha256:new-image");
        assert!(!equivalent(&expected, &changed, false));
        assert!(equivalent(&expected, &changed, true));
        changed["Config"]["Image"] = json!("other-controller:dev");
        assert!(!equivalent(&expected, &changed, true));
        let mut changed = expected.clone();
        changed["HostConfig"]["Privileged"] = json!(true);
        assert!(!equivalent(&expected, &changed, false));
        let mut changed = expected.clone();
        changed["Mounts"][0]["Source"] = json!("/etc");
        assert!(!equivalent(&expected, &changed, false));
        let mut changed = expected.clone();
        changed["Config"]["Env"] = json!(["SAFE=0"]);
        assert!(!equivalent(&expected, &changed, false));
        let mut ordered = expected.clone();
        ordered["Config"]["Env"] = json!(["A=1", "B=2"]);
        let mut reordered = ordered.clone();
        reordered["Config"]["Env"] = json!(["B=2", "A=1"]);
        assert!(equivalent(&ordered, &reordered, false));
        reordered["Config"]["Env"] = json!(["A=1", "B=2", "A=3"]);
        assert!(!equivalent(&ordered, &reordered, false));
    }
}
