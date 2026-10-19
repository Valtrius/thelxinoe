//! First-party releases. Journals, image pins and recovery live outside server SQLite.
use super::*;
use reqwest::Method;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use thelxinoe_releases::{Envelope, Manifest};

#[derive(Clone, Serialize, Deserialize)]
struct Update {
    id: String,
    #[serde(default)]
    created_at: i64,
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
    #[serde(default)]
    download: Option<Download>,
    /// Why a preparation or activation stopped: `environment` (Docker, disk,
    /// download), `interrupted` (a restart), `validation` (the release failed
    /// on this server's data) or `incompatible` (it cannot migrate this state).
    #[serde(default)]
    failure: Option<String>,
    /// The Compose `latest` tags point at the release this attempt installed.
    #[serde(default)]
    latest: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Download {
    component: String,
    received: u64,
    total: Option<u64>,
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
    json!({"id":u.id,"created_at":u.created_at,"stage":u.stage,"download":u.download,"error":u.error,"version":u.manifest.as_ref().map(|m|m.version.as_str()).unwrap_or(&u.old.version),"previous_version":u.old.version,"recovery":"full-state-restore","snapshot_ready":u.snapshot_ready,"recovery_tested":u.recovery_tested,"activation_crossed":u.activation_crossed,"generation":u.generation,"source_generation":u.old.generation,"archive":u.archive,"failure":u.failure})
}
fn all() -> Result<Vec<Update>> {
    let mut items = vec![];
    if root().is_dir() {
        for e in std::fs::read_dir(root()).map_err(|_| unavailable())? {
            let e = e.map_err(|_| unavailable())?;
            if e.path().join("update.json").is_file() {
                // One unreadable journal must not stop the controller or every
                // later update; it stays on disk for inspection.
                match read(&e.file_name().to_string_lossy()) {
                    Ok(u) => items.push(u),
                    Err((_, error)) => {
                        eprintln!(
                            "Skipping unreadable update journal {:?}: {error}",
                            e.file_name()
                        )
                    }
                }
            }
        }
    }
    items.sort_by_key(|u| std::cmp::Reverse(u.created_at));
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
        created_at: thelxinoe_core::now(),
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
        download: None,
        failure: None,
        latest: false,
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
        u.error = Some(e.1.to_string());
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
    policy::matches(&policy::fingerprint(expected), actual)
}
pub(super) async fn list(State(runtime): State<Runtime>) -> Result<Json<Value>> {
    // A missing journal is conclusive only when no command still owns the gate.
    // Keep the guard until after the journal read to reconcile lost POST replies.
    let guard = runtime.0.try_lock_owned().ok();
    let generation = if store::root().join("desired-state.json").is_file() {
        Some(
            persisted(store::read::<Deployment>(
                &store::root().join("desired-state.json"),
            ))?
            .generation,
        )
    } else {
        None
    };
    Ok(Json(
        json!({"items":all()?.iter().map(public).collect::<Vec<_>>(),"configured":trust().is_ok(),"generation":generation,"busy":guard.is_none()}),
    ))
}
#[derive(Deserialize)]
pub(super) struct Prepare {
    envelope: Envelope,
    request_id: Option<String>,
}
pub(super) async fn preflight(
    State(runtime): State<Runtime>,
    Json(input): Json<Prepare>,
) -> Result<Json<Value>> {
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let key = input.request_id.unwrap_or_else(thelxinoe_core::id);
    id(&key)?;
    // A server restart or lost response must not create a second operation.
    if dir(&key).join("update.json").is_file() {
        return Ok(Json(public(&read(&key)?)));
    }
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
            "ready"
                | "blocked"
                | "recovered"
                | "committed"
                | "restored"
                | "runtime-failure"
                | "superseded"
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
    if !server_state_isolated(&source, &d.media_source).await? {
        return Err(conflict(
            "Server state must be isolated from media and system files",
        ));
    }
    for mut previous in all()? {
        if previous.stage == "ready" && !owns_pre_activation(&previous, &d) {
            previous.stage = "superseded".into();
            previous.error = Some("A newer deployment superseded this preflight".into());
            write(&previous)?;
            remove_copies(&previous, None);
        }
    }
    let mut u = Update {
        id: key,
        created_at: thelxinoe_core::now(),
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
        download: None,
        failure: None,
        latest: false,
    };
    private(&dir(&u.id))?;
    write(&u)?;
    let result = public(&u);
    tokio::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = prepare(&mut u).await {
            u.stage = "blocked".into();
            u.error = Some(e.1.to_string());
            u.failure.get_or_insert_with(|| "environment".into());
            u.download = None;
            let _ = write(&u);
            let _ = updates::start(u.old.server["Id"].as_str().unwrap_or("")).await;
            // A retry is a new attempt; these copies only hold disk space and credentials.
            remove_copies(&u, None);
        }
    });
    Ok(Json(result))
}
async fn pull(u: &mut Update, component: &str, image: &thelxinoe_releases::Image) -> Result<()> {
    u.download = Some(Download {
        component: component.into(),
        received: 0,
        total: None,
    });
    write(u)?;
    fetch(component, image, |download| {
        u.download = Some(download);
        write(u)
    })
    .await
}
/// Make a signed release image available locally, reporting download progress
/// about once a second. Cached immutable images also support offline recovery
/// and release fixtures.
async fn fetch(
    component: &str,
    image: &thelxinoe_releases::Image,
    mut progress: impl FnMut(Download) -> Result<()>,
) -> Result<()> {
    let raw = match engine(&format!("/images/{}/json", image.reference)).await {
        Ok(v) => v,
        Err((StatusCode::NOT_FOUND, _)) => {
            let mut layers = std::collections::BTreeMap::<String, (u64, Option<u64>, bool)>::new();
            let mut published = std::time::Instant::now();
            let mut latest = None;
            crate::docker::pull_image(&image.reference, |event| {
                let status = event["status"].as_str().unwrap_or("");
                if let Some(id) = event["id"].as_str() {
                    let total = event["progressDetail"]["total"].as_u64().filter(|v| *v > 0);
                    if layers.contains_key(id)
                        || total.is_some()
                        || matches!(status, "Pulling fs layer" | "Waiting" | "Already exists")
                    {
                        let layer = layers.entry(id.into()).or_insert((0, None, false));
                        if matches!(
                            status,
                            "Already exists" | "Download complete" | "Pull complete"
                        ) {
                            layer.2 = true;
                            layer.0 = layer.1.unwrap_or(layer.0);
                        } else if status == "Downloading" {
                            layer.1 = total.or(layer.1);
                            layer.0 = event["progressDetail"]["current"]
                                .as_u64()
                                .unwrap_or(layer.0)
                                .min(layer.1.unwrap_or(u64::MAX));
                        }
                    }
                }
                let total = (!layers.is_empty()
                    && layers
                        .values()
                        .all(|(_, total, done)| total.is_some() || *done))
                .then(|| {
                    layers
                        .values()
                        .map(|(current, total, _)| total.unwrap_or(*current))
                        .sum()
                });
                let download = Download {
                    component: component.into(),
                    received: layers.values().map(|(current, _, _)| *current).sum(),
                    total,
                };
                if published.elapsed() >= Duration::from_secs(1) {
                    progress(download)?;
                    published = std::time::Instant::now();
                } else {
                    latest = Some(download);
                }
                Ok(())
            })
            .await?;
            if let Some(download) = latest {
                progress(download)?;
            }
            engine(&format!("/images/{}/json", image.reference)).await?
        }
        Err(e) => return Err(e),
    };
    signed_identity(&raw, image)
}
/// Docker's classic and containerd stores report different local image IDs.
/// The signed registry digest pins the manifest and its entire content chain.
fn signed_identity(raw: &Value, image: &thelxinoe_releases::Image) -> Result<()> {
    if !raw["RepoDigests"].as_array().is_some_and(|digests| {
        digests.iter().any(|digest| {
            digest
                .as_str()
                .and_then(|value| value.rsplit_once('@'))
                .map(|(_, hash)| hash)
                == image.reference.rsplit_once('@').map(|(_, hash)| hash)
        })
    }) || raw["Architecture"] != "amd64"
        || raw["Os"] != "linux"
    {
        return Err(conflict(
            "Release image does not match the signed Linux x86-64 identity",
        ));
    }
    Ok(())
}
/// A release's images, downloading in the background so a later preflight finds
/// them cached; the server stays available meanwhile.
struct Downloading {
    references: [String; 2],
    running: bool,
    download: Option<Download>,
    error: Option<String>,
}
static DOWNLOADS: std::sync::Mutex<Option<Downloading>> = std::sync::Mutex::new(None);
#[derive(Deserialize)]
pub(super) struct Fetch {
    envelope: Envelope,
}
pub(super) async fn download(Json(input): Json<Fetch>) -> Result<Json<Value>> {
    let manifest = thelxinoe_releases::verify(&input.envelope, &trust()?)
        .map_err(|_| bad("Release signature or manifest is invalid"))?;
    let images = [manifest.server.clone(), manifest.controller.clone()];
    let references = images.clone().map(|image| image.reference);
    let state =
        |downloading: &Downloading| json!({"state":"downloading","download":downloading.download});
    {
        let mut downloads = DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(current) = downloads.as_ref().filter(|d| d.references == references) {
            if current.running {
                return Ok(Json(state(current)));
            }
            // Report a failure once; the next request starts again.
            if let Some(error) = current.error.clone() {
                *downloads = None;
                return Ok(Json(json!({"state":"failed","error":error})));
            }
        }
    }
    let mut missing = false;
    for image in &images {
        match engine(&format!("/images/{}/json", image.reference)).await {
            Ok(raw) => signed_identity(&raw, image)?,
            Err((StatusCode::NOT_FOUND, _)) => missing = true,
            Err(e) => return Err(e),
        }
    }
    if !missing {
        return Ok(Json(json!({"state":"ready"})));
    }
    let mut downloads = DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(current) = downloads.as_ref().filter(|d| d.running) {
        return if current.references == references {
            Ok(Json(state(current)))
        } else {
            Err(conflict("Another release is downloading"))
        };
    }
    *downloads = Some(Downloading {
        references: references.clone(),
        running: true,
        download: None,
        error: None,
    });
    drop(downloads);
    tokio::spawn(async move {
        let update = |change: &dyn Fn(&mut Downloading)| {
            let mut downloads = DOWNLOADS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(current) = downloads.as_mut().filter(|d| d.references == references) {
                change(current);
            }
        };
        let mut result = Ok(());
        for (component, image) in [("server", &images[0]), ("controller", &images[1])] {
            result = fetch(component, image, |download| {
                update(&|current| current.download = Some(download.clone()));
                Ok(())
            })
            .await;
            if result.is_err() {
                break;
            }
        }
        let error = result.err().map(|(_, error)| error.to_string());
        update(&|current| {
            current.running = false;
            current.download = None;
            current.error = error.clone();
        });
    });
    Ok(Json(json!({"state":"downloading","download":null})))
}
async fn server_state_isolated(source: &str, media: &str) -> Result<bool> {
    if policy::appdata_isolated(source, media) {
        return Ok(true);
    }
    if !policy::media_disjoint(source, media) {
        return Ok(false);
    }
    // Saved Compose turns first-party volumes into binds to the same host
    // directory. Accept only default local-volume data, never Docker's other
    // state or a volume driver that redirects the mount to host system files.
    let volumes = engine("/volumes").await?;
    Ok(volumes["Volumes"].as_array().is_some_and(|volumes| {
        volumes.iter().any(|volume| {
            volume["Driver"] == "local"
                && (volume["Options"].is_null()
                    || volume["Options"]
                        .as_object()
                        .is_some_and(|options| options.is_empty()))
                && volume["Mountpoint"].as_str().is_some_and(|root| {
                    !root.is_empty()
                        && std::path::Path::new(source).starts_with(std::path::Path::new(root))
                })
        })
    }))
}
/// Database checks and migrations can take long on slow disks; the probe cannot.
const STATE_CHECK_LIMIT: u64 = 1800;
async fn worker(u: &Update, label: &str, spec: Value, seconds: u64) -> Result<()> {
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
        updates::wait(key, seconds).await
    }
    .await;
    let cleaned = updates::remove(key).await;
    result?;
    cleaned
}
fn worker_spec(u: &Update, image: &str, command: &str, mounts: Value) -> Result<Value> {
    let identity = Identity::from_user(&u.old.server["Config"])?;
    let user = if command == "controller-probe" {
        u.old.controller["Config"]["User"].clone()
    } else {
        json!(identity.user())
    };
    let mut spec = json!({"Image":image,"User":user,"Cmd":[command],"Labels":{"app.thelxinoe.product-update":u.id,"app.thelxinoe.deployment":u.old.id},"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Tmpfs":{"/tmp":"rw,noexec,nosuid,size=64m","/var/cache/thelxinoe":format!("rw,nosuid,size=64m,uid={},gid={}",identity.uid,identity.gid)},"Mounts":mounts}});
    crate::docker::limit(&mut spec, 1 << 30, 2, 64);
    Ok(spec)
}
async fn verify_state(u: &Update, leaf: &str) -> Result<u32> {
    // SQLite may create WAL/SHM even for a read-only integrity connection.
    // Every state worker uses the server identity so later validation can write them.
    let spec = worker_spec(
        u,
        &updates::current_image().await?,
        "verify-state",
        json!([{"Type":"bind","Source":host(u,leaf),"Target":"/state"}]),
    )?;
    worker(u, "verify", spec, STATE_CHECK_LIMIT).await?;
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
    )?;
    worker(u, "validate", spec, STATE_CHECK_LIMIT).await?;
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
    // Every product copy holds server state; only snapshots leave out its disposables.
    if restore {
        updates::copy_state(&u.old, &u.id, source, &host(u, leaf), true, leaf).await
    } else {
        updates::copy_server_state(&u.old, &u.id, source, &host(u, leaf), leaf).await
    }
}
async fn prepare(u: &mut Update) -> Result<()> {
    let manifest = u.manifest.clone().ok_or_else(unavailable)?;
    pull(u, "server", &manifest.server).await?;
    pull(u, "controller", &manifest.controller).await?;
    u.download = None;
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
    if manifest
        .candidate(&u.old.version, schema, thelxinoe_core::now())
        .is_err()
    {
        u.failure = Some("incompatible".into());
        return Err(conflict(
            "Release version, validity, schema or recovery protocol is incompatible",
        ));
    }
    u.stage = "validating".into();
    write(u)?;
    let validated = async {
        copy(u, &host(u, "preflight-snapshot"), "clone", false).await?;
        server_check(
            u,
            "clone",
            &manifest.server.reference,
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
            &manifest.controller.reference,
            "controller-probe",
            json!([{"Type":"bind","Source":host(u,"probe"),"Target":"/probe"}]),
        )?;
        worker(u, "probe", spec, 180).await?;
        let probe: Value = persisted(store::read(&dir(&u.id).join("probe/controller.json")))?;
        if probe["version"] != manifest.version || probe["recovery_protocol"] != 1 {
            return Err(conflict("Successor controller failed its release contract"));
        }
        Ok(())
    }
    .await;
    if let Err(e) = validated {
        u.failure = Some(classify(&e).into());
        return Err(e);
    }
    u.recovery_tested = true;
    u.stage = "ready".into();
    write(u)?;
    // Activation takes its own rollback copy; the preflight's are no longer needed.
    remove_copies(u, None);
    Ok(())
}
/// A worker that ran and reported a broken contract is the release failing on this
/// server's data; anything else (Docker, disk, time limits) may pass on a retry.
fn classify(error: &Failure) -> &'static str {
    if updates::contract_failure(error)
        || [
            "Server migration contract does not match",
            "Successor controller failed its release contract",
            "Isolated live-state validation returned an unexpected contract",
        ]
        .iter()
        .any(|message| error.1.starts_with(message))
    {
        "validation"
    } else {
        "environment"
    }
}
/// Delete an attempt's state copies, except `keep`. They hold complete server
/// state, credentials included.
fn remove_copies(u: &Update, keep: Option<&str>) {
    for leaf in [
        "preflight-snapshot",
        "clone",
        "rollback",
        "validated",
        "probe",
    ] {
        let path = dir(&u.id).join(leaf);
        if Some(leaf) == keep || !path.exists() {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            eprintln!("Could not remove {}: {error}", path.display());
        }
    }
    // Progress reports of copy workers interrupted before they could remove them.
    let prefix = format!("thelxinoe-state-{}-", &u.id[..u.id.len().min(8)]);
    if let Ok(entries) = std::fs::read_dir(store::root().join("progress")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}
/// The image tag a Compose service names, kept on its successors.
const COMPOSE_IMAGE: &str = "app.thelxinoe.compose-image";
/// The successor of `raw` running `image`. A container's configuration also holds
/// what its image supplied (environment, command, health check, labels); those
/// give way to the new image's, while the deployment's own settings stay.
/// `previous` and `next` are the image inspections, or null when unknown.
fn recreate(raw: &Value, image: &str, previous: &Value, next: &Value) -> Result<Value> {
    let mut spec = raw["Config"].clone();
    spec["Image"] = json!(image);
    let inherited = &previous["Config"];
    if let Some(env) = spec["Env"].as_array_mut()
        && let Some(defaults) = inherited["Env"].as_array()
    {
        env.retain(|item| !defaults.contains(item));
    }
    for key in [
        "Cmd",
        "Entrypoint",
        "WorkingDir",
        "Healthcheck",
        "StopSignal",
    ] {
        if !inherited.is_null() && spec[key] == inherited[key] {
            spec[key] = Value::Null;
        }
    }
    if let Some(labels) = spec["Labels"].as_object_mut() {
        labels.retain(|key, value| {
            !key.starts_with("org.opencontainers.image.") && inherited["Labels"][key] != *value
        });
        // Compose recreates a container whose image ID differs from this label.
        if labels.contains_key("com.docker.compose.image")
            && let Some(id) = next["Id"].as_str()
        {
            labels.insert("com.docker.compose.image".into(), json!(id));
        }
        // Successors run a pinned digest; remember the tag Compose names.
        if let Some(reference) = raw["Config"]["Image"].as_str().filter(|r| !r.contains('@')) {
            labels.insert(COMPOSE_IMAGE.into(), json!(reference));
        }
    }
    spec["Hostname"] = json!("");
    spec["HostConfig"] = raw["HostConfig"].clone();
    let networks = raw["NetworkSettings"]["Networks"]
        .as_object()
        .ok_or_else(unavailable)?
        .iter()
        .map(|(name, n)| {
            (
                name.clone(),
                json!({"Aliases":policy::aliases(&n["Aliases"],raw["Id"].as_str()),"IPAMConfig":n["IPAMConfig"]}),
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
    ))?;
    retire_obsolete(d.generation);
    Ok(())
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
            u.error = Some(e.1.to_string());
            u.failure = Some(classify(&e).into());
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
        &manifest.server.reference,
        "validate-state",
        json!([{"Type":"bind","Source":u.source,"Target":"/var/lib/thelxinoe"}]),
    )?;
    spec["Labels"]["app.thelxinoe.validation"] = json!("true");
    worker(u, "live-validation", spec, STATE_CHECK_LIMIT).await?;
    // Copy the report out with trusted code, then inspect the version/schema contract.
    let report = live_report(u).await?;
    if report["version"] != manifest.version || report["schema"] != manifest.migration.target {
        return Err(conflict(
            "Isolated live-state validation returned an unexpected contract",
        ));
    }
    handoff(u).await
}
/// The live-state validation report alone, copied out by a worker running as
/// the server; the rest of the live state is not copied again.
async fn live_report(u: &Update) -> Result<Value> {
    let leaf = dir(&u.id).join("validated");
    private(&leaf)?;
    let identity = Identity::from_user(&u.old.server["Config"])?;
    persisted(
        std::os::unix::fs::chown(&leaf, Some(identity.uid), Some(identity.gid)).map_err(Into::into),
    )?;
    let spec = worker_spec(
        u,
        &updates::current_image().await?,
        "state-report",
        json!([{"Type":"bind","Source":u.source,"Target":"/source","ReadOnly":true},
            {"Type":"bind","Source":host(u,"validated"),"Target":"/destination"}]),
    )?;
    worker(u, "report", spec, 180).await?;
    persisted(store::read(&leaf.join(".release-validation.json")))
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
            .reference
            .clone()
    };
    let controller_image = if u.restoring_release {
        immutable(&u.old.controller)?
    } else {
        u.manifest
            .as_ref()
            .ok_or_else(unavailable)?
            .controller
            .reference
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
    // A missing image only loses the hand-over of image defaults.
    let inspect = |reference: String| async move {
        engine(&format!("/images/{reference}/json"))
            .await
            .unwrap_or(Value::Null)
    };
    let images = [
        inspect(immutable(&u.old.server)?).await,
        inspect(server_image.clone()).await,
        inspect(immutable(&u.old.controller)?).await,
        inspect(controller_image.clone()).await,
    ];
    let retired = if u.restoring_release {
        "replaced"
    } else {
        "retired"
    };
    rename(
        retiring.server["Id"].as_str().ok_or_else(unavailable)?,
        &format!("{server_name}-{retired}-{}", &u.id[..8]),
    )
    .await?;
    let mut spec = recreate(&u.old.server, &server_image, &images[0], &images[1])?;
    spec["Labels"]["app.thelxinoe.product-update"] = json!(u.id);
    spec["Labels"]["app.thelxinoe.deployment"] = json!(u.old.id);
    u.next_server = Some(create(&server_name, spec).await?);
    write(u)?;
    let mut spec = recreate(&u.old.controller, &controller_image, &images[2], &images[3])?;
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
        &format!("{controller_name}-{retired}-{}", &u.id[..8]),
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
fn owns_pre_activation(u: &Update, current: &Deployment) -> bool {
    current.id == u.old.id
        && ((current.generation == u.old.generation
            && current.server["Id"] == u.old.server["Id"]
            && current.controller["Id"] == u.old.controller["Id"])
            || (u.generation == Some(current.generation)
                && u.next_server.is_some()
                && u.next_controller.is_some()
                && current.server["Id"].as_str() == u.next_server.as_deref()
                && current.controller["Id"].as_str() == u.next_controller.as_deref()))
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
    let current = bootstrap().await?;
    if !owns_pre_activation(u, &current) {
        return Err(conflict(
            "This attempt no longer owns the accepted deployment",
        ));
    }
    u.stage = "recovering".into();
    write(u)?;
    clean_workers(u).await?;
    clean_successors(u, &u.old).await?;
    let server = u.old.server["Id"].as_str().ok_or_else(unavailable)?;
    // The previous containers may have been removed by hand meanwhile.
    let present = |result: Result<()>| match result {
        Err((StatusCode::NOT_FOUND, _)) => Ok(false),
        other => other.map(|_| true),
    };
    if u.snapshot_ready {
        if !present(updates::stop(server).await)? {
            // Never restore beneath a server that took over the same state.
            let named = engine(&format!("/containers/{}/json", name(&u.old.server)?)).await;
            if named.is_ok_and(|c| c["State"]["Running"] == true) {
                return Err(conflict(
                    "Another server container is running; stop it before recovering the update",
                ));
            }
        }
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
    let server_present = present(rename(server, name(&u.old.server)?).await)?;
    present(
        rename(
            u.old.controller["Id"].as_str().ok_or_else(unavailable)?,
            name(&u.old.controller)?,
        )
        .await,
    )?;
    if current.generation != u.old.generation {
        let mut restored = u.old.clone();
        restored.generation = next_generation()?;
        commit(&restored)?;
    }
    if server_present {
        updates::start(server).await?;
    } else {
        u.error = Some(
            "The previous containers were removed; run docker compose up -d to start Thelxinoe again"
                .into(),
        );
    }
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
    // Completed recovery is idempotent; never erase subsequent user changes.
    if matches!(u.stage.as_str(), "recovered" | "restored") {
        return Ok(Json(public(&u)));
    }
    if u.stage == "superseded" {
        return Err(conflict(
            "This attempt no longer owns the accepted deployment",
        ));
    }
    if !u.activation_crossed && !u.restoring_release {
        recover_old(&mut u).await?;
        return Ok(Json(public(&u)));
    }
    let d = bootstrap().await?;
    if !u.snapshot_ready || (!u.restoring_release && !snapshot_matches(&u, &d)) {
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
            u.error = Some(e.1.to_string());
            let _ = write(&u);
        }
    });
    Ok(Json(result))
}
fn snapshot_matches(u: &Update, current: &Deployment) -> bool {
    let Some(generation) = u.generation else {
        return false;
    };
    if current.generation == generation {
        return true;
    }
    // A verified Compose recreation advances container IDs and the deployment
    // generation, but retains the same installed release and recovery snapshot.
    let path = store::root().join(format!("generations/{generation}/desired-state.json"));
    let Ok(accepted) = store::read::<Deployment>(&path) else {
        return false;
    };
    current.generation > generation
        && current.id == accepted.id
        && current.version == accepted.version
        && current.server["Config"]["Labels"]["app.thelxinoe.product-update"] == u.id
        && current.controller["Config"]["Labels"]["app.thelxinoe.product-update"] == u.id
        && equivalent(&accepted.server, &current.server)
        && equivalent(&accepted.controller, &current.controller)
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
    // Interrupted attempts recover against the containers they started with first.
    if all()
        .map_err(|(_, e)| anyhow::anyhow!(e))?
        .iter()
        .any(|u| !settled(&u.stage))
    {
        return Ok(false);
    }
    // Compose can recreate a pinned generation with new container IDs. Only accept its
    // original names, exact images and equivalent security/mount configuration.
    let controller = engine(&format!("/containers/{own}/json"))
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    if !equivalent(&d.controller, &controller) {
        return Ok(false);
    }
    let server_name = name(&d.server).map_err(|(_, e)| anyhow::anyhow!(e))?;
    let mut server = engine(&format!("/containers/{server_name}/json"))
        .await
        .map_err(|(_, e)| anyhow::anyhow!(e))?;
    if !equivalent(&d.server, &server) {
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
/// Compose recreates the containers after `docker compose pull`, a rebuild or an
/// edited `.env`. Adopt them as the next generation of this deployment, so
/// installed and transferred services stay registered, when they keep its
/// storage, Docker access, network and media; otherwise stop with the reason.
/// Runs before this controller takes the writer role.
pub(crate) async fn adopt_local(runtime: &std::path::Path) -> anyhow::Result<()> {
    let own = std::env::var("HOSTNAME")?;
    println!("{}", accept_containers(&own, Some(runtime), report).await?);
    Ok(())
}
fn report(changes: &[String]) -> anyhow::Result<()> {
    println!("Adopting the recreated Thelxinoe containers:");
    for line in changes {
        println!("  {line}");
    }
    Ok(())
}
/// Compose can recreate the server alone, for example after an edited `.env`,
/// while this controller keeps running. Adopt it between Docker operations.
pub(super) async fn adopt_recreated(runtime: Runtime) {
    let mut reported = None;
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let Ok(own) = std::env::var("HOSTNAME") else {
            return;
        };
        let Ok(d) = store::read::<Deployment>(&store::root().join("desired-state.json")) else {
            continue;
        };
        // Only the accepted writer adopts; a retiring controller leaves it to its successor.
        if !d.controller["Id"]
            .as_str()
            .is_some_and(|id| id.starts_with(&own))
        {
            continue;
        }
        // Checking the server's identity needs no lock, so Docker operations are
        // only held up when Compose actually replaced it.
        let replaced = match name(&d.server) {
            Ok(server) => engine(&format!("/containers/{server}/json"))
                .await
                .map_or(true, |current| current["Id"] != d.server["Id"]),
            Err(_) => false,
        };
        if !replaced {
            continue;
        }
        let Ok(_guard) = runtime.0.try_lock_owned() else {
            continue;
        };
        let outcome = accept_containers(&own, None, report)
            .await
            .unwrap_or_else(|error| format!("Recreated containers were not adopted: {error}"));
        // Repeat a refusal only when its reason changes.
        if !outcome.starts_with("Nothing to accept") && reported.as_ref() != Some(&outcome) {
            println!("{outcome}");
        }
        reported = Some(outcome);
    }
}
/// Without `runtime` the caller already holds the writer lease.
async fn accept_containers(
    own: &str,
    runtime: Option<&std::path::Path>,
    confirm: impl FnOnce(&[String]) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    let fail = |(_, message): crate::docker::Failure| anyhow::anyhow!(message);
    let file = store::root().join("desired-state.json");
    if !file.exists() {
        return Ok(
            "Nothing to accept: a new deployment accepts its first containers on its own.".into(),
        );
    }
    let mut d: Deployment = store::read(&file)?;
    let me = engine(&format!("/containers/{own}/json"))
        .await
        .map_err(fail)?;
    let project = me["Config"]["Labels"]["com.docker.compose.project"]
        .as_str()
        .ok_or_else(|| {
            anyhow::anyhow!("Start the controller with Docker Compose from the Thelxinoe directory")
        })?
        .to_owned();
    let rows = engine("/containers/json?all=true").await.map_err(fail)?;
    let mut found = Vec::new();
    for component in ["controller", "server"] {
        // The project's service containers, never one-off runs such as this one.
        let ids: Vec<&str> = rows
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| {
                c["Labels"]["com.docker.compose.project"] == project.as_str()
                    && c["Labels"]["app.thelxinoe.component"] == component
                    && c["Labels"]["com.docker.compose.oneoff"] != "True"
            })
            .filter_map(|c| c["Id"].as_str())
            .collect();
        anyhow::ensure!(
            ids.len() == 1,
            "Expected one {component} container in the {project} project, found {}; run docker compose up -d first",
            ids.len()
        );
        found.push(
            engine(&format!("/containers/{}/json", ids[0]))
                .await
                .map_err(fail)?,
        );
    }
    let (controller, mut server) = (found.remove(0), found.remove(0));
    let source = |c: &Value, destination: &str| {
        mount(c, destination)
            .ok()
            .and_then(|m| m["Source"].as_str())
            .map(str::to_owned)
    };
    let storage = source(&controller, "/var/lib/thelxinoe/deployment");
    anyhow::ensure!(
        storage.as_deref() == Some(d.appdata_source.as_str())
            && source(&me, "/var/lib/thelxinoe/deployment") == storage,
        "The controller now uses different deployment storage; a different state folder is a different deployment"
    );
    anyhow::ensure!(
        source(&controller, "/var/run/docker.sock").is_some(),
        "The controller must mount the Docker socket"
    );
    let shared = source(&controller, "/run/thelxinoe");
    anyhow::ensure!(
        shared.is_some() && source(&server, "/run/thelxinoe") == shared,
        "The server must share the controller's runtime volume"
    );
    Identity::from_user(&server["Config"]).map_err(fail)?;
    // Both images come from one release; `docker compose pull` fetches them together.
    let version = |c: &Value| {
        c["Config"]["Labels"]["org.opencontainers.image.version"]
            .as_str()
            .map(str::to_owned)
    };
    let (server_version, controller_version) = (version(&server), version(&controller));
    anyhow::ensure!(
        server_version.is_none()
            || controller_version.is_none()
            || server_version == controller_version,
        "The server ({}) and controller ({}) images are different Thelxinoe versions; run docker compose pull, then docker compose up -d",
        server_version.unwrap_or_default(),
        controller_version.unwrap_or_default()
    );
    let networks: Vec<String> = server["NetworkSettings"]["Networks"]
        .as_object()
        .map(|n| n.keys().cloned().collect())
        .unwrap_or_default();
    anyhow::ensure!(
        networks.len() == 1,
        "The server must be on exactly one Docker network"
    );
    let media = policy::server_media_source(&server)
        .map_err(|e| anyhow::anyhow!(e))?
        .to_owned();
    anyhow::ensure!(
        !server["Mounts"].as_array().into_iter().flatten().any(|m| {
            m["Destination"]
                .as_str()
                .is_some_and(|p| p == "/media" || p.starts_with("/media/"))
                && !m["Source"]
                    .as_str()
                    .is_some_and(|s| policy::media_disjoint(&d.appdata_source, s))
        }),
        "Deployment state must be outside every media mount"
    );
    // Installed services keep the network and media folder they were created with.
    let installed: Vec<Managed> = services()
        .map_err(fail)?
        .into_iter()
        .filter(|s| s.phase != "transferring")
        .collect();
    anyhow::ensure!(
        networks[0] == d.network || installed.is_empty(),
        "Thelxinoe-installed services use the network {}; keep the server on it",
        d.network
    );
    anyhow::ensure!(
        media == d.media_source
            || !installed
                .iter()
                .any(|s| templates::find(&s.kind).is_some_and(|t| t.media)),
        "Thelxinoe-installed media services use {}; keep it mounted as the server's /media",
        d.media_source
    );
    // Interrupted work recovers against the containers it started with.
    anyhow::ensure!(
        all().map_err(fail)?.iter().all(|u| settled(&u.stage)),
        "Finish or recover the product update first"
    );
    // An older release may not open a database the installed one migrated.
    if let Some(version) = server_version.as_deref()
        && older(version, &d.version)
    {
        anyhow::bail!(
            "The recreated containers run {version}, older than the installed {}, whose database {version} may not open. Run docker compose pull to get the newest release, or use Rollback in Settings → Server to return to an earlier one",
            d.version
        );
    }
    anyhow::ensure!(
        !super::backups::interrupted().map_err(fail)?,
        "Finish the interrupted backup or restore with the previous containers first"
    );
    normalize_stopped_server(&mut server).await.map_err(fail)?;
    let mut changes = Vec::new();
    for (component, before, after) in [
        ("server", &d.server, &server),
        ("controller", &d.controller, &controller),
    ] {
        let mut found = Vec::new();
        // Docker reports the default OOM setting as false or unset.
        let (mut before, mut after) = (before.clone(), after.clone());
        normalize_stopped_server_defaults(&mut before);
        normalize_stopped_server_defaults(&mut after);
        let (before, after) = (policy::fingerprint(&before), policy::fingerprint(&after));
        policy::changes(&before, &after, "", &mut found);
        // Recreation always gives a new hostname and new Compose bookkeeping labels.
        let noise = |path: &str| {
            path == "Config.Hostname" || path.starts_with("Config.Labels.com.docker.compose.")
        };
        changes.extend(
            found
                .iter()
                .filter(|c| !c["path"].as_str().is_some_and(noise))
                .map(|c| {
                    let values = match (c["before"].as_str(), c["after"].as_str()) {
                        (None, None) => String::new(),
                        (old, new) => {
                            format!(": {} -> {}", old.unwrap_or("unset"), new.unwrap_or("unset"))
                        }
                    };
                    format!(
                        "{component} {} {}{values}",
                        c["path"].as_str().unwrap_or_default(),
                        c["change"].as_str().unwrap_or_default()
                    )
                }),
        );
    }
    if changes.is_empty()
        && d.server["Id"] == server["Id"]
        && d.controller["Id"] == controller["Id"]
    {
        return Ok(
            "Nothing to accept: these containers are already the accepted deployment.".into(),
        );
    }
    if changes.is_empty() {
        changes.push("Recreated containers with the accepted configuration".into());
    }
    confirm(&changes)?;
    // The accepted controller keeps the lease while it runs; one starting with new
    // containers holds it only briefly while it checks.
    let mut _lease = None;
    if let Some(runtime) = runtime {
        let mut attempts = 0;
        _lease = Some(loop {
            match crate::lease::Lease::acquire(runtime) {
                Ok(lease) => break lease,
                Err(_) if attempts < 30 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(_) => anyhow::bail!("Another controller holds this deployment"),
            }
        });
    }
    if let Some(version) = server_version.or(controller_version) {
        d.version = version;
    }
    d.network = networks[0].clone();
    d.media_source = media;
    d.server = server;
    d.controller = controller;
    d.generation = next_generation().map_err(fail)?;
    commit(&d).map_err(fail)?;
    Ok(format!("Adopted as generation {}.", d.generation))
}
/// Whether release `version` precedes `installed`; development builds compare as equal.
fn older(version: &str, installed: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> {
        let parts = v
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<Vec<u64>>>()?;
        (parts.len() == 3).then_some(parts)
    };
    matches!((parse(version), parse(installed)), (Some(a), Some(b)) if a < b)
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
/// Compose recreation of the same accepted configuration, after a restart.
fn equivalent(expected: &Value, actual: &Value) -> bool {
    if !same_image_identity(expected, actual) || expected["Name"] != actual["Name"] {
        return false;
    }
    let env = |c: &Value| -> Option<std::collections::BTreeMap<String, String>> {
        let mut result = std::collections::BTreeMap::new();
        for item in c["Config"]["Env"].as_array()? {
            let (key, value) = item.as_str()?.split_once('=')?;
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
/// Journal stages no restart has to continue.
fn settled(stage: &str) -> bool {
    matches!(
        stage,
        "ready"
            | "blocked"
            | "recovered"
            | "committed"
            | "restored"
            | "runtime-failure"
            | "recovery-required"
            | "superseded"
    )
}
pub(crate) async fn startup() -> anyhow::Result<()> {
    if let Ok(d) = store::read::<Deployment>(&store::root().join("desired-state.json")) {
        retire_obsolete(d.generation);
    }
    for mut u in all().map_err(|(_, e)| anyhow::anyhow!(e))? {
        if settled(&u.stage) {
            continue;
        }
        if let Err((_, error)) = resume(&mut u).await {
            needs_recovery(&mut u, &error);
        }
    }
    retag_installed().await;
    clean_orphans().await;
    prune_journals();
    Ok(())
}
/// An attempt the controller cannot finish waits for an administrator. The
/// controller keeps running, so adoption, diagnostics and recovery stay possible.
fn needs_recovery(u: &mut Update, error: &str) {
    eprintln!("Server update {} needs recovery: {error}", u.id);
    u.stage = "recovery-required".into();
    u.error = Some(error.into());
    let _ = write(u);
}
async fn resume(u: &mut Update) -> Result<()> {
    let d = bootstrap().await?;
    if u.stage == "handoff"
        && d.generation == u.generation.unwrap_or(0)
        && d.controller["Id"].as_str() == u.next_controller.as_deref()
    {
        return take_over(u, &d).await;
    }
    if u.activation_crossed {
        u.stage = "runtime-failure".into();
        u.error = Some(
            "Server activation was interrupted; inspect health before explicit recovery".into(),
        );
        write(u)?;
        remove_retired(u).await;
        return Ok(());
    }
    if u.restoring_release {
        return restore_release(u).await;
    }
    if !owns_pre_activation(u, &d) && d.generation > u.old.generation {
        // Historical attempts must not stop the accepted controller starting.
        // No Docker or appdata mutation is permitted for this old generation.
        u.stage = "superseded".into();
        u.error = Some("A newer deployment superseded this attempt".into());
        return write(u);
    }
    interrupted(u).await
}
/// The accepted successor finishes its own activation.
async fn take_over(u: &mut Update, d: &Deployment) -> Result<()> {
    persisted(store::write_json(
        &dir(&u.id).join("writer.json"),
        &json!({"generation":d.generation,"version":thelxinoe_core::VERSION}),
    ))?;
    u.activation_crossed = true;
    u.stage = "activating".into();
    write(u)?;
    let activated = async {
        let key = u.next_server.as_deref().ok_or_else(unavailable)?;
        updates::start(key).await?;
        let mut steady = 0;
        for _ in 0..180 {
            let raw = engine(&format!("/containers/{key}/json")).await?;
            if raw["State"]["Health"]["Status"] == "healthy" {
                return Ok(());
            }
            if raw["State"]["Running"] == false {
                return Err(conflict("Accepted server exited"));
            }
            // An override can disable the health check; then staying up counts.
            if raw["State"]["Health"].is_null() {
                steady += 1;
                if steady >= 30 {
                    return Ok(());
                }
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
        u.error = Some("Accepted server could not start; explicit recovery is required".into());
    }
    write(u)?;
    if activated.is_ok() {
        u.latest = retag_latest(u).await;
        write(u)?;
        clean_copies(u);
    }
    remove_retired(u).await;
    Ok(())
}
/// The accepted descriptor is sufficient to recreate old images offline; retired
/// Compose containers must not compete with the accepted service names.
async fn remove_retired(u: &Update) {
    let retired = u.retiring.as_ref().unwrap_or(&u.old);
    let _ = updates::remove(retired.server["Id"].as_str().unwrap_or("")).await;
    let _ = updates::remove(retired.controller["Id"].as_str().unwrap_or("")).await;
}
/// Recover an attempt a restart interrupted before the point of no return. The
/// release may still be fine, so it can be tried again.
async fn interrupted(u: &mut Update) -> Result<()> {
    let preparing = matches!(
        u.stage.as_str(),
        "preparing" | "snapshotting" | "validating"
    );
    recover_old(u).await?;
    u.failure = Some("interrupted".into());
    if preparing {
        u.stage = "blocked".into();
        u.error.get_or_insert_with(|| {
            "Preparation was interrupted by a restart; it will start again".into()
        });
        u.download = None;
        remove_copies(u, None);
    } else {
        u.error.get_or_insert_with(|| {
            "The update was interrupted by a restart; the previous version is running".into()
        });
    }
    write(u)
}
/// Before this controller accepts containers that Compose recreated, finish the
/// attempts a restart interrupted. Their previous containers may be gone.
pub(crate) async fn recover_interrupted() {
    let own = std::env::var("HOSTNAME").unwrap_or_default();
    for mut u in all().unwrap_or_default() {
        if settled(&u.stage) {
            continue;
        }
        let predecessor = !own.is_empty()
            && [Some(&u.old), u.retiring.as_ref()]
                .into_iter()
                .flatten()
                .any(|d| {
                    d.controller["Id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(&own))
                });
        let result =
            if u.stage == "handoff" && !dir(&u.id).join("writer.json").exists() && predecessor {
                // This controller restarted while its successor never took over.
                hand_back(&mut u).await
            } else if u.stage == "handoff" || u.activation_crossed || u.restoring_release {
                Err(conflict(
                    "The update was interrupted while switching versions; recover it explicitly",
                ))
            } else {
                interrupted(&mut u).await
            };
        if let Err((_, error)) = result {
            needs_recovery(&mut u, &error);
        }
    }
}
/// Pre-activation attempts for an older generation can never activate. Retire
/// them, and the copies of those that never touched live state, so clients and
/// later updates only see current attempts.
fn retire_obsolete(generation: u64) {
    for mut u in all().unwrap_or_default() {
        let stale = u.old.generation < generation
            && (matches!(u.stage.as_str(), "ready" | "blocked")
                || (u.stage == "recovery-required"
                    && !u.activation_crossed
                    && !u.restoring_release
                    && u.generation.is_none()));
        if !stale {
            continue;
        }
        let copies = u.stage != "recovery-required";
        u.stage = "superseded".into();
        u.error = Some("A newer deployment superseded this attempt".into());
        u.download = None;
        if write(&u).is_ok() && copies {
            remove_copies(&u, None);
        }
    }
}
/// Worker containers a restart left behind, for attempts that are settled.
async fn clean_orphans() {
    let Ok(rows) = engine("/containers/json?all=true").await else {
        return;
    };
    let attempts = all()
        .unwrap_or_default()
        .into_iter()
        .filter(|u| settled(&u.stage))
        .map(|u| (u.id, u.old.id))
        .collect::<Vec<_>>();
    for c in rows.as_array().into_iter().flatten() {
        let labels = &c["Labels"];
        let Some(attempt) = labels["app.thelxinoe.product-update"]
            .as_str()
            .or(labels["app.thelxinoe.update"].as_str())
        else {
            continue;
        };
        // Successors carry their component label and stay.
        if !labels["app.thelxinoe.component"].is_null() {
            continue;
        }
        if attempts.iter().any(|(id, deployment)| {
            id == attempt && labels["app.thelxinoe.deployment"].as_str() == Some(deployment)
        }) && let Some(container) = c["Id"].as_str()
        {
            let _ = updates::remove(container).await;
        }
    }
}
/// Superseded attempts hold no copies; keep a month of them for diagnostics.
fn prune_journals() {
    let cutoff = thelxinoe_core::now() - 30 * 86400;
    for u in all().unwrap_or_default() {
        if u.stage == "superseded" && u.created_at < cutoff {
            let _ = std::fs::remove_dir_all(dir(&u.id));
        }
    }
}
/// `docker compose up` compares containers with the images their Compose tags name
/// locally. A deployment following `latest` gets that tag moved to the installed
/// (or restored) release, so Compose neither recreates the new containers nor
/// brings back another release; `docker compose pull` still reaches anything newer.
async fn retag_latest(u: &Update) -> bool {
    let images = if u.restoring_release {
        match (immutable(&u.old.server), immutable(&u.old.controller)) {
            (Ok(server), Ok(controller)) => [server, controller],
            _ => return false,
        }
    } else {
        let Some(manifest) = &u.manifest else {
            return false;
        };
        [
            manifest.server.reference.clone(),
            manifest.controller.reference.clone(),
        ]
    };
    let mut tagged = true;
    for (raw, image) in [(&u.old.server, &images[0]), (&u.old.controller, &images[1])] {
        let Some(repository) = raw["Config"]["Labels"][COMPOSE_IMAGE]
            .as_str()
            .or(raw["Config"]["Image"].as_str())
            .and_then(|reference| reference.strip_suffix(":latest"))
        else {
            continue;
        };
        let path = format!("/images/{image}/tag?repo={repository}&tag=latest");
        if let Err((_, error)) = request(Method::POST, &path, None).await {
            eprintln!("Could not point {repository}:latest at the installed release: {error}");
            tagged = false;
        }
    }
    tagged
}
/// Retry a `latest` tag a previous start could not move, for the release that is
/// still installed.
async fn retag_installed() {
    let Ok(d) = store::read::<Deployment>(&store::root().join("desired-state.json")) else {
        return;
    };
    let Some(mut u) = all()
        .unwrap_or_default()
        .into_iter()
        .find(|u| matches!(u.stage.as_str(), "committed" | "restored"))
    else {
        return;
    };
    let installed = if u.restoring_release {
        Some(u.old.version.as_str())
    } else {
        u.manifest.as_ref().map(|m| m.version.as_str())
    };
    if u.latest || installed != Some(d.version.as_str()) {
        return;
    }
    u.latest = retag_latest(&u).await;
    let _ = write(&u);
}
/// Once a release commits, only its rollback copy can still be used. Earlier
/// attempts' copies hold complete server state, credentials included.
fn clean_copies(committed: &Update) {
    for u in all().unwrap_or_default() {
        let finished = matches!(
            u.stage.as_str(),
            "committed" | "blocked" | "recovered" | "restored" | "runtime-failure" | "superseded"
        );
        if finished {
            remove_copies(&u, (u.id == committed.id).then_some("rollback"));
        }
    }
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
    // A successor that is still starting holds the lease for a while.
    let mut attempts = 0;
    let _lease = loop {
        match crate::lease::Lease::acquire(runtime) {
            Ok(lease) => break lease,
            Err(_) if attempts < 300 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error),
        }
    };
    for mut u in all().map_err(|(_, e)| anyhow::anyhow!(e))? {
        if u.stage == "handoff"
            && !dir(&u.id).join("writer.json").exists()
            && let Err((_, error)) = hand_back(&mut u).await
        {
            needs_recovery(&mut u, &error);
        }
    }
    Ok(true)
}
/// Take the deployment back from a successor that never became the writer.
async fn hand_back(u: &mut Update) -> Result<()> {
    if !u.restoring_release {
        return recover_old(u).await;
    }
    // Keep the existing controller available for explicit continuation;
    // never start its newer server against the restored older database.
    let mut active = u.retiring.clone().ok_or_else(unavailable)?;
    clean_successors(u, &active).await?;
    rename(
        active.server["Id"].as_str().ok_or_else(unavailable)?,
        name(&u.old.server)?,
    )
    .await?;
    rename(
        active.controller["Id"].as_str().ok_or_else(unavailable)?,
        name(&u.old.controller)?,
    )
    .await?;
    active.generation = next_generation()?;
    commit(&active)?;
    u.stage = "recovery-required".into();
    u.error = Some(
        "Recovery controller did not take over; resume recovery through the private controller API"
            .into(),
    );
    write(u)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_ownership_allows_its_handoff_but_rejects_later_deployments() {
        let mut update: Update = serde_json::from_value(json!({
            "id":"attempt", "stage":"ready", "error":null,"manifest":null,"envelope":null,
            "old":{"id":"deployment","generation":2,"version":"0.1.0",
                "server":{"Id":"old-server"},"controller":{"Id":"old-controller"},
                "network":"media","media_source":"/media","appdata_source":"/deployment"},
            "source":"/state","snapshot_ready":false,"recovery_tested":true,
            "activation_crossed":false,"next_server":null,"next_controller":null,"generation":null
        }))
        .unwrap();
        let mut current = update.old.clone();
        assert!(owns_pre_activation(&update, &current));
        current.generation = 3;
        assert!(!owns_pre_activation(&update, &current));
        update.generation = Some(3);
        update.next_server = Some("successor-server".into());
        update.next_controller = Some("successor-controller".into());
        current.server["Id"] = json!("successor-server");
        current.controller["Id"] = json!("successor-controller");
        assert!(owns_pre_activation(&update, &current));
        current.generation = 4;
        assert!(!owns_pre_activation(&update, &current));
        current.generation = 3;
        current.id = "other-deployment".into();
        assert!(!owns_pre_activation(&update, &current));
    }

    #[test]
    fn successors_take_new_image_defaults_and_keep_deployment_settings() {
        let raw = json!({"Config":{"Image":"ghcr.io/valtrius/thelxinoe-server:latest","User":"1000:1000",
            "Env":["PATH=/usr/bin","THELXINOE_WEB=/opt/web","THELXINOE_PUBLIC_URL=https://media.example"],
            "Cmd":["thelxinoe-server"],"Entrypoint":null,"WorkingDir":"/","Healthcheck":{"Test":["CMD","old-check"]},
            "Labels":{"com.docker.compose.project":"thelxinoe","com.docker.compose.image":"sha256:old",
                "org.opencontainers.image.version":"0.0.1","app.thelxinoe.component":"server","vendor":"thelxinoe"}},
            "HostConfig":{"ReadonlyRootfs":true},"NetworkSettings":{"Networks":{"media":{"Aliases":["server"],"IPAMConfig":null}}}});
        let previous = json!({"Id":"sha256:old","Config":{"Env":["PATH=/usr/bin","THELXINOE_WEB=/opt/web"],
            "Cmd":["thelxinoe-server"],"Entrypoint":null,"WorkingDir":"/","Healthcheck":{"Test":["CMD","old-check"]},
            "Labels":{"org.opencontainers.image.version":"0.0.1","vendor":"thelxinoe"}}});
        let next =
            json!({"Id":"sha256:new","Config":{"Env":["PATH=/usr/bin","THELXINOE_WEB=/srv/web"]}});
        let spec = recreate(
            &raw,
            "ghcr.io/valtrius/thelxinoe-server@sha256:new",
            &previous,
            &next,
        )
        .unwrap();
        assert_eq!(
            spec["Image"],
            "ghcr.io/valtrius/thelxinoe-server@sha256:new"
        );
        // Only the deployment's own environment remains; the new image supplies the rest.
        assert_eq!(
            spec["Env"],
            json!(["THELXINOE_PUBLIC_URL=https://media.example"])
        );
        for key in ["Cmd", "WorkingDir", "Healthcheck"] {
            assert!(spec[key].is_null(), "{key} still pins the old image");
        }
        assert_eq!(spec["User"], "1000:1000");
        assert_eq!(
            spec["Labels"],
            json!({"com.docker.compose.project":"thelxinoe",
            "com.docker.compose.image":"sha256:new","app.thelxinoe.component":"server",
            "app.thelxinoe.compose-image":"ghcr.io/valtrius/thelxinoe-server:latest"})
        );
        // A successor's successor still knows the Compose tag.
        let mut successor = raw.clone();
        successor["Config"]["Image"] = spec["Image"].clone();
        successor["Config"]["Labels"] = spec["Labels"].clone();
        let later = recreate(&successor, "image@sha256:newer", &next, &Value::Null).unwrap();
        assert_eq!(
            later["Labels"][COMPOSE_IMAGE],
            "ghcr.io/valtrius/thelxinoe-server:latest"
        );
        assert_eq!(spec["HostConfig"]["ReadonlyRootfs"], true);
        // Without image evidence the container's configuration is kept as it was.
        let kept = recreate(&raw, "image", &Value::Null, &Value::Null).unwrap();
        assert_eq!(kept["Env"], raw["Config"]["Env"]);
        assert_eq!(kept["Healthcheck"], raw["Config"]["Healthcheck"]);
        // The predecessor's own short ID alias (Docker before 26) stays behind.
        let mut started = raw.clone();
        started["Id"] = json!(format!("aa4612e2625c{}", "0".repeat(52)));
        started["NetworkSettings"]["Networks"]["media"]["Aliases"] =
            json!(["server", "aa4612e2625c"]);
        let spec = recreate(&started, "image", &Value::Null, &Value::Null).unwrap();
        assert_eq!(
            spec["NetworkingConfig"]["EndpointsConfig"]["media"]["Aliases"],
            json!(["server"])
        );
    }

    #[test]
    fn newer_engine_host_settings_are_not_drift() {
        let recorded = json!({"Image":"sha256:a","Config":{"Image":"image"},
            "HostConfig":{"Privileged":false,"CapAdd":null,"Binds":["/srv:/srv"]}});
        let mut current = recorded.clone();
        current["HostConfig"]["CgroupnsMode"] = json!("private");
        current["HostConfig"]["CapAdd"] = json!([]);
        assert!(deployment_matches(&recorded, &current));
        current["HostConfig"]["Binds"] = json!(["/etc:/srv"]);
        assert!(!deployment_matches(&recorded, &current));
        let mut current = recorded.clone();
        current["HostConfig"]["Privileged"] = json!(true);
        assert!(!deployment_matches(&recorded, &current));
    }

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
        assert!(equivalent(&expected, &expected));
        let mut platform_alias = expected.clone();
        platform_alias["Image"] = json!("sha256:platform-image");
        assert!(equivalent(&expected, &platform_alias));
        let mut no_manifest = expected.clone();
        no_manifest
            .as_object_mut()
            .unwrap()
            .remove("ImageManifestDescriptor");
        assert!(equivalent(&expected, &no_manifest));
        let mut different_image_without_manifest = no_manifest.clone();
        different_image_without_manifest["Image"] = json!("sha256:different-image");
        assert!(!equivalent(&expected, &different_image_without_manifest));
        let mut renamed = expected.clone();
        renamed["Name"] = json!("/retired");
        assert!(!equivalent(&expected, &renamed));
        let mut changed = expected.clone();
        changed["ImageManifestDescriptor"]["digest"] = json!("sha256:new-manifest");
        changed["Image"] = json!("sha256:new-image");
        assert!(!equivalent(&expected, &changed));
        let mut changed = expected.clone();
        changed["HostConfig"]["Privileged"] = json!(true);
        assert!(!equivalent(&expected, &changed));
        let mut changed = expected.clone();
        changed["Mounts"][0]["Source"] = json!("/etc");
        assert!(!equivalent(&expected, &changed));
        let mut changed = expected.clone();
        changed["Config"]["Env"] = json!(["SAFE=0"]);
        assert!(!equivalent(&expected, &changed));
        let mut ordered = expected.clone();
        ordered["Config"]["Env"] = json!(["A=1", "B=2"]);
        let mut reordered = ordered.clone();
        reordered["Config"]["Env"] = json!(["B=2", "A=1"]);
        assert!(equivalent(&ordered, &reordered));
        reordered["Config"]["Env"] = json!(["A=1", "B=2", "A=3"]);
        assert!(!equivalent(&ordered, &reordered));
    }

    #[tokio::test]
    async fn local_acceptance_records_changed_containers_of_the_same_deployment() {
        let fixture = crate::test_support::Docker::new();
        fixture
            .scope(async {
                let root = store::root();
                let storage = root.to_string_lossy().into_owned();
                let runtime = tempfile::tempdir().unwrap();
                let labels = |component: &str| {
                    json!({"com.docker.compose.project":"thelxinoe","app.thelxinoe.component":component})
                };
                let bind = |source: &str, destination: &str| {
                    json!({"Type":"bind","Source":source,"Destination":destination,"RW":true})
                };
                let controller = |id: &str, image: &str| {
                    json!({"Id":id,"Name":"/thelxinoe-controller","Image":image,
                        "Config":{"Image":"thelxinoe-controller:0.1.0","User":"0:1000","Labels":labels("controller")},
                        "HostConfig":{"NetworkMode":"none"},"State":{"Running":true},
                        "Mounts":[bind(&storage,"/var/lib/thelxinoe/deployment"),
                            bind("/var/run/docker.sock","/var/run/docker.sock"),
                            bind("runtime","/run/thelxinoe")],
                        "NetworkSettings":{"Networks":{"none":{"NetworkID":"none"}}}})
                };
                let server = |id: &str, image: &str, env: &[&str]| {
                    json!({"Id":id,"Name":"/thelxinoe-server","Image":image,
                        "Config":{"Image":"thelxinoe-server:0.1.0","User":"1000:1000","Env":env,"Labels":labels("server")},
                        "HostConfig":{"RestartPolicy":{"Name":"unless-stopped"}},"State":{"Running":true},
                        "Mounts":[bind("/srv/media","/media"),bind("/srv/state","/var/lib/thelxinoe"),
                            bind("runtime","/run/thelxinoe")],
                        "NetworkSettings":{"Networks":{"media":{"NetworkID":"network"}}}})
                };
                let accepted: Deployment = serde_json::from_value(json!({"id":"deployment","generation":1,
                    "version":"test","server":server("old-server",&format!("sha256:{}", "1".repeat(64)),&[]),
                    "controller":controller("old-controller",&format!("sha256:{}", "1".repeat(64))),"network":"media",
                    "media_source":"/srv/media","appdata_source":storage})).unwrap();
                store::commit_generation(&root, 1, &serde_json::to_value(&accepted).unwrap(), &json!({}))
                    .unwrap();
                // Compose recreated both from a rebuild and an edited .env.
                fixture.insert("controller", controller("controller", &format!("sha256:{}", "2".repeat(64))));
                fixture.insert(
                    "server",
                    server("server", &format!("sha256:{}", "2".repeat(64)), &["THELXINOE_PUBLIC_URL=https://media.example"]),
                );
                let mut oneoff = controller("oneoff", &format!("sha256:{}", "2".repeat(64)));
                oneoff["Config"]["Labels"]["com.docker.compose.oneoff"] = json!("True");
                fixture.insert("oneoff", oneoff);
                let reviewed = std::cell::RefCell::new(Vec::new());
                let declined = accept_containers("oneoff", Some(runtime.path()), |_| {
                    anyhow::bail!("Nothing was changed")
                })
                .await;
                assert!(declined.is_err());
                let current: Deployment = store::read(&root.join("desired-state.json")).unwrap();
                assert_eq!(current.generation, 1);
                let outcome = accept_containers("oneoff", Some(runtime.path()), |changes| {
                    reviewed.borrow_mut().extend_from_slice(changes);
                    Ok(())
                })
                .await
                .unwrap();
                assert!(outcome.starts_with("Adopted as generation 2"), "{outcome}");
                let reviewed = reviewed.into_inner();
                for expected in ["server Image changed", "controller Image changed",
                    "server Config.Env.THELXINOE_PUBLIC_URL added"] {
                    assert!(reviewed.iter().any(|line| line.starts_with(expected)), "{expected} in {reviewed:?}");
                }
                let current: Deployment = store::read(&root.join("desired-state.json")).unwrap();
                assert_eq!((current.id.as_str(), current.generation), ("deployment", 2));
                assert_eq!((current.server["Id"].clone(), current.controller["Id"].clone()), (json!("server"), json!("controller")));
                let again = accept_containers("oneoff", Some(runtime.path()), |_| unreachable!()).await.unwrap();
                assert!(again.starts_with("Nothing to accept"), "{again}");
                // A different deployment folder, a second network or a moved media folder that
                // installed services rely on is never accepted.
                let mut elsewhere = controller("controller", &format!("sha256:{}", "2".repeat(64)));
                elsewhere["Mounts"][0]["Source"] = json!("/srv/other");
                fixture.insert("controller", elsewhere);
                let error = accept_containers("oneoff", Some(runtime.path()), |_| Ok(())).await.unwrap_err();
                assert!(error.to_string().contains("different deployment storage"), "{error}");
                fixture.insert("controller", controller("controller", &format!("sha256:{}", "2".repeat(64))));
                let mut joined = server("server", &format!("sha256:{}", "3".repeat(64)), &[]);
                joined["NetworkSettings"]["Networks"]["proxy"] = json!({"NetworkID":"proxy"});
                fixture.insert("server", joined);
                let error = accept_containers("oneoff", Some(runtime.path()), |_| Ok(())).await.unwrap_err();
                assert!(error.to_string().contains("exactly one Docker network"), "{error}");
                let installed: Managed = serde_json::from_value(json!({"id":"88888888-8888-8888-8888-888888888888",
                    "kind":"radarr","container":"radarr","name":"radarr","image":"image","phase":"active",
                    "spec":{},"expected":null})).unwrap();
                save(&installed).unwrap();
                let mut moved = server("server", &format!("sha256:{}", "3".repeat(64)), &[]);
                moved["Mounts"][0]["Source"] = json!("/srv/other-media");
                fixture.insert("server", moved);
                let error = accept_containers("oneoff", Some(runtime.path()), |_| Ok(())).await.unwrap_err();
                assert!(error.to_string().contains("keep it mounted as the server's /media"), "{error}");
                let current: Deployment = store::read(&root.join("desired-state.json")).unwrap();
                assert_eq!(current.generation, 2);
            })
            .await;
    }

    fn journal(id: &str, stage: &str, generation: u64, server: &str) -> Update {
        serde_json::from_value(json!({
            "id":id,"created_at":thelxinoe_core::now(),"stage":stage,"error":null,"manifest":null,
            "envelope":null,"old":{"id":"deployment","generation":generation,"version":"0.0.1",
                "server":{"Id":server,"Name":"/service"},"controller":{"Id":"old-controller","Name":"/thelxinoe-controller"},
                "network":"media","media_source":"/media","appdata_source":store::root()},
            "source":store::root().join("live"),"snapshot_ready":false,"recovery_tested":false,
            "activation_crossed":false,"next_server":null,"next_controller":null,"generation":null
        }))
        .unwrap()
    }
    fn accept(generation: u64, server: &str) {
        let d = journal("unused", "ready", generation, server).old;
        store::commit_generation(
            &store::root(),
            generation,
            &serde_json::to_value(&d).unwrap(),
            &json!({}),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn newer_generations_retire_attempts_that_can_no_longer_activate() {
        let fixture = crate::test_support::Docker::new();
        fixture
            .scope(async {
                let blocked = journal(
                    "11111111-0000-0000-0000-000000000000",
                    "blocked",
                    1,
                    "original",
                );
                let current = journal(
                    "22222222-0000-0000-0000-000000000000",
                    "ready",
                    2,
                    "original",
                );
                let mut stuck = journal(
                    "33333333-0000-0000-0000-000000000000",
                    "recovery-required",
                    1,
                    "original",
                );
                stuck.snapshot_ready = true;
                for u in [&blocked, &current, &stuck] {
                    write(u).unwrap();
                }
                std::fs::create_dir_all(dir(&blocked.id).join("preflight-snapshot")).unwrap();
                std::fs::create_dir_all(dir(&stuck.id).join("rollback")).unwrap();
                let progress =
                    store::root().join("progress/thelxinoe-state-11111111-preflight-snapshot");
                std::fs::create_dir_all(&progress).unwrap();
                accept(2, "original");
                startup().await.unwrap();
                let blocked = read(&blocked.id).unwrap();
                assert_eq!(blocked.stage, "superseded");
                assert!(!dir(&blocked.id).join("preflight-snapshot").exists());
                assert!(!progress.exists());
                assert_eq!(read(&current.id).unwrap().stage, "ready");
                // A recovery that never finished keeps its rollback copy.
                assert_eq!(read(&stuck.id).unwrap().stage, "superseded");
                assert!(dir(&stuck.id).join("rollback").exists());
                assert_eq!(
                    public(&blocked)["error"],
                    "A newer deployment superseded this attempt"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn an_interrupted_preparation_can_be_tried_again() {
        let fixture = crate::test_support::Docker::new();
        fixture
            .scope(async {
                accept(1, "original");
                let u = journal(
                    "44444444-0000-0000-0000-000000000000",
                    "snapshotting",
                    1,
                    "original",
                );
                write(&u).unwrap();
                std::fs::create_dir_all(dir(&u.id).join("preflight-snapshot")).unwrap();
                startup().await.unwrap();
                let u = read(&u.id).unwrap();
                assert_eq!(u.stage, "blocked");
                assert_eq!(u.failure.as_deref(), Some("interrupted"));
                assert!(u.error.unwrap().contains("interrupted by a restart"));
                assert!(!dir(&u.id).join("preflight-snapshot").exists());
                // The previous server runs again; the missing old controller is no obstacle.
                assert!(fixture.running("original"));
            })
            .await;
    }

    #[tokio::test]
    async fn recovery_never_restores_beneath_a_recreated_server_and_never_stops_the_controller() {
        let fixture = crate::test_support::Docker::new();
        fixture
            .scope(async {
                accept(1, "gone");
                let mut u = journal(
                    "55555555-0000-0000-0000-000000000000",
                    "isolated-migration",
                    1,
                    "gone",
                );
                u.snapshot_ready = true;
                write(&u).unwrap();
                // Compose recreated the server under its name and started it.
                fixture.insert(
                    "service",
                    json!({"Id":"service","Name":"/service","State":{"Running":true},
                    "Config":{},"HostConfig":{},"NetworkSettings":{"Networks":{}}}),
                );
                startup().await.unwrap();
                let u = read(&u.id).unwrap();
                assert_eq!(u.stage, "recovery-required");
                assert!(
                    u.error
                        .unwrap()
                        .contains("Another server container is running")
                );
                assert_eq!(fixture.copies(), 0);
            })
            .await;
    }

    #[test]
    fn only_a_broken_contract_counts_against_the_release() {
        assert_eq!(
            classify(&conflict(
                "Isolated worker failed its contract: Error: schema"
            )),
            "validation"
        );
        assert_eq!(
            classify(&conflict(
                "Server migration contract does not match the signed release"
            )),
            "validation"
        );
        assert_eq!(
            classify(&conflict("Isolated worker exceeded its time limit")),
            "environment"
        );
        assert_eq!(
            classify(&conflict("Docker refused to create container x: no")),
            "environment"
        );
        assert!(older("0.0.1", "0.0.2"));
        assert!(older("0.9.9", "1.0.0"));
        assert!(!older("0.0.2", "0.0.2"));
        assert!(!older("0.0.3", "0.0.2"));
        assert!(!older("test", "0.0.2"));
    }
}
