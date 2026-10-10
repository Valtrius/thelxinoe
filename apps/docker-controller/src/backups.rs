//! Quiesced component snapshots and encrypted portable archives, independent of server SQLite.
use super::*;
use crate::recovery::{RollbackAction, RollbackPhase};
#[cfg(test)]
#[path = "backup_recovery_tests.rs"]
mod recovery_tests;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path as FsPath, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
struct Component {
    key: String,
    container: String,
    source: String,
    running: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    created_at: i64,
    deployment: Deployment,
    services: Vec<Managed>,
    components: Vec<Component>,
    compose: Value,
}
#[derive(Serialize, Deserialize)]
struct Record {
    id: String,
    stage: String,
    created_at: i64,
    error: Option<String>,
    components: Vec<Component>,
    #[serde(default)]
    recovery: Option<String>,
    #[serde(default)]
    recovery_ready: bool,
    #[serde(default)]
    rollback_phase: Option<RollbackPhase>,
    #[serde(default)]
    release_restore: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct Input {
    passphrase: String,
}
fn root() -> PathBuf {
    store::root().join("backups")
}
fn archive_root() -> PathBuf {
    std::env::var_os("THELXINOE_BACKUPS")
        .map(PathBuf::from)
        .unwrap_or_else(root)
}
fn work(key: &str) -> PathBuf {
    root().join(key)
}
fn record(r: &Record) -> Result<()> {
    validate_record(r)?;
    persisted(store::write_json(&work(&r.id).join("operation.json"), r))
}
fn validate_record(r: &Record) -> Result<()> {
    crate::recovery::validate(r.rollback_phase, &r.stage)?;
    if r.recovery_ready {
        let recovery = r
            .recovery
            .as_deref()
            .and_then(|v| v.strip_prefix("recovery-"))
            .ok_or_else(|| conflict("Recovery journal is missing its captured state"))?;
        id(recovery)?;
    }
    Ok(())
}
fn public(r: &Record) -> Value {
    json!({"id":r.id,"stage":r.stage,"created_at":r.created_at,"error":r.error,"archive":format!("{}.age",r.id)})
}
fn failure_stage(r: &Record) -> &'static str {
    // The restore attempt is persisted before quiescence, even if its snapshot never completes.
    if r.recovery.is_some() {
        "restore-failed"
    } else {
        "failed"
    }
}
fn private_dir(path: &FsPath) -> Result<()> {
    persisted(std::fs::create_dir_all(path).map_err(Into::into))?;
    persisted(
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(Into::into),
    )
}
fn host(d: &Deployment, key: &str, leaf: &str) -> String {
    format!("{}/backups/{key}/{leaf}", d.appdata_source)
}
pub(super) async fn list() -> Result<Json<Value>> {
    let mut items = vec![];
    if root().exists() {
        for entry in std::fs::read_dir(root()).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            if entry.path().join("operation.json").is_file() {
                let r: Record = persisted(store::read(&entry.path().join("operation.json")))?;
                validate_record(&r)?;
                items.push(public(&r));
            }
        }
    }
    items.sort_by_key(|v| std::cmp::Reverse(v["created_at"].as_i64().unwrap_or(0)));
    if archive_root().exists() {
        for entry in std::fs::read_dir(archive_root()).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(key) = name.strip_suffix(".age")
                && id(key).is_ok()
                && !items.iter().any(|r| r["id"] == key)
            {
                items.push(
                    json!({"id":key,"stage":"imported","created_at":0,"archive":name,"error":null}),
                );
            }
        }
    }
    Ok(Json(
        json!({"items":items,"destination":archive_root().display().to_string()}),
    ))
}
async fn inspect(d: &Deployment) -> Result<(Vec<Managed>, Vec<Component>)> {
    let server_id = d.server["Id"].as_str().ok_or_else(unavailable)?;
    let server = engine(&format!("/containers/{server_id}/json")).await?;
    if !product::deployment_matches(&d.server, &server) {
        return Err(conflict(
            "First-party deployment drift blocks backup and restore",
        ));
    }
    if server["HostConfig"]["AutoRemove"] == true {
        return Err(conflict(
            "Docker auto-remove deployments cannot be stopped for backup",
        ));
    }
    let source = mount(&server, "/var/lib/thelxinoe")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let source = policy::host_path(&source)
        .ok_or_else(|| conflict("Unsupported server state source"))?
        .to_string_lossy()
        .into_owned();
    if !adoption::state_isolated(&source, &server, d)
        || FsPath::new(&d.appdata_source).starts_with(&source)
    {
        return Err(conflict("Server state overlaps media or deployment state"));
    }
    let mut components = vec![Component {
        key: "server".into(),
        container: server_id.into(),
        source,
        running: server["State"]["Running"] == true,
    }];
    let mut managed = services()?;
    managed.sort_by(|a, b| a.id.cmp(&b.id));
    let mut media_sources = Vec::new();
    for service in &managed {
        if service.phase != "active" {
            return Err(conflict("Finish service recovery before backing up"));
        }
        let raw = updates::verified(service, d).await?;
        let source = adoption::backup_source(&raw, d)?;
        for m in raw["Mounts"].as_array().ok_or_else(unavailable)? {
            if m["Destination"] != "/config" && m["Type"] != "tmpfs" {
                media_sources.push(m["Source"].as_str().ok_or_else(unavailable)?.to_owned());
            }
        }
        components.push(Component {
            key: service.id.clone(),
            container: service.container.clone(),
            source,
            running: raw["State"]["Running"] == true,
        });
    }
    if components.iter().any(|c| {
        media_sources
            .iter()
            .any(|media| !policy::media_disjoint(&c.source, media))
    }) {
        return Err(conflict(
            "Component state overlaps another service's media or download storage",
        ));
    }
    Ok((managed, components))
}
async fn stop_all(components: &[Component]) -> Result<()> {
    for c in components {
        if c.running {
            updates::stop(&c.container).await?;
        }
        let raw = engine(&format!("/containers/{}/json", c.container)).await?;
        if raw["State"]["Running"] != false {
            return Err(conflict("Component did not stop for a consistent snapshot"));
        }
    }
    Ok(())
}
async fn restart(components: &[Component]) -> Result<()> {
    let mut failure = None;
    for c in components.iter().rev() {
        if !c.running {
            continue;
        }
        let result = async {
            // A registry-only release relinquishes this startup obligation as well.
            if c.key != "server"
                && service_path(&c.key).exists()
                && load(&c.key)?.phase == "returned"
            {
                return Ok(());
            }
            let raw = engine(&format!("/containers/{}/json", c.container)).await?;
            if !raw["Mounts"].as_array().is_some_and(|mounts| {
                mounts.iter().any(|m| {
                    m["Source"]
                        .as_str()
                        .and_then(policy::host_path)
                        .is_some_and(|source| source == FsPath::new(&c.source))
                })
            }) {
                return Err(conflict("Interrupted backup component storage changed"));
            }
            if raw["State"]["Running"] != true {
                updates::start(&c.container).await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}
async fn rollback_original(d: &Deployment, r: &mut Record, copy: bool) -> Result<()> {
    let action = crate::recovery::plan(r.rollback_phase, copy && r.recovery_ready)?;
    if matches!(action, RollbackAction::CopyRecovery) {
        r.stage = "rollback-copying".into();
        record(r)?;
        // A journal that still permits copying cannot authorize overwriting a resumed service.
        for c in &r.components {
            if engine(&format!("/containers/{}/json", c.container)).await?["State"]["Running"]
                != false
            {
                return Err(conflict(
                    "A component resumed before recovery; explicit recovery is required",
                ));
            }
        }
        stop_all(&r.components).await?;
        let recovery = r.recovery.as_deref().ok_or_else(unavailable)?;
        let key = recovery.strip_prefix("recovery-").ok_or_else(unavailable)?;
        id(key)?;
        for (i, c) in r.components.iter().enumerate() {
            updates::copy_state(
                d,
                &r.id,
                &host(d, &r.id, &format!("{recovery}/{}", c.key)),
                &c.source,
                true,
                &format!("recover{i}"),
            )
            .await?;
        }
    }
    r.rollback_phase = Some(RollbackPhase::Activating);
    r.stage = "rollback-activating".into();
    record(r)?;
    restart(&r.components).await?;
    r.stage = failure_stage(r).into();
    record(r)
}
async fn capture(d: &Deployment, r: &Record, leaf: &str) -> Result<()> {
    for (i, c) in r.components.iter().enumerate() {
        private_dir(&work(&r.id).join(leaf).join(&c.key))?;
        updates::copy_state(
            d,
            &r.id,
            &c.source,
            &host(d, &r.id, &format!("{leaf}/{}", c.key)),
            false,
            &format!("b{i}"),
        )
        .await?;
    }
    Ok(())
}
fn check_input(input: &Input) -> Result<()> {
    if !(16..=1024).contains(&input.passphrase.len()) {
        return Err(bad("Use a backup passphrase of 16 to 1024 bytes"));
    }
    Ok(())
}
pub(super) async fn create(
    State(runtime): State<Runtime>,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    check_input(&input)?;
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let d = bootstrap().await?;
    let (services, components) = inspect(&d).await?;
    let key = thelxinoe_core::id();
    private_dir(&root())?;
    private_dir(&archive_root())?;
    private_dir(&work(&key))?;
    let mut r = Record {
        id: key,
        stage: "queued".into(),
        created_at: thelxinoe_core::now(),
        error: None,
        components,
        recovery: None,
        recovery_ready: false,
        rollback_phase: None,
        release_restore: None,
    };
    record(&r)?;
    let response = public(&r);
    tokio::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let result = async {
            r.stage = "quiescing".into();
            record(&r)?;
            stop_all(&r.components).await?;
            r.stage = "snapshotting".into();
            record(&r)?;
            capture(&d, &r, "snapshot").await?;
            let manifest = Manifest {
                format: 1,
                created_at: r.created_at,
                compose: persisted(store::read(&store::root().join("compose.override.yaml")))?,
                deployment: d,
                services,
                components: r.components.clone(),
            };
            persisted(store::write_json(
                &work(&r.id).join("snapshot/manifest.json"),
                &manifest,
            ))?;
            restart(&r.components).await?;
            r.stage = "encrypting".into();
            record(&r)?;
            let source = work(&r.id).join("snapshot");
            let output = archive_root().join(format!("{}.age", r.id));
            tokio::task::spawn_blocking(move || {
                thelxinoe_backup::encrypt(&source, &output, input.passphrase)
            })
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| conflict("Backup encryption failed"))?;
            // Plaintext snapshots remain private until encryption has completed successfully.
            persisted(std::fs::remove_dir_all(work(&r.id).join("snapshot")).map_err(Into::into))?;
            Ok::<_, crate::docker::Failure>(())
        }
        .await;
        match result {
            Ok(()) => r.stage = "complete".into(),
            Err(e) => {
                r.stage = "failed".into();
                r.error = Some(e.1.to_string());
                if restart(&r.components).await.is_err() {
                    r.rollback_phase = Some(RollbackPhase::Activating);
                    r.stage = "rollback-activating".into();
                    r.error = Some("Backup failed; one or more components need restart".into());
                }
            }
        }
        let _ = record(&r);
    });
    Ok(Json(response))
}
pub(super) async fn restore(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    id(&key)?;
    check_input(&input)?;
    let guard = runtime
        .0
        .try_lock_owned()
        .map_err(|_| conflict("Another Docker operation is active"))?;
    let d = bootstrap().await?;
    let (current_services, components) = inspect(&d).await?;
    private_dir(&work(&key))?;
    let mut r: Record = if work(&key).join("operation.json").exists() {
        persisted(store::read(&work(&key).join("operation.json")))?
    } else {
        Record {
            id: key.clone(),
            stage: "imported".into(),
            created_at: thelxinoe_core::now(),
            error: None,
            components: vec![],
            recovery: None,
            recovery_ready: false,
            rollback_phase: None,
            release_restore: None,
        }
    };
    if !matches!(
        r.stage.as_str(),
        "complete" | "restored" | "restore-failed" | "imported"
    ) {
        return Err(conflict("Backup is not ready to restore"));
    }
    let archive = archive_root().join(format!("{key}.age"));
    let restore_id = thelxinoe_core::id();
    let stage = work(&key).join(&restore_id);
    private_dir(&stage)?;
    let output = stage.clone();
    tokio::task::spawn_blocking(move || {
        thelxinoe_backup::decrypt(&archive, &output, input.passphrase)
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| conflict("Passphrase or archive verification failed"))?;
    let manifest: Manifest = persisted(store::read(&stage.join("manifest.json")))?;
    // An archive cannot introduce a new Docker spec: match its authenticated
    // descriptor against one previously accepted by this controller.
    let accepted: Deployment = persisted(store::read(
        &store::root()
            .join("generations")
            .join(manifest.deployment.generation.to_string())
            .join("desired-state.json"),
    ))?;
    if manifest.format != 1
        || manifest.deployment.id != d.id
        || serde_json::to_value(&manifest.deployment).map_err(|_| unavailable())?
            != serde_json::to_value(&accepted).map_err(|_| unavailable())?
        || serde_json::to_value(&manifest.services).map_err(|_| unavailable())?
            != serde_json::to_value(&current_services).map_err(|_| unavailable())?
        || manifest.components.len() != components.len()
        || manifest.components.iter().zip(&components).any(|(a, b)| {
            a.key != b.key
                || (a.key != "server" && a.container != b.container)
                || a.source != b.source
        })
    {
        return Err(conflict(
            "Restore requires a previously accepted deployment and the same managed-service layout",
        ));
    }
    r.components = components;
    r.stage = "restore-queued".into();
    r.error = None;
    r.recovery = Some(format!("recovery-{restore_id}"));
    r.recovery_ready = false;
    r.rollback_phase = Some(RollbackPhase::Copying);
    let change_release = manifest.deployment.server["Image"] != d.server["Image"]
        || manifest.deployment.controller["Image"] != d.controller["Image"];
    if change_release {
        // Fail before stopping anything when a retained old image was removed.
        for raw in [&manifest.deployment.server, &manifest.deployment.controller] {
            engine(&format!("/images/{}/json", immutable(raw)?)).await?;
        }
    }
    r.release_restore = change_release.then(|| restore_id.clone());
    record(&r)?;
    let response = public(&r);
    crate::recovery::spawn(async move {
        let _guard = guard;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let recovery = format!("recovery-{restore_id}");
        let mut captured = false;
        let mut crossed = false;
        let result = async {
            stop_all(&r.components).await?;
            capture(&d, &r, &recovery).await?;
            captured = true;
            r.recovery_ready = true;
            r.stage = "restoring".into();
            record(&r)?;
            for (i, c) in r.components.iter().enumerate() {
                crossed = true;
                updates::copy_state(
                    &d,
                    &key,
                    &host(&d, &key, &format!("{restore_id}/{}", c.key)),
                    &c.source,
                    true,
                    &format!("r{i}"),
                )
                .await?;
            }
            // The compatible descriptor is authenticated in the archive and remains the accepted generation.
            for s in &manifest.services {
                save(s)?;
            }
            if change_release {
                r.stage = "release-handoff".into();
                record(&r)?;
                let source = r
                    .components
                    .iter()
                    .find(|c| c.key == "server")
                    .ok_or_else(unavailable)?
                    .source
                    .clone();
                product::restore_archive(
                    key.clone(),
                    restore_id.clone(),
                    manifest.deployment,
                    d.clone(),
                    source,
                )
                .await?;
                return Ok(());
            }
            r.stage = "restore-activating".into();
            record(&r)?;
            Ok::<_, crate::docker::Failure>(())
        }
        .await;
        match result {
            Ok(()) => {
                if change_release {
                    return;
                }
                // External work can resume after this boundary. Do not copy old state over running components.
                if restart(&r.components).await.is_ok() {
                    r.stage = "restored".into();
                } else {
                    r.stage = "restore-activating".into();
                    r.error = Some("State restored; component restart needs recovery".into());
                }
            }
            Err(e) => {
                if change_release && product::recovery_stage(&restore_id).is_some() {
                    r.stage = "release-handoff".into();
                    r.error = Some("State restored; resume deployment recovery in Product updates or through the private controller API".into());
                    let _ = record(&r);
                    return;
                }
                r.error = Some(e.1.to_string());
                if let Err(error) = rollback_original(&d, &mut r, crossed && captured).await {
                    r.error = Some(error.1.to_string());
                    if r.rollback_phase != Some(RollbackPhase::Activating) {
                        r.stage = "recovery-required".into();
                    }
                }
            }
        }
        if let Err(error) = record(&r) {
            eprintln!("Backup recovery journal failed: {}", error.1);
        }
    });
    Ok(Json(response))
}
async fn remove_interrupted_workers(d: &Deployment, r: &Record) -> Result<()> {
    let all = engine("/containers/json?all=true").await?;
    for c in all.as_array().ok_or_else(unavailable)? {
        if c["Labels"]["app.thelxinoe.update"] == r.id
            && c["Labels"]["app.thelxinoe.deployment"] == d.id
        {
            updates::remove(c["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    Ok(())
}
pub(super) async fn recover_interrupted() -> Result<()> {
    if !root().exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root()).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        if !entry.path().join("operation.json").exists() {
            continue;
        }
        let mut r: Record = persisted(store::read(&entry.path().join("operation.json")))?;
        validate_record(&r)?;
        if r.stage == "release-handoff" {
            match r
                .release_restore
                .as_deref()
                .and_then(product::recovery_stage)
                .as_deref()
            {
                Some("restored") => {
                    let d = bootstrap().await?;
                    let server = r
                        .components
                        .iter_mut()
                        .find(|c| c.key == "server")
                        .ok_or_else(unavailable)?;
                    server.container = d.server["Id"].as_str().ok_or_else(unavailable)?.into();
                    if let Err(error) = restart(&r.components).await {
                        r.error = Some(error.1.to_string());
                        record(&r)?;
                        continue;
                    }
                    r.stage = "restored".into();
                    r.error = None;
                    record(&r)?;
                    continue;
                }
                Some(_) => continue,
                None => {
                    r.stage = "restoring".into();
                    record(&r)?;
                }
            }
        }
        if !matches!(
            r.stage.as_str(),
            "queued"
                | "quiescing"
                | "snapshotting"
                | "encrypting"
                | "restore-queued"
                | "restoring"
                | "restore-activating"
                | "rollback-activating"
                | "rollback-copying"
                | "recovery-required"
        ) {
            continue;
        }
        id(&r.id)?;
        if r.recovery.is_none()
            || matches!(
                r.stage.as_str(),
                "restore-activating" | "rollback-activating"
            )
        {
            let terminal = if r.stage == "restore-activating" {
                "restored"
            } else {
                failure_stage(&r)
            };
            if terminal != "restored" {
                r.rollback_phase = Some(RollbackPhase::Activating);
                r.stage = "rollback-activating".into();
            }
            record(&r)?;
            let cleanup = async {
                let d = bootstrap().await?;
                remove_interrupted_workers(&d, &r).await
            }
            .await;
            let restarting = restart(&r.components).await;
            match cleanup.and(restarting) {
                Ok(()) => {
                    r.stage = terminal.into();
                    r.error = Some(
                        "Controller interrupted the operation; component startup recovered".into(),
                    );
                }
                Err(error) => r.error = Some(error.1.to_string()),
            }
            record(&r)?;
            continue;
        }
        let d = bootstrap().await?;
        let (_, current) = inspect(&d).await?;
        if r.components.len() != current.len()
            || r.components
                .iter()
                .zip(&current)
                .any(|(a, b)| a.key != b.key || a.container != b.container || a.source != b.source)
        {
            return Err(conflict("Interrupted backup component layout changed"));
        }
        remove_interrupted_workers(&d, &r).await?;
        let terminal = failure_stage(&r);
        let copy = matches!(
            r.stage.as_str(),
            "restoring" | "recovery-required" | "rollback-copying"
        );
        if let Err(error) = rollback_original(&d, &mut r, copy).await {
            if r.rollback_phase != Some(RollbackPhase::Activating) {
                return Err(error);
            }
            r.error = Some(error.1.to_string());
            record(&r)?;
            continue;
        }
        r.stage = terminal.into();
        r.error = Some("Controller interrupted the operation; component startup recovered".into());
        record(&r)?;
    }
    Ok(())
}
