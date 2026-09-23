//! Remove stopped, owned containers before deleting their private configuration.
use super::*;
use std::path::{Path, PathBuf};

fn owned(raw: &Value, deployment: &str, service: &str, updates: &[String]) -> bool {
    let labels = &raw["Config"]["Labels"];
    labels["app.thelxinoe.deployment"] == deployment
        && (labels["app.thelxinoe.managed-id"] == service
            || labels["app.thelxinoe.removal"] == service
            || labels["app.thelxinoe.update"]
                .as_str()
                .is_some_and(|key| updates.iter().any(|id| id == key)))
}

fn checked_directory(root: &Path, path: &Path) -> Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| unavailable())?;
    if relative.as_os_str().is_empty() {
        return Err(unavailable());
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(unavailable());
        }
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            _ => {
                return Err(conflict(
                    "Service configuration must be inside managed storage",
                ));
            }
        }
    }
    Ok(())
}

async fn delete_directory(d: &Deployment, service: &str, root: &Path, path: &Path) -> Result<()> {
    checked_directory(root, path)?;
    if !path.exists() {
        return Ok(());
    }
    let relative = path.strip_prefix(root).map_err(|_| unavailable())?;
    let source = Path::new(&d.appdata_source).join(relative);
    // The regular controller has no DAC override. Use the same fixed-path,
    // isolated worker boundary as snapshots for service-owned appdata.
    let spec = json!({"Image":updates::current_image().await?,"Cmd":["appdata-remove"],
        "Healthcheck":{"Test":["NONE"]},"Labels":{"app.thelxinoe.removal":service,"app.thelxinoe.deployment":d.id},
        "HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,"CapDrop":["ALL"],
            "CapAdd":["DAC_OVERRIDE","FOWNER"],"SecurityOpt":["no-new-privileges:true"],
            "Memory":268435456,"NanoCpus":1000000000u64,"PidsLimit":32,
            "Mounts":[{"Type":"bind","Source":source,"Target":"/destination"}]}});
    let container = request(reqwest::Method::POST, "/containers/create", Some(spec)).await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let result = async {
        updates::start(&container).await?;
        updates::wait(&container, 120).await
    }
    .await;
    let cleaned = updates::remove(&container).await;
    result?;
    cleaned?;
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(unavailable()),
    }
}

pub(super) async fn remove(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if s.phase == "removed" {
        return Ok(Json(json!({"accepted":true,"removed":true})));
    }
    if !matches!(
        s.phase.as_str(),
        "active" | "creating" | "recreating" | "changing" | "removing"
    ) || adoption::pending(s)
    {
        return Err(conflict(
            "Finish the pending service operation before removal",
        ));
    }
    let updates = updates::removable_updates(&s.id)?;
    let root = store::root();
    let service_dir = service_path(&s.id)
        .parent()
        .ok_or_else(unavailable)?
        .to_path_buf();
    checked_directory(&root, &service_dir)?;
    let directories: Vec<PathBuf> = updates
        .iter()
        .map(|key| root.join("updates").join(key))
        .collect();
    for directory in &directories {
        checked_directory(&root, directory)?;
    }
    let retained = updates::retained_originals(d, s).await?;
    let rows = engine("/containers/json?all=true").await?;
    let controller = std::env::var("HOSTNAME").map_err(|_| unavailable())?;
    if controller.len() < 12 || !controller.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unavailable());
    }
    let mut containers = Vec::new();
    let protected: Vec<String> = std::iter::once(format!("{}/services/{}", d.appdata_source, s.id))
        .chain(
            updates
                .iter()
                .map(|key| format!("{}/updates/{key}", d.appdata_source)),
        )
        .collect();
    for row in rows.as_array().ok_or_else(unavailable)? {
        let key = row["Id"].as_str().ok_or_else(unavailable)?;
        // This controller owns the parent deployment mount used for deletion.
        if key.starts_with(&controller) {
            continue;
        }
        let raw = engine(&format!("/containers/{key}/json")).await?;
        if owned(&raw, &d.id, &s.id, &updates) {
            if raw["State"]["Running"] != false {
                return Err(conflict(
                    "Stop the service and its retained containers before removal",
                ));
            }
            if raw["Config"]["Labels"]["app.thelxinoe.managed-id"] == s.id
                && !retained.iter().any(|old| old == key)
            {
                verify_recorded(d, s, &raw).await?;
                if !s.expected.is_null() && policy::fingerprint(&raw) != s.expected {
                    return Err(conflict(
                        "Repair changed service configuration before removal",
                    ));
                }
            }
            containers.push(key.to_owned());
        } else if key == s.container
            || raw["Mounts"].as_array().into_iter().flatten().any(|mount| {
                mount["Source"].as_str().is_some_and(|source| {
                    protected
                        .iter()
                        .any(|path| !policy::media_disjoint(source, path))
                })
            })
        {
            return Err(conflict(
                "Another container still uses this service's configuration",
            ));
        }
    }
    s.phase = "removing".into();
    save(s)?;
    for container in containers {
        // No force: Docker refuses if an external actor starts it after inspection.
        match request(
            reqwest::Method::DELETE,
            &format!("/containers/{container}?v=true"),
            None,
        )
        .await
        {
            Ok(_) | Err((StatusCode::NOT_FOUND, _)) => {}
            Err(error) => return Err(error),
        }
    }
    for directory in directories {
        delete_directory(d, &s.id, &root, &directory).await?;
    }
    // Keep only a credential-free completion marker for lost-response retries.
    for entry in std::fs::read_dir(&service_dir).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        if entry.file_name() == "service.json" {
            continue;
        }
        if entry.file_type().map_err(|_| unavailable())?.is_dir() {
            delete_directory(d, &s.id, &root, &entry.path()).await?;
        } else {
            std::fs::remove_file(entry.path()).map_err(|_| unavailable())?;
        }
    }
    s.phase = "removed".into();
    s.spec = Value::Null;
    s.expected = Value::Null;
    s.container.clear();
    s.error = None;
    s.active_update = None;
    save(s)?;
    Ok(Json(json!({"accepted":true,"removed":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removal_ownership_excludes_other_services_and_deployments() {
        let raw = |deployment, service, update| {
            json!({"Config":{"Labels":{
            "app.thelxinoe.deployment":deployment,"app.thelxinoe.managed-id":service,
            "app.thelxinoe.update":update}}})
        };
        let updates = vec!["update".into()];
        assert!(owned(&raw("d", "s", ""), "d", "s", &updates));
        assert!(owned(&raw("d", "", "update"), "d", "s", &updates));
        assert!(!owned(&raw("other", "s", "update"), "d", "s", &updates));
        assert!(!owned(&raw("d", "other", "other"), "d", "s", &updates));
    }
    #[test]
    fn deletion_rejects_parent_paths_and_symlinks() {
        let root = std::env::temp_dir().join(thelxinoe_core::id());
        std::fs::create_dir_all(root.join("services")).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("services/linked")).unwrap();
        assert!(checked_directory(&root, &root).is_err());
        assert!(checked_directory(&root, &root.join("../outside")).is_err());
        assert!(checked_directory(&root, &root.join("services/linked/appdata")).is_err());
        assert!(checked_directory(&root, &root.join("services/missing/appdata")).is_ok());
        std::fs::remove_file(root.join("services/linked")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
