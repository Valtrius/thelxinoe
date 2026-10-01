//! Bounded CLI operations. A stopped definition container pins the installed image;
//! each operation has its own recoverable runtime identity and private key mount.
use super::*;
use reqwest::Method;
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path as FsPath, PathBuf},
};

fn appdata(key: &str) -> PathBuf {
    service_path(key).with_file_name("appdata")
}
fn write_config(s: &Managed, file: &str, bytes: &[u8]) -> Result<()> {
    let path = appdata(&s.id).join(file);
    persisted(store::write(&path, bytes))?;
    let identity = Identity::service("recyclarr", &s.spec)?;
    persisted(
        std::os::unix::fs::chown(path, Some(identity.uid), Some(identity.gid)).map_err(Into::into),
    )
}
fn journal(key: &str, run: &str) -> PathBuf {
    service_path(key)
        .with_file_name("runs")
        .join(run)
        .join("result.json")
}
pub(super) fn local_settings() -> &'static str {
    "resource_providers:\n  - name: pinned-guides\n    type: trash-guides\n    path: /config/resources/trash-guides/git/official\n    replace_default: true\n  - name: pinned-templates\n    type: config-templates\n    path: /config/resources/config-templates/git/official\n    replace_default: true\n"
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn state_directory(url: &str) -> String {
    let mut hash = 14695981039346656037_u64;
    for byte in url.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(1099511628211);
    }
    hash.to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn resource_revision(key: &str) -> Result<String> {
    let mut hash = Sha256::new();
    for repo in ["trash-guides", "config-templates"] {
        let git = appdata(key).join(format!("resources/{repo}/git/official/.git"));
        let head = std::fs::read_to_string(git.join("HEAD"))
            .map_err(|_| conflict("Guide resources are unavailable"))?;
        let revision = if let Some(reference) = head.trim().strip_prefix("ref: ") {
            if !reference.starts_with("refs/heads/") || reference.contains("..") {
                return Err(unavailable());
            }
            std::fs::read_to_string(git.join(reference))
                .or_else(|_| {
                    let packed = std::fs::read_to_string(git.join("packed-refs"))?;
                    packed
                        .lines()
                        .find_map(|line| {
                            line.split_once(' ')
                                .filter(|(_, r)| *r == reference)
                                .map(|(sha, _)| sha.to_owned())
                        })
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                "Missing guide reference",
                            )
                        })
                })
                .map_err(|_| unavailable())?
        } else {
            head
        };
        if revision.trim().len() != 40 || !revision.trim().bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(unavailable());
        }
        hash.update(revision.trim());
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn resources(key: &str, kind: &str, folder: &str) -> Result<Vec<Value>> {
    let path = appdata(key).join(format!(
        "resources/trash-guides/git/official/docs/json/{kind}/{folder}"
    ));
    let mut rows = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|_| unavailable())? {
        let entry = entry.map_err(|_| unavailable())?;
        if entry.path().extension().is_some_and(|e| e == "json") {
            let mut value: Value = persisted(store::read(&entry.path()))?;
            value["source_url"] = json!(format!(
                "https://github.com/TRaSH-Guides/Guides/blob/master/docs/json/{kind}/{folder}/{}",
                entry.file_name().to_string_lossy()
            ));
            rows.push(value);
        }
    }
    Ok(rows)
}
fn profile(key: &str, kind: &str, trash: &str) -> Result<Value> {
    resources(key, kind, "quality-profiles")?
        .into_iter()
        .find(|p| p["trash_id"] == trash)
        .ok_or_else(|| conflict("Selected guide ID is no longer available; choose another guide"))
}
async fn output(container: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .unix_socket("/var/run/docker.sock")
        .no_proxy()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| unavailable())?;
    let mut response = client
        .get(format!(
            "http://docker/containers/{container}/logs?stdout=true&stderr=true&tail=10000"
        ))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(conflict("CLI output exceeded its limit"));
        }
        bytes.extend(chunk);
    }
    // Recyclarr emits ANSI colors even with TERM=dumb. Normalize its output
    // before recognizing error records because the CLI can return zero on errors.
    let raw = String::from_utf8_lossy(&bytes);
    let mut chars = raw.chars().peekable();
    let mut text = String::new();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        } else if c != '\r' {
            text.push(c);
        }
    }
    Ok(text)
}
pub(super) async fn cleanup(d: &Deployment, key: &str) -> Result<()> {
    for row in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if row["Labels"]["app.thelxinoe.deployment"] == d.id
            && row["Labels"]["app.thelxinoe.recyclarr-service"] == key
        {
            updates::remove(row["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    let root = service_path(key).with_file_name("runtime");
    if root.exists() {
        persisted(std::fs::remove_dir_all(root).map_err(Into::into))?;
    }
    Ok(())
}
pub(super) async fn command(
    d: &Deployment,
    s: &Managed,
    operation: &str,
    args: &[String],
    runtime: Option<&str>,
) -> Result<(bool, String)> {
    let identity = Identity::service("recyclarr", &s.spec)?;
    let mut mounts = vec![
        json!({"Type":"bind","Source":format!("{}/services/{}/appdata",d.appdata_source,s.id),"Target":"/config"}),
    ];
    if let Some(runtime) = runtime {
        mounts.push(json!({"Type":"bind","Source":runtime,"Target":"/runtime","ReadOnly":true}));
    }
    let spec = json!({"Image":s.image,"User":identity.user(),"Cmd":args,"Tty":true,"Env":["TZ=UTC","TERM=dumb"],"Healthcheck":{"Test":["NONE"]},"Labels":{"app.thelxinoe.deployment":d.id,"app.thelxinoe.recyclarr-service":s.id,"app.thelxinoe.recyclarr-run":operation},"HostConfig":{"NetworkMode":d.network,"RestartPolicy":{"Name":"no"},"Mounts":mounts,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Memory":1073741824u64,"PidsLimit":128}});
    let container = request(
        Method::POST,
        &format!(
            "/containers/create?name=thelxinoe-recyclarr-{}",
            thelxinoe_core::id()
        ),
        Some(spec),
    )
    .await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let outcome = async {
        updates::start(&container).await?;
        let success = updates::wait(&container, 600).await.is_ok();
        let text = output(&container).await?;
        Ok((success && !text.contains("[ERR]"), text))
    }
    .await;
    updates::remove(&container).await?;
    outcome
}
#[derive(Deserialize)]
pub(super) struct Catalog {
    kind: String,
}
pub(super) async fn catalog(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Catalog>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service("recyclarr").await;
    if !matches!(input.kind.as_str(), "radarr" | "sonarr") {
        return Err(bad("Choose Radarr or Sonarr"));
    }
    let s = load(&key)?;
    let d = bootstrap().await?;
    if s.kind != "recyclarr" || s.phase != "active" {
        return Err(conflict("Recyclarr is not ready"));
    }
    updates::verified(&s, &d).await?;
    cleanup(&d, &key).await?;
    write_config(&s, "settings.yml", b"{}\n")?;
    let (success, text) = command(
        &d,
        &s,
        &thelxinoe_core::id(),
        &[
            "list".into(),
            "quality-profiles".into(),
            input.kind.clone(),
            "--raw".into(),
        ],
        None,
    )
    .await?;
    if !success {
        return Err(conflict(
            "Guide catalog refresh failed; check registry and GitHub access",
        ));
    }
    let mut items = Vec::new();
    for row in resources(&key, &input.kind, "quality-profiles")? {
        let trash = row["trash_id"].as_str().ok_or_else(unavailable)?;
        // v8.7 terminal output can wrap URLs. Cross-check CLI IDs against the
        // exact fetched JSON instead of guessing names from wrapped text.
        if !text
            .lines()
            .any(|line| line.split('\t').next() == Some(trash))
        {
            return Err(conflict("Recyclarr catalog output contract changed"));
        }
        let groups = resources(&key, &input.kind, "cf-groups")?
            .into_iter()
            .filter(|g| g["quality_profiles"]["include"].get(trash).is_some())
            .map(|g| json!({"trash_id":g["trash_id"],"name":g["name"],"default":g["default"]}))
            .collect::<Vec<_>>();
        items.push(
            json!({"trash_id":trash,"name":row["name"],"url":row["source_url"],"groups":groups}),
        );
    }
    if items.is_empty() {
        return Err(conflict("Guide catalog is empty"));
    }
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(Json(
        json!({"items":items,"image":s.image,"resources":resource_revision(&key)?}),
    ))
}
#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Target {
    pub service_id: String,
    pub kind: String,
    pub container_id: String,
    pub port: u16,
    pub url_base: String,
    pub generation: String,
    pub trash_id: String,
    pub revision: String,
    pub profile_id: Option<i64>,
    pub quality_sizes: bool,
    pub reset_scores: bool,
    pub groups: Value,
    pub overrides: Value,
    pub secret: String,
}
#[derive(Deserialize)]
pub(super) struct Run {
    operation_id: String,
    image: String,
    resources: String,
    targets: Vec<Target>,
    #[serde(default)]
    preview: bool,
}
fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
async fn target_url(d: &Deployment, target: &Target) -> Result<String> {
    id(&target.service_id)?;
    if !matches!(target.kind.as_str(), "radarr" | "sonarr")
        || target.container_id.len() != 64
        || !target.container_id.bytes().all(|b| b.is_ascii_hexdigit())
        || !target.url_base.starts_with('/') && !target.url_base.is_empty()
        || target.url_base.contains(['?', '#', '\\', '\n', '\r'])
    {
        return Err(bad("Invalid Recyclarr target"));
    }
    let raw = engine(&format!("/containers/{}/json", target.container_id)).await?;
    if raw["State"]["Running"] != true {
        return Err(conflict("Target is unavailable"));
    }
    let network = &raw["NetworkSettings"]["Networks"][&d.network];
    let address = network["IPAddress"]
        .as_str()
        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
        .ok_or_else(|| conflict("Target must share the server deployment network"))?;
    let address = if address.contains(':') {
        format!("[{address}]")
    } else {
        address.to_owned()
    };
    Ok(format!(
        "http://{address}:{}{}",
        target.port, target.url_base
    ))
}
fn config(key: &str, targets: &[(Target, String)]) -> Result<String> {
    let mut text = String::new();
    for kind in ["radarr", "sonarr"] {
        let rows = targets
            .iter()
            .filter(|(t, _)| t.kind == kind)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            continue;
        }
        text.push_str(&format!("{kind}:\n"));
        let mut managers = std::collections::BTreeMap::<&str, Vec<&(Target, String)>>::new();
        for row in rows {
            managers.entry(&row.0.service_id).or_default().push(row);
        }
        for profiles in managers.values() {
            let (manager, url) = profiles[0];
            text.push_str(&format!("  managed_{}:\n    base_url: {}\n    api_key: !file /runtime/{}\n    delete_old_custom_formats: false\n    quality_profiles:\n",manager.service_id.replace('-', ""),quote(url),manager.service_id));
            for (target, _) in profiles {
                let guide = profile(key, kind, &target.trash_id)?;
                let name = guide["name"].as_str().ok_or_else(unavailable)?;
                text.push_str(&format!("      - trash_id: {}\n        name: {}\n        reset_unmatched_scores:\n          enabled: {}\n",quote(&target.trash_id),quote(name),target.reset_scores));
                if let Some(value) = target.overrides.get("min_format_score") {
                    text.push_str(&format!("        min_format_score: {value}\n"));
                }
                if target.overrides.get("upgrade_allowed").is_some()
                    || target.overrides.get("upgrade_until_score").is_some()
                {
                    let allowed = target
                        .overrides
                        .get("upgrade_allowed")
                        .unwrap_or(&guide["upgradeAllowed"]);
                    if !allowed.is_boolean() {
                        return Err(conflict("Guide upgrade settings are unavailable"));
                    }
                    text.push_str(&format!("        upgrade:\n          allowed: {allowed}\n"));
                    if let Some(value) = target.overrides.get("upgrade_until_score") {
                        text.push_str(&format!("          until_score: {value}\n"));
                    }
                }
            }
            if profiles.iter().any(|(target, _)| target.quality_sizes) {
                text.push_str(&format!(
                    "    quality_definition:\n      type: {}\n",
                    if kind == "radarr" { "movie" } else { "series" }
                ));
            }
            let skip = profiles
                .iter()
                .flat_map(|(target, _)| {
                    target.groups["skip"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                })
                .collect::<std::collections::BTreeSet<_>>();
            let mut add = std::collections::BTreeMap::<String, Vec<Value>>::new();
            for (target, _) in profiles {
                for group in target.groups["add"].as_array().into_iter().flatten() {
                    let trash = group["trash_id"].as_str().ok_or_else(unavailable)?;
                    add.entry(trash.into())
                        .or_default()
                        .push(json!({"trash_id":target.trash_id}));
                }
            }
            if !skip.is_empty() {
                for group in resources(key, kind, "cf-groups")? {
                    let trash = group["trash_id"].as_str().ok_or_else(unavailable)?;
                    if !skip.contains(trash)
                        || !(group["default"] == true || group["default"] == "true")
                    {
                        continue;
                    }
                    for (target, _) in profiles {
                        if group["quality_profiles"]["include"]
                            .get(&target.trash_id)
                            .is_some()
                            && !target.groups["skip"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|id| id == trash)
                        {
                            add.entry(trash.into())
                                .or_default()
                                .push(json!({"trash_id":target.trash_id}));
                        }
                    }
                }
            }
            if !add.is_empty() || !skip.is_empty() {
                let groups = json!({"skip":skip,"add":add.into_iter().map(|(trash,mut profiles)|{profiles.sort_by_key(Value::to_string);profiles.dedup();json!({"trash_id":trash,"assign_scores_to":profiles})}).collect::<Vec<_>>()});
                text.push_str(&format!("    custom_format_groups: {groups}\n"));
            }
        }
    }
    Ok(text)
}
fn tracked_profile(key: &str, target: &Target, url: &str) -> Result<Option<i64>> {
    fn walk(path: &FsPath, trash: &str) -> Result<Option<i64>> {
        if !path.exists() {
            return Ok(None);
        }
        for entry in std::fs::read_dir(path).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            if entry.file_type().map_err(|_| unavailable())?.is_dir() {
                if let Some(id) = walk(&entry.path(), trash)? {
                    return Ok(Some(id));
                }
            } else if entry.path().extension().is_some_and(|e| e == "json") {
                let value: Value = persisted(store::read(&entry.path()))?;
                if let Some(id) = tracked_id(&value, trash) {
                    return Ok(Some(id));
                }
            }
        }
        Ok(None)
    }
    let directory = state_directory(url);
    walk(
        &appdata(key)
            .join("state")
            .join(&target.kind)
            .join(directory),
        &target.trash_id,
    )
}
fn tracked_id(value: &Value, trash: &str) -> Option<i64> {
    if let Some(object) = value.as_object() {
        if object
            .get("trash_id")
            .or(object.get("trashId"))
            .and_then(Value::as_str)
            == Some(trash)
        {
            for field in [
                "id",
                "quality_profile_id",
                "qualityProfileId",
                "serviceId",
                "service_id",
            ] {
                if let Some(id) = object.get(field).and_then(Value::as_i64) {
                    return Some(id);
                }
            }
        }
        if let Some(entry) = object.get(trash)
            && let Some(id) = entry.as_i64().or_else(|| entry["id"].as_i64())
        {
            return Some(id);
        }
        for child in object.values() {
            if let Some(id) = tracked_id(child, trash) {
                return Some(id);
            }
        }
    }
    for child in value.as_array().into_iter().flatten() {
        if let Some(id) = tracked_id(child, trash) {
            return Some(id);
        }
    }
    None
}
async fn upstream_hashes(
    d: &Deployment,
    s: &Managed,
    targets: &[(Target, String)],
    runtime_host: &str,
    operation: &str,
) -> Result<Value> {
    let identity = Identity::service("recyclarr", &s.spec)?;
    let mut rows = std::collections::BTreeMap::<String, Value>::new();
    for (target, url) in targets {
        let guide = profile(&s.id, &target.kind, &target.trash_id)?;
        let row = rows
            .entry(target.service_id.clone())
            .or_insert_with(|| json!({"service_id":target.service_id,"url":url,"profiles":[]}));
        row["profiles"]
            .as_array_mut()
            .ok_or_else(unavailable)?
            .push(json!({"name":guide["name"],"tracked_id":tracked_profile(&s.id,target,url)?}));
    }
    let rows = rows.into_values().collect::<Vec<_>>();
    let spec = json!({"Image":updates::current_image().await?,"User":identity.user(),"Cmd":["recyclarr-check",serde_json::to_string(&rows).map_err(|_|unavailable())?],"Tty":true,"Healthcheck":{"Test":["NONE"]},"Labels":{"app.thelxinoe.deployment":d.id,"app.thelxinoe.recyclarr-service":s.id,"app.thelxinoe.recyclarr-run":operation},"HostConfig":{"NetworkMode":d.network,"ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Memory":268435456u64,"PidsLimit":32,"Mounts":[{"Type":"bind","Source":runtime_host,"Target":"/runtime","ReadOnly":true}]}});
    let container = request(Method::POST, "/containers/create", Some(spec)).await?["Id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let outcome=async {
        updates::start(&container).await?;
        let valid=updates::wait(&container,120).await.is_ok();
        let log=output(&container).await?;
        if !valid {
            if log.contains("profile name collides") {return Err(conflict("Guide profile name exists without matching Recyclarr ownership; import its state or choose another guide"));}
            if log.contains("authorization") {return Err(conflict("Target API authorization failed; reconnect its credentials"));}
            return Err(conflict("Target settings are unavailable"));
        }
        serde_json::from_str::<Value>(&log).map_err(|_|conflict("Target settings verification contract changed"))
    }.await;
    updates::remove(&container).await?;
    outcome
}
pub(super) async fn run(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Run>,
) -> Result<Json<Value>> {
    id(&input.operation_id)?;
    let _guard = runtime.0.service("recyclarr").await;
    let path = journal(&key, &input.operation_id);
    if path.exists() {
        return Ok(Json(persisted(store::read(&path))?));
    }
    let s = load(&key)?;
    let d = bootstrap().await?;
    if s.kind != "recyclarr"
        || s.phase != "active"
        || s.image != input.image
        || resource_revision(&key)? != input.resources
    {
        return Err(conflict(
            "Image or guide revision changed; refresh before applying",
        ));
    }
    updates::verified(&s, &d).await?;
    cleanup(&d, &key).await?;
    let identity = Identity::service("recyclarr", &s.spec)?;
    let private = service_path(&key)
        .with_file_name("runtime")
        .join(&input.operation_id);
    persisted(std::fs::create_dir_all(&private).map_err(Into::into))?;
    persisted(
        std::os::unix::fs::chown(&private, Some(identity.uid), Some(identity.gid))
            .map_err(Into::into),
    )?;
    persisted(
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))
            .map_err(Into::into),
    )?;
    let outcome = async {
        let mut targets = Vec::new();
        for mut target in input.targets {
            if target.trash_id.len() != 32 || !target.trash_id.bytes().all(|b|b.is_ascii_hexdigit()) { return Err(bad("Invalid guide ID")); }
            let url = target_url(&d, &target).await?;
            let file = private.join(&target.service_id);
            persisted(store::write(&file,target.secret.as_bytes()))?;
            persisted(std::os::unix::fs::chown(&file,Some(identity.uid),Some(identity.gid)).map_err(Into::into))?;
            target.secret.clear();
            targets.push((target,url));
        }
        let text = config(&key,&targets)?;
        bind_state(&key,&targets)?;
        write_config(&s,"recyclarr.yml",text.as_bytes())?;
        write_config(&s,"settings.yml",local_settings().as_bytes())?;
        let runtime_host = format!("{}/services/{key}/runtime/{}",d.appdata_source,input.operation_id);
        let upstream=upstream_hashes(&d,&s,&targets,&runtime_host,&input.operation_id).await?;
        let (valid, preview) = command(&d,&s,&input.operation_id,&["sync".into(),"--preview".into(),"--log".into(),"info".into()],Some(&runtime_host)).await?;
        let valid = valid && !preview.contains("[ERR]");
        let mut evidence = json!({"id":input.operation_id,"image":s.image,"resources":input.resources,"config_hash":digest(text.as_bytes()),"upstream_hashes":upstream,"state":if valid {"previewed"} else {"blocked"},"preview":preview,"targets":[]});
        let checked=upstream_hashes(&d,&s,&targets,&runtime_host,&input.operation_id).await?;
        let stable=checked==upstream;
        evidence["upstream_hashes_before_apply"]=checked;
        if !stable { evidence["state"]=json!("invalidated"); evidence["error"]=json!("Target settings changed after preview; review the changes before retrying"); }
        if valid && stable && !input.preview {
            let (applied, log) = command(&d,&s,&input.operation_id,&["sync".into(),"--log".into(),"info".into()],Some(&runtime_host)).await?;
            evidence["state"] = json!(if applied && !log.contains("[ERR]") {"applied"} else {"partial"});
            evidence["apply"] = json!(log);
            let mut applied_targets=Vec::new();
            for (t,url) in &targets { applied_targets.push(json!({"service_id":t.service_id,"generation":t.generation,"revision":t.revision,"trash_id":t.trash_id,"profile_id":tracked_profile(&key,t,url)?})); }
            evidence["targets"]=json!(applied_targets);
        }
        // Never persist a key in evidence, including an upstream error response.
        let mut serialized = evidence.to_string();
        for entry in std::fs::read_dir(&private).map_err(|_| unavailable())? {
            let secret = std::fs::read_to_string(entry.map_err(|_| unavailable())?.path()).map_err(|_| unavailable())?;
            if !secret.is_empty() { serialized = serialized.replace(&secret,"[redacted]"); }
        }
        let evidence: Value = serde_json::from_str(&serialized).map_err(|_| unavailable())?;
        persisted(store::write_json(&path,&evidence))?;
        Ok(Json(evidence))
    }.await;
    cleanup(&d, &key).await?;
    outcome
}
pub(super) async fn result(Path((key, run)): Path<(String, String)>) -> Result<Json<Value>> {
    id(&key)?;
    id(&run)?;
    let path = journal(&key, &run);
    if !path.exists() {
        return Ok(Json(json!({"state":"interrupted"})));
    }
    Ok(Json(persisted(store::read(&path))?))
}

pub(super) async fn qualify(
    d: &Deployment,
    operation: &str,
    old: &Managed,
    image: &str,
    source: &str,
) -> Result<()> {
    let identity = Identity::service("recyclarr", &old.spec)?;
    let leaf = format!("recyclarr-validation-{}", thelxinoe_core::id());
    let root = store::root().join("updates").join(operation).join(&leaf);
    let host = format!("{}/updates/{operation}/{leaf}", d.appdata_source);
    persisted(std::fs::create_dir_all(root.join("config")).map_err(Into::into))?;
    updates::copy_state(
        d,
        operation,
        source,
        &format!("{host}/config"),
        false,
        "recyclarr-clone",
    )
    .await?;
    let config = root.join("config");
    if !config.join("resources/trash-guides/git/official").is_dir() {
        // Download only public resources. No target credentials or Arr access
        // are available to this catalog command; apply uses the offline fixture.
        persisted(store::write(&config.join("settings.yml"), b"{}\n"))?;
        persisted(
            std::os::unix::fs::chown(
                config.join("settings.yml"),
                Some(identity.uid),
                Some(identity.gid),
            )
            .map_err(Into::into),
        )?;
        let fetch=request(Method::POST,"/containers/create",Some(json!({"Image":image,"User":identity.user(),"Cmd":["list","quality-profiles","radarr","--raw"],"Tty":true,"Labels":{"app.thelxinoe.update":operation,"app.thelxinoe.deployment":d.id},"HostConfig":{"NetworkMode":d.network,"ReadonlyRootfs":true,"Tmpfs":{"/tmp":"rw,nosuid,nodev,size=256m"},"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Mounts":[{"Type":"bind","Source":format!("{host}/config"),"Target":"/config"}]}}))).await?["Id"].as_str().ok_or_else(unavailable)?.to_owned();
        let result = async {
            updates::start(&fetch).await?;
            updates::wait(&fetch, 200).await
        }
        .await;
        updates::remove(&fetch).await?;
        result?;
    }
    let settings = config.join("settings.yml");
    persisted(store::write(&settings, local_settings().as_bytes()))?;
    persisted(
        std::os::unix::fs::chown(&settings, Some(identity.uid), Some(identity.gid))
            .map_err(Into::into),
    )?;
    let secret = thelxinoe_core::id().replace('-', "");
    let keyfile = root.join("key");
    persisted(store::write(&keyfile, secret.as_bytes()))?;
    persisted(
        std::os::unix::fs::chown(&keyfile, Some(identity.uid), Some(identity.gid))
            .map_err(Into::into),
    )?;
    let mut yaml = String::new();
    let previous = std::fs::read_to_string(config.join("recyclarr.yml"))
        .ok()
        .and_then(|s| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&s).ok());
    for kind in ["radarr", "sonarr"] {
        let t = templates::find(kind).ok_or_else(unavailable)?;
        let directory = root.join(kind);
        persisted(std::fs::create_dir_all(&directory).map_err(Into::into))?;
        let xml = format!(
            "<Config><BindAddress>*</BindAddress><Port>{}</Port><ApiKey>{secret}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>Enabled</AuthenticationRequired><UpdateAutomatically>False</UpdateAutomatically></Config>",
            t.port.ok_or_else(unavailable)?
        );
        persisted(store::write(&directory.join("config.xml"), xml.as_bytes()))?;
        persisted(
            std::os::unix::fs::chown(directory.join("config.xml"), Some(1000), Some(1000))
                .map_err(Into::into),
        )?;
        let profiles = previous
            .as_ref()
            .and_then(|p| p.get(kind))
            .and_then(|p| p.as_mapping());
        yaml.push_str(&format!("{kind}:\n"));
        let fallback = if kind == "radarr" {
            "d1d67249d3890e49bc12e275d989a7e9"
        } else {
            "72dae194fc92bf828f32cde7744e51a1"
        };
        let mut selections = profiles
            .into_iter()
            .flatten()
            .filter_map(|(_, v)| {
                Some((
                    serde_yaml_ng::to_string(v.get("quality_profiles")?).ok()?,
                    v.get("custom_format_groups")
                        .and_then(|x| serde_yaml_ng::to_string(x).ok()),
                    v.get("quality_definition")
                        .and_then(|x| serde_yaml_ng::to_string(x).ok()),
                ))
            })
            .collect::<Vec<_>>();
        if selections.is_empty() {
            selections.push((format!("- trash_id: {fallback}\n"), None, None));
        }
        for (i, (selected, groups, sizes)) in selections.iter().enumerate() {
            yaml.push_str(&format!("  fixture_{kind}_{i}:\n    base_url: http://127.0.0.1:{}\n    api_key: !file /fixtures/key\n    delete_old_custom_formats: false\n    quality_profiles:\n",t.port.ok_or_else(unavailable)?));
            for line in selected.lines() {
                yaml.push_str(&format!("      {line}\n"));
            }
            for (field, configured) in [
                ("custom_format_groups", groups),
                ("quality_definition", sizes),
            ] {
                if let Some(configured) = configured {
                    yaml.push_str(&format!("    {field}:\n"));
                    for line in configured.lines() {
                        yaml.push_str(&format!("      {line}\n"));
                    }
                }
            }
        }
    }
    persisted(store::write(&config.join("recyclarr.yml"), yaml.as_bytes()))?;
    persisted(
        std::os::unix::fs::chown(
            config.join("recyclarr.yml"),
            Some(identity.uid),
            Some(identity.gid),
        )
        .map_err(Into::into),
    )?;
    let labels = json!({"app.thelxinoe.update":operation,"app.thelxinoe.deployment":d.id});
    let anchor=request(Method::POST,"/containers/create",Some(json!({"Image":updates::current_image().await?,"Cmd":["recyclarr-fixture"],"Labels":labels,"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":"none","ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"]}}))).await?["Id"].as_str().ok_or_else(unavailable)?.to_owned();
    updates::start(&anchor).await?;
    let outcome=async {
        for kind in ["radarr","sonarr"] {
            let t=templates::find(kind).ok_or_else(unavailable)?;
            pull_image(&format!("{}@{}",t.repository,t.digest)).await?;
            let fixture=request(Method::POST,"/containers/create",Some(json!({"Image":format!("{}@{}",t.repository,t.digest),"Labels":labels,"Env":["PUID=1000","PGID=1000"],"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":format!("container:{anchor}"),"Mounts":[{"Type":"bind","Source":format!("{host}/{kind}"),"Target":"/config"}]}}))).await?["Id"].as_str().ok_or_else(unavailable)?.to_owned();
            updates::start(&fixture).await?;
            let checker=request(Method::POST,"/containers/create",Some(json!({"Image":updates::current_image().await?,"Cmd":["adapter-health",kind],"User":"1000:1000","Tty":true,"Labels":labels,"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":format!("container:{anchor}"),"ReadonlyRootfs":true,"CapDrop":["ALL"],"Mounts":[{"Type":"bind","Source":format!("{host}/{kind}"),"Target":"/config","ReadOnly":true}]}}))).await?["Id"].as_str().ok_or_else(unavailable)?.to_owned();
            updates::start(&checker).await?;
            if let Err(error)=updates::wait(&checker,200).await {
                let logs=output(&checker).await.unwrap_or_default().replace(&secret,"[redacted]");
                persisted(store::write(&root.join(format!("{kind}-health.log")),logs.as_bytes()))?;
                return Err(error);
            }
            updates::remove(&checker).await?;
        }
        for preview in [true,false] {
            let mut args=vec!["sync","--log","info"];
            if preview { args.push("--preview"); }
            let candidate=request(Method::POST,"/containers/create",Some(json!({"Image":image,"User":identity.user(),"Cmd":args,"Tty":true,"Env":["TERM=dumb"],"Labels":labels,"Healthcheck":{"Test":["NONE"]},"HostConfig":{"NetworkMode":format!("container:{anchor}"),"ReadonlyRootfs":true,"Tmpfs":{"/tmp":"rw,nosuid,nodev,size=256m"},"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"Mounts":[{"Type":"bind","Source":format!("{host}/config"),"Target":"/config"},{"Type":"bind","Source":host,"Target":"/fixtures","ReadOnly":true}]}}))).await?["Id"].as_str().ok_or_else(unavailable)?.to_owned();
            updates::start(&candidate).await?;
            let validation=updates::wait(&candidate,200).await;
            let log=output(&candidate).await.unwrap_or_default().replace(&secret,"[redacted]");
            persisted(store::write(&root.join(if preview {"preview.log"} else {"apply.log"}),log.as_bytes()))?;
            validation?;
            if log.contains("[ERR]") { return Err(conflict("Recyclarr candidate failed isolated guide application")); }
            updates::remove(&candidate).await?;
        }
        Ok(())
    }.await;
    let rows = engine("/containers/json?all=true").await?;
    for row in rows.as_array().ok_or_else(unavailable)? {
        if row["Labels"]["app.thelxinoe.update"] == operation && row["Id"] != anchor {
            updates::remove(row["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    updates::remove(&anchor).await?;
    if keyfile.exists() {
        persisted(std::fs::remove_file(keyfile).map_err(Into::into))?;
    }
    for kind in ["radarr", "sonarr"] {
        let file = root.join(kind).join("config.xml");
        if file.exists() {
            persisted(std::fs::remove_file(file).map_err(Into::into))?;
        }
    }
    outcome
}

pub(super) async fn review_config(d: &Deployment, review: &str, raw: &Value) -> Result<Value> {
    let operation = thelxinoe_core::id();
    let path = store::root().join("adoption-reviews").join(&operation);
    persisted(std::fs::create_dir_all(&path).map_err(Into::into))?;
    let source = mount(raw, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?;
    let host = format!("{}/adoption-reviews/{operation}", d.appdata_source);
    updates::copy_state(d, &operation, source, &host, false, "recyclarr-review").await?;
    let outcome = (|| {
        let text = std::fs::read_to_string(path.join("recyclarr.yml"))
            .map_err(|_| conflict("A supported recyclarr.yml is required"))?;
        let config: Value = serde_yaml_ng::from_str(&text).map_err(|_| {
            conflict("Import supports plain API keys and guide profiles in recyclarr.yml")
        })?;
        let root = config
            .as_object()
            .ok_or_else(|| conflict("Unsupported Recyclarr configuration"))?;
        if root
            .keys()
            .any(|k| !matches!(k.as_str(), "radarr" | "sonarr"))
        {
            return Err(conflict(
                "Import supports Radarr and Sonarr guide profiles only",
            ));
        }
        if path.join("configs").exists()
            && std::fs::read_dir(path.join("configs"))
                .map_err(|_| unavailable())?
                .next()
                .is_some()
        {
            return Err(conflict(
                "Merge split configuration before importing Recyclarr",
            ));
        }
        if let Ok(text) = std::fs::read_to_string(path.join("settings.yml")) {
            let settings: Value = serde_yaml_ng::from_str(&text)
                .map_err(|_| conflict("Unsupported Recyclarr settings"))?;
            if !settings.is_null() && !settings.as_object().is_some_and(|s| s.is_empty()) {
                return Err(conflict(
                    "Import requires official default resources and settings",
                ));
            }
        }
        let mut targets = Vec::new();
        for kind in ["radarr", "sonarr"] {
            for (_, instance) in config[kind].as_object().into_iter().flatten() {
                let fields = instance
                    .as_object()
                    .ok_or_else(|| conflict("Unsupported Recyclarr instance"))?;
                if fields.keys().any(|k| {
                    !matches!(
                        k.as_str(),
                        "base_url"
                            | "api_key"
                            | "delete_old_custom_formats"
                            | "quality_profiles"
                            | "custom_format_groups"
                            | "quality_definition"
                    )
                }) || instance["delete_old_custom_formats"] == true
                {
                    return Err(conflict(
                        "Import supports guide profiles without destructive cleanup or naming settings",
                    ));
                }
                let profiles = instance["quality_profiles"]
                    .as_array()
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| {
                        conflict("Import requires guide-backed profiles per instance")
                    })?;
                let mut seen = std::collections::HashSet::new();
                for profile in profiles {
                    if profile.as_object().is_none_or(|p| {
                        p.keys().any(|k| {
                            !matches!(
                                k.as_str(),
                                "trash_id"
                                    | "name"
                                    | "reset_unmatched_scores"
                                    | "min_format_score"
                                    | "upgrade"
                            )
                        })
                    }) || profile["reset_unmatched_scores"]["enabled"] == true
                    {
                        return Err(conflict("Unsupported imported profile overrides"));
                    }
                    let trash = profile["trash_id"]
                        .as_str()
                        .filter(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                        .ok_or_else(|| conflict("Import requires guide trash_id selections"))?;
                    if !seen.insert(trash) {
                        return Err(conflict("Imported guide IDs must be unique per instance"));
                    }
                    let guide_dir = path.join(format!(
                        "resources/trash-guides/git/official/docs/json/{kind}/quality-profiles"
                    ));
                    let guides=std::fs::read_dir(guide_dir).map_err(|_|conflict("Run the original Recyclarr once to populate official guide resources before importing"))?;
                    let selected = guides
                        .filter_map(|e| e.ok())
                        .filter_map(|e| std::fs::read(e.path()).ok())
                        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
                        .find(|g| g["trash_id"] == trash)
                        .ok_or_else(|| {
                            conflict("Imported guide is absent from official resources")
                        })?;
                    let mut groups = if instance["custom_format_groups"].is_null() {
                        json!({"add":[],"skip":[]})
                    } else {
                        instance["custom_format_groups"].clone()
                    };
                    if groups
                        .as_object()
                        .is_none_or(|g| g.keys().any(|k| !matches!(k.as_str(), "add" | "skip")))
                    {
                        return Err(conflict("Unsupported imported custom-format groups"));
                    }
                    if let Some(add) = groups["add"].as_array_mut() {
                        for group in add.iter() {
                            if group.as_object().is_none_or(|g| {
                                g.keys().any(|key| {
                                    !matches!(key.as_str(), "trash_id" | "assign_scores_to")
                                })
                            }) {
                                return Err(conflict("Unsupported imported group override"));
                            }
                            if let Some(assign) = group.get("assign_scores_to") {
                                let assign =
                                    assign.as_array().filter(|a| !a.is_empty()).ok_or_else(
                                        || conflict("Unsupported imported score assignments"),
                                    )?;
                                if assign.iter().any(|a| {
                                    a.as_object().is_none_or(|o| o.len() != 1)
                                        || !profiles.iter().any(|p| p["trash_id"] == a["trash_id"])
                                }) {
                                    return Err(conflict(
                                        "Imported score assignments must reference configured guide IDs",
                                    ));
                                }
                            }
                        }
                        add.retain(|group| {
                            group.get("assign_scores_to").is_none_or(|assign| {
                                assign
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|p| p["trash_id"] == trash)
                            })
                        });
                        for group in add {
                            if let Some(map) = group.as_object_mut() {
                                map.remove("assign_scores_to");
                            }
                        }
                    }
                    for field in ["add", "skip"] {
                        if groups.get(field).is_none() {
                            groups[field] = json!([]);
                        }
                        for group in groups[field]
                            .as_array()
                            .ok_or_else(|| conflict("Unsupported imported custom-format groups"))?
                        {
                            let group_id = if field == "add" {
                                group["trash_id"].as_str()
                            } else {
                                group.as_str()
                            };
                            if group_id.is_none()
                                || (field == "add"
                                    && group.as_object().is_none_or(|g| g.len() != 1))
                            {
                                return Err(conflict("Unsupported imported group override"));
                            }
                            let group_dir = path.join(format!(
                                "resources/trash-guides/git/official/docs/json/{kind}/cf-groups"
                            ));
                            let compatible = std::fs::read_dir(group_dir)
                                .map_err(|_| unavailable())?
                                .filter_map(|e| e.ok())
                                .filter_map(|e| std::fs::read(e.path()).ok())
                                .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
                                .any(|g| {
                                    g["trash_id"].as_str() == group_id
                                        && g["quality_profiles"]["include"].get(trash).is_some()
                                });
                            if !compatible {
                                return Err(conflict(
                                    "Imported custom-format group is incompatible with its profile",
                                ));
                            }
                        }
                    }
                    if !instance["quality_definition"].is_null()
                        && (instance["quality_definition"]
                            .as_object()
                            .is_none_or(|m| m.len() != 1)
                            || instance["quality_definition"]["type"]
                                != if kind == "radarr" { "movie" } else { "series" })
                    {
                        return Err(conflict(
                            "Imported quality-size overrides cannot be represented",
                        ));
                    }
                    let mut overrides = json!({});
                    let upgrade = &profile["upgrade"];
                    if !upgrade.is_null()
                        && upgrade.as_object().is_none_or(|m| {
                            m.keys()
                                .any(|k| !matches!(k.as_str(), "allowed" | "until_score"))
                                || !upgrade["allowed"].is_boolean()
                        })
                    {
                        return Err(conflict("Unsupported imported upgrade settings"));
                    }
                    for (field, value) in [
                        ("min_format_score", profile.get("min_format_score")),
                        ("upgrade_until_score", upgrade.get("until_score")),
                        ("upgrade_allowed", upgrade.get("allowed")),
                    ] {
                        if let Some(value) = value {
                            if !(if field == "upgrade_allowed" {
                                value.is_boolean()
                            } else {
                                value
                                    .as_i64()
                                    .is_some_and(|v| (-100000..=100000).contains(&v))
                            }) {
                                return Err(conflict("Invalid imported profile override"));
                            }
                            overrides[field] = value.clone();
                        }
                    }
                    let secret = instance["api_key"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| conflict("Import requires directly configured API keys"))?;
                    let url = instance["base_url"]
                        .as_str()
                        .ok_or_else(|| conflict("Import requires an explicit target URL"))?;
                    targets.push(json!({"kind":kind,"trash_id":trash,"secret":secret,"url":url,"quality_sizes":!instance["quality_definition"].is_null(),"groups":groups,"overrides":overrides,"guide_name":selected["name"]}));
                }
            }
        }
        if targets.is_empty() {
            return Err(conflict(
                "Import requires a configured Radarr or Sonarr target",
            ));
        }
        let hash_path = store::root()
            .join("adoption-reviews")
            .join(format!("{review}.config-hash"));
        let hash = digest(text.as_bytes());
        if hash_path.exists() {
            if std::fs::read_to_string(&hash_path).map_err(|_| unavailable())? != hash {
                return Err(conflict(
                    "Recyclarr configuration changed after review; review it again",
                ));
            }
        } else {
            persisted(store::write(&hash_path, hash.as_bytes()))?;
        }
        Ok(json!({"targets":targets,"hash":digest(text.as_bytes())}))
    })();
    persisted(std::fs::remove_dir_all(path).map_err(Into::into))?;
    outcome
}

fn bind_state(key: &str, targets: &[(Target, String)]) -> Result<()> {
    let file = appdata(key).join("state-bindings.json");
    let mut bindings: Value = if file.exists() {
        persisted(store::read(&file))?
    } else {
        json!({})
    };
    for (target, url) in targets {
        let current = state_directory(url);
        if let Some(previous) = bindings[&target.service_id]
            .as_str()
            .filter(|p| p.len() == 16 && p.bytes().all(|b| b.is_ascii_hexdigit()))
            && previous != current
        {
            let parent = appdata(key).join("state").join(&target.kind);
            let old = parent.join(previous);
            let next = parent.join(&current);
            if old.is_dir() && !next.exists() {
                persisted(std::fs::rename(&old, &next).map_err(Into::into))?;
            }
        }
        bindings[&target.service_id] = json!(current);
    }
    persisted(store::write_json(&file, &bindings))
}
pub(super) async fn import(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service("recyclarr").await;
    let s = load(&key)?;
    if appdata(&key).join("state-bindings.json").exists() {
        return Ok(Json(json!({"accepted":true})));
    }
    if s.kind != "recyclarr" || !adoption::pending(&s) {
        return Err(conflict(
            "Only a pending Recyclarr transfer can import state",
        ));
    }
    let config: Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(appdata(&key).join("recyclarr.yml")).map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    let mut bindings = json!({});
    for target in input["targets"].as_array().ok_or_else(unavailable)? {
        let service = target["service_id"].as_str().ok_or_else(unavailable)?;
        id(service)?;
        let kind = target["kind"]
            .as_str()
            .filter(|k| matches!(*k, "radarr" | "sonarr"))
            .ok_or_else(unavailable)?;
        let instance = config[kind]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, v)| v)
            .find(|v| {
                v["quality_profiles"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|profile| profile["trash_id"] == target["trash_id"])
            })
            .ok_or_else(unavailable)?;
        bindings[service] = json!(state_directory(
            instance["base_url"].as_str().ok_or_else(unavailable)?
        ));
    }
    persisted(store::write_json(
        &appdata(&key).join("state-bindings.json"),
        &bindings,
    ))?;
    Ok(Json(json!({"accepted":true})))
}
