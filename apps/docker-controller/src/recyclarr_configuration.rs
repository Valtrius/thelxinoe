use super::*;
use serde_yaml_ng::Value as Yaml;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Files = BTreeMap<String, String>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Bundle {
    pub image: String,
    pub resources: String,
    pub files: Files,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Configuration {
    pub revision: String,
    pub mode: String,
    pub active: Bundle,
    pub defaults: Bundle,
    pub base_defaults: Bundle,
    pub bindings: Vec<Target>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub operation_id: String,
    pub base_revision: String,
    pub image: String,
    pub files: Files,
    pub defaults: Bundle,
    pub valid: bool,
    pub edited: bool,
    pub diagnostics: Value,
}

#[derive(Serialize)]
pub(crate) struct Diagnostic {
    file: String,
    line: usize,
    column: usize,
    message: String,
}

fn location(root: &FsPath) -> PathBuf {
    root.join(".thelxinoe/configuration.json")
}

pub(crate) fn read_at(root: &FsPath) -> Result<Option<Configuration>> {
    if location(root).is_file() {
        Ok(Some(persisted(store::read(&location(root)))?))
    } else {
        Ok(None)
    }
}

pub(crate) fn read(key: &str) -> Result<Option<Configuration>> {
    read_at(&appdata(key))
}

fn candidate_path(root: &FsPath) -> PathBuf {
    root.join(".thelxinoe/candidate.json")
}

pub(crate) fn candidate_at(root: &FsPath) -> Result<Option<Candidate>> {
    if candidate_path(root).is_file() {
        Ok(Some(persisted(store::read(&candidate_path(root)))?))
    } else {
        Ok(None)
    }
}

pub(crate) fn write_candidate(root: &FsPath, candidate: &Candidate) -> Result<()> {
    persisted(store::write_json(&candidate_path(root), candidate))
}

fn default_bindings(root: &FsPath, bindings: &[Target]) -> Result<Vec<Target>> {
    let mut default_bindings = Vec::new();
    let mut managers = BTreeSet::new();
    for manager in bindings {
        if !managers.insert(manager.service_id.clone()) {
            continue;
        }
        for guide in resources_at(root, &manager.kind, "quality-profiles")? {
            let trash = guide["trash_id"].as_str().ok_or_else(unavailable)?;
            let mut binding = bindings
                .iter()
                .find(|binding| {
                    binding.service_id == manager.service_id && binding.trash_id == trash
                })
                .cloned()
                .unwrap_or_else(|| {
                    let mut binding = manager.clone();
                    binding.trash_id = trash.into();
                    binding.profile_id = None;
                    binding.groups = json!({"add":[],"skip":[]});
                    binding.overrides = json!({});
                    binding.reset_scores = true;
                    binding.quality_sizes = false;
                    binding
                });
            binding.secret.clear();
            default_bindings.push(binding);
        }
    }
    default_bindings.sort_by(|a, b| {
        (&a.kind, &a.service_id, &a.trash_id).cmp(&(&b.kind, &b.service_id, &b.trash_id))
    });
    Ok(default_bindings)
}

pub(crate) fn prepare_upgrade(
    root: &FsPath,
    old: &Managed,
    image: &str,
) -> Result<Option<Configuration>> {
    let Some(mut configuration) = read_at(root)? else {
        return Ok(None);
    };
    let default_bindings = default_bindings(root, &configuration.bindings)?;
    let defaults = bundle(root, image, &resource_revision_at(root)?, &default_bindings)?;
    let pending = candidate_at(&appdata(&old.id))?
        .filter(|candidate| candidate.image == image && candidate.edited);
    if let Some(pending) = pending {
        if pending.base_revision != configuration.revision {
            return Err(conflict(
                "Candidate draft is stale; review it against the current configuration",
            ));
        }
        configuration.active.files = pending.files;
        configuration.mode = "customized".into();
    } else if configuration.mode == "defaults" {
        configuration.active = defaults.clone();
        configuration.base_defaults = defaults.clone();
    }
    configuration.active.image = image.into();
    configuration.active.resources = defaults.resources.clone();
    configuration.defaults = defaults;
    revision(&mut configuration);
    Ok(Some(configuration))
}

pub(crate) async fn activate_upgrade(d: &Deployment, s: &Managed, operation: &str) -> Result<()> {
    let file = store::root()
        .join("updates")
        .join(operation)
        .join("recyclarr-configuration.json");
    if file.is_file() {
        let configuration: Configuration = persisted(store::read(&file))?;
        let source: String = persisted(store::read(
            &file.with_file_name("recyclarr-resources-source.json"),
        ))?;
        updates::copy_state(
            d,
            operation,
            &source,
            &format!("{}/services/{}/appdata/resources", d.appdata_source, s.id),
            true,
            "recyclarr-resources",
        )
        .await?;
        commit(s, &configuration)?;
        let candidate = candidate_path(&appdata(&s.id));
        if candidate.is_file() {
            persisted(std::fs::remove_file(candidate).map_err(Into::into))?;
        }
    }
    Ok(())
}

pub(crate) fn prepared_revision(operation: &str) -> Result<Option<String>> {
    let file = store::root()
        .join("updates")
        .join(operation)
        .join("recyclarr-configuration.json");
    if file.is_file() {
        Ok(Some(
            persisted(store::read::<Configuration>(&file))?.revision,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn view(configuration: &Configuration, mut candidate: Option<Candidate>) -> Value {
    if let Some(candidate) = &mut candidate
        && candidate.base_revision != configuration.revision
    {
        candidate.valid = false;
    }
    let bindings = configuration
        .bindings
        .iter()
        .map(|target| {
            (
                target.service_id.clone(),
                json!({
                    "service_id":target.service_id,"kind":target.kind,
                    "instance":instance(&target.service_id),
                    "base_url":format!("{}_base_url",instance(&target.service_id)),
                    "api_key":format!("{}_api_key",instance(&target.service_id))
                }),
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    json!({"initialized":true,"revision":configuration.revision,"mode":configuration.mode,
        "image":configuration.active.image,"files":configuration.active.files,
        "defaults":configuration.defaults,"base_defaults":configuration.base_defaults,
        "bindings":bindings,"candidate":candidate,"used_services":used_services(&configuration.active.files,&configuration.bindings)})
}

pub(crate) fn used_services(files: &Files, bindings: &[Target]) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for (file, content) in files {
        if file != "recyclarr.yml" && !file.starts_with("configs/") {
            continue;
        }
        let Ok(document) = serde_yaml_ng::from_str::<Yaml>(content) else {
            continue;
        };
        for kind in ["radarr", "sonarr"] {
            for (name, config) in document
                .get(kind)
                .and_then(Yaml::as_mapping)
                .into_iter()
                .flatten()
            {
                for target in bindings.iter().filter(|target| target.kind == kind) {
                    if name.as_str() == Some(instance(&target.service_id).as_str())
                        || tagged(&config["base_url"], "!secret")
                            == Some(format!("{}_base_url", instance(&target.service_id)).as_str())
                    {
                        used.insert(target.service_id.clone());
                    }
                }
            }
        }
    }
    used
}

pub(crate) fn instance(service: &str) -> String {
    format!("managed_{}", service.replace('-', ""))
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
        })
}

pub(crate) fn file_path(path: &str) -> bool {
    if matches!(path, "recyclarr.yml" | "settings.yml") {
        return true;
    }
    safe_relative(path)
        && (path.ends_with(".yml") || path.ends_with(".yaml"))
        && (path
            .strip_prefix("configs/")
            .is_some_and(|name| !name.contains('/'))
            || path.starts_with("includes/"))
}

fn diagnostic(file: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        file: file.into(),
        line: 1,
        column: 1,
        message: message.into(),
    }
}

fn tagged<'a>(value: &'a Yaml, tag: &str) -> Option<&'a str> {
    match value {
        Yaml::Tagged(value) if value.tag == tag => value.value.as_str(),
        _ => None,
    }
}

fn inspect(
    value: &Yaml,
    file: &str,
    bindings: &[Target],
    files: &Files,
    includes: &mut BTreeSet<String>,
    problems: &mut Vec<Diagnostic>,
    depth: usize,
) {
    if depth > 32 {
        problems.push(diagnostic(file, "YAML nesting exceeds 32 levels"));
        return;
    }
    match value {
        Yaml::Mapping(mapping) => {
            for (key, child) in mapping {
                let key = key.as_str().unwrap_or("");
                if matches!(key, "api_key" | "base_url") {
                    let expected = bindings.iter().any(|target| {
                        tagged(child, "!secret")
                            == Some(format!("{}_{key}", instance(&target.service_id)).as_str())
                    });
                    if !expected {
                        problems.push(diagnostic(
                            file,
                            format!("{key} must use a connected manager's !secret reference"),
                        ));
                    }
                }
                if key == "include" {
                    for include in child.as_sequence().into_iter().flatten() {
                        if let Some(path) = include.get("config").and_then(Yaml::as_str) {
                            let normalized = format!("includes/{path}");
                            if !safe_relative(path) || !files.contains_key(&normalized) {
                                problems.push(diagnostic(
                                    file,
                                    "Local include is missing or outside includes/",
                                ));
                            } else {
                                includes.insert(normalized);
                            }
                        }
                    }
                }
                inspect(child, file, bindings, files, includes, problems, depth + 1);
            }
        }
        Yaml::Sequence(sequence) => {
            for child in sequence {
                inspect(child, file, bindings, files, includes, problems, depth + 1);
            }
        }
        Yaml::Tagged(tag) => {
            let allowed = tag.tag == "!secret"
                && bindings.iter().any(|target| {
                    ["api_key", "base_url"].iter().any(|suffix| {
                        tag.value.as_str()
                            == Some(format!("{}_{suffix}", instance(&target.service_id)).as_str())
                    })
                });
            if !allowed {
                problems.push(diagnostic(file, "Use a connected manager's !secret reference; external files and environment values are unavailable"));
            }
        }
        _ => {}
    }
}

pub(crate) fn diagnostics(files: &Files, bindings: &[Target]) -> Vec<Diagnostic> {
    let mut problems = Vec::new();
    if files.is_empty()
        || files.len() > 64
        || files.values().map(String::len).sum::<usize>() > 1024 * 1024
    {
        return vec![diagnostic(
            "recyclarr.yml",
            "Choose 1–64 YAML files totaling at most 1 MiB",
        )];
    }
    if !files.contains_key("recyclarr.yml")
        && !files.keys().any(|name| name.starts_with("configs/"))
    {
        problems.push(diagnostic(
            "recyclarr.yml",
            "Keep recyclarr.yml or at least one file under configs/",
        ));
    }
    let mut graph = BTreeMap::new();
    let mut instances = BTreeSet::new();
    for (file, content) in files {
        if !file_path(file) || content.len() > 128 * 1024 || content.contains('\0') {
            problems.push(diagnostic(
                file,
                "Use a safe YAML path and keep each file below 128 KiB",
            ));
            continue;
        }
        let document: Yaml = match serde_yaml_ng::from_str(content) {
            Ok(value) => value,
            Err(error) => {
                let location = error.location();
                problems.push(Diagnostic {
                    file: file.clone(),
                    line: location.as_ref().map_or(1, |l| l.line()),
                    column: location.as_ref().map_or(1, |l| l.column()),
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !document.is_mapping() {
            problems.push(diagnostic(file, "The document must be a YAML mapping"));
            continue;
        }
        let mut includes = BTreeSet::new();
        inspect(
            &document,
            file,
            bindings,
            files,
            &mut includes,
            &mut problems,
            0,
        );
        graph.insert(file.clone(), includes);
        if file == "recyclarr.yml" || file.starts_with("configs/") {
            for kind in ["radarr", "sonarr"] {
                for (name, config) in document
                    .get(kind)
                    .and_then(Yaml::as_mapping)
                    .into_iter()
                    .flatten()
                {
                    let name = name.as_str().unwrap_or("");
                    if name.is_empty() || !instances.insert(name.to_owned()) {
                        problems.push(diagnostic(
                            file,
                            "Instance names must be unique across configuration files",
                        ));
                    }
                    let target = bindings.iter().find(|target| {
                        target.kind == kind
                            && (tagged(&config["base_url"], "!secret")
                                == Some(
                                    format!("{}_base_url", instance(&target.service_id)).as_str(),
                                )
                                || (config.get("base_url").is_none()
                                    && name == instance(&target.service_id)))
                    });
                    if target.is_none_or(|target| {
                        config.get("api_key").is_some_and(|key| {
                            tagged(key, "!secret")
                                != Some(
                                    format!("{}_api_key", instance(&target.service_id)).as_str(),
                                )
                        })
                    }) {
                        problems.push(diagnostic(file, "Each instance must bind its URL and API key to the same connected manager"));
                    }
                }
            }
        }
    }
    fn cycle(
        file: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        stack: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        if visited.contains(file) {
            return false;
        }
        if !stack.insert(file.into()) {
            return true;
        }
        let found = graph
            .get(file)
            .into_iter()
            .flatten()
            .any(|next| cycle(next, graph, stack, visited));
        stack.remove(file);
        visited.insert(file.into());
        found
    }
    for file in graph.keys() {
        if cycle(file, &graph, &mut BTreeSet::new(), &mut BTreeSet::new()) {
            problems.push(diagnostic(file, "Local includes form a cycle"));
        }
    }
    problems
}

pub(crate) fn configured_profiles(key: &str, files: &Files, bindings: &[Target]) -> Result<Value> {
    configured_profiles_at(&appdata(key), files, bindings)
}
pub(crate) fn configured_profiles_at(
    root: &FsPath,
    files: &Files,
    bindings: &[Target],
) -> Result<Value> {
    fn templates(path: &FsPath, found: &mut BTreeMap<String, String>) -> Result<()> {
        if !path.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(path).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let path = entry.path();
            if entry.file_type().map_err(|_| unavailable())?.is_dir() {
                if entry.file_name() != ".git" {
                    templates(&path, found)?;
                }
            } else if path
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
            {
                found.insert(
                    path.file_stem().unwrap().to_string_lossy().into(),
                    std::fs::read_to_string(path).map_err(|_| unavailable())?,
                );
            }
        }
        Ok(())
    }
    fn collect(
        value: &Yaml,
        files: &Files,
        templates: &BTreeMap<String, String>,
        depth: usize,
        profiles: &mut Vec<Yaml>,
    ) -> Result<()> {
        if depth > 32 {
            return Err(bad("Include nesting exceeds 32 levels"));
        }
        for include in value
            .get("include")
            .and_then(Yaml::as_sequence)
            .into_iter()
            .flatten()
        {
            let content = if let Some(path) = include.get("config").and_then(Yaml::as_str) {
                files.get(&format!("includes/{path}"))
            } else if let Some(name) = include.get("template").and_then(Yaml::as_str) {
                templates.get(name)
            } else {
                None
            };
            let content = content
                .ok_or_else(|| bad("An include could not be resolved for profile tracking"))?;
            collect(
                &serde_yaml_ng::from_str(content).map_err(|_| bad("Invalid include YAML"))?,
                files,
                templates,
                depth + 1,
                profiles,
            )?;
        }
        profiles.extend(
            value
                .get("quality_profiles")
                .and_then(Yaml::as_sequence)
                .into_iter()
                .flatten()
                .cloned(),
        );
        Ok(())
    }
    let mut template_files = BTreeMap::new();
    templates(
        &root.join("resources/config-templates/git/official"),
        &mut template_files,
    )?;
    let mut result = BTreeMap::new();
    for (file, content) in files
        .iter()
        .filter(|(name, _)| name.as_str() == "recyclarr.yml" || name.starts_with("configs/"))
    {
        let document: Yaml = serde_yaml_ng::from_str(content).map_err(|_| bad("Invalid YAML"))?;
        for kind in ["radarr", "sonarr"] {
            for (name, config) in document
                .get(kind)
                .and_then(Yaml::as_mapping)
                .into_iter()
                .flatten()
            {
                let binding = bindings
                    .iter()
                    .find(|target| {
                        target.kind == kind
                            && (tagged(&config["base_url"], "!secret")
                                == Some(
                                    format!("{}_base_url", instance(&target.service_id)).as_str(),
                                )
                                || name.as_str() == Some(instance(&target.service_id).as_str()))
                    })
                    .ok_or_else(|| bad("Bind this instance to a connected manager"))?;
                let mut profiles = Vec::new();
                collect(config, files, &template_files, 0, &mut profiles)?;
                for profile in profiles {
                    let trash = profile.get("trash_id").and_then(Yaml::as_str);
                    let guide = if let Some(trash) = trash {
                        resources_at(root, kind, "quality-profiles")?
                            .into_iter()
                            .find(|guide| guide["trash_id"] == trash)
                    } else {
                        None
                    };
                    let name = profile
                        .get("name")
                        .and_then(Yaml::as_str)
                        .or_else(|| guide.as_ref().and_then(|guide| guide["name"].as_str()))
                        .ok_or_else(|| bad("Quality profiles need a name or supported guide ID"))?;
                    result.insert((binding.service_id.clone(),name.to_owned()),json!({"service_id":binding.service_id,"kind":kind,"trash_id":trash,"name":name,"file":file}));
                }
            }
        }
    }
    Ok(json!(result.into_values().collect::<Vec<_>>()))
}

pub(crate) fn check_required(profiles: &Value, required_profiles: &[Value]) -> Result<()> {
    for required in required_profiles {
        let same_guide = required["trash_id"].is_string()
            && profiles
                .as_array()
                .into_iter()
                .flatten()
                .filter(|profile| {
                    profile["service_id"] == required["service_id"]
                        && profile["trash_id"] == required["trash_id"]
                })
                .count()
                == 1;
        if !same_guide
            && !profiles.as_array().into_iter().flatten().any(|profile| {
                profile["service_id"] == required["service_id"]
                    && profile["name"] == required["name"]
            })
        {
            return Err(conflict(
                "A selected acquisition profile would leave the configuration. Choose its replacement in the manager's defaults before saving.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn bundle(
    root: &FsPath,
    image: &str,
    resources: &str,
    bindings: &[Target],
) -> Result<Bundle> {
    let targets = bindings
        .iter()
        .map(|target| (target.clone(), String::new()))
        .collect::<Vec<_>>();
    let yaml = config_at(root, &targets)?;
    Ok(Bundle {
        image: image.into(),
        resources: resources.into(),
        files: BTreeMap::from([
            (
                "recyclarr.yml".into(),
                format!(
                    "# Recyclarr defaults · policy 1\n# Image: {image}\n{}",
                    if yaml.is_empty() { "{}\n" } else { &yaml }
                ),
            ),
            ("settings.yml".into(), local_settings().into()),
        ]),
    })
}

pub(crate) fn revision(configuration: &mut Configuration) {
    configuration.revision.clear();
    configuration.revision = digest(&serde_json::to_vec(configuration).unwrap());
}

pub(crate) fn write_at(root: &FsPath, configuration: &Configuration) -> Result<()> {
    persisted(store::write_json(
        &root
            .join(".thelxinoe/revisions")
            .join(format!("{}.json", configuration.revision)),
        configuration,
    ))?;
    persisted(store::write_json(&location(root), configuration))
}

pub(crate) fn materialize(root: &FsPath, files: &Files, identity: &Identity) -> Result<()> {
    for (name, content) in files {
        if !file_path(name) {
            return Err(bad("Invalid YAML file path"));
        }
        let mut path = root.to_path_buf();
        for component in name.split('/') {
            path.push(component);
            if std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(conflict("A configuration path contains a symbolic link"));
            }
        }
        if std::fs::read(&path).is_ok_and(|bytes| bytes == content.as_bytes()) {
            continue;
        }
        persisted(store::write(&path, content.as_bytes()))?;
        persisted(
            std::os::unix::fs::chown(&path, Some(identity.uid), Some(identity.gid))
                .map_err(Into::into),
        )?;
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| directory.starts_with(root)) {
            persisted(
                std::os::unix::fs::chown(directory, Some(identity.uid), Some(identity.gid))
                    .map_err(Into::into),
            )?;
            if directory == root {
                break;
            }
            parent = directory.parent();
        }
    }
    Ok(())
}

pub(crate) fn commit(s: &Managed, configuration: &Configuration) -> Result<()> {
    let root = appdata(&s.id);
    // The manifest is the commit point; CLI jobs only see their sealed copy.
    let previous = read_at(&root)?;
    if previous
        .as_ref()
        .is_some_and(|previous| previous.revision != configuration.revision)
    {
        updates::invalidate_recyclarr(&s.id)?;
    }
    write_at(&root, configuration)?;
    if let Some(previous) = previous {
        for name in previous
            .active
            .files
            .keys()
            .filter(|name| !configuration.active.files.contains_key(*name))
        {
            if file_path(name) && root.join(name).is_file() {
                persisted(std::fs::remove_file(root.join(name)).map_err(Into::into))?;
            }
        }
    }
    repair(s, configuration)
}

fn repair(s: &Managed, configuration: &Configuration) -> Result<()> {
    let root = appdata(&s.id);
    let identity = Identity::service("recyclarr", &s.spec)?;
    materialize(
        &root.join(".thelxinoe/defaults"),
        &configuration.defaults.files,
        &identity,
    )?;
    materialize(
        &root.join(".thelxinoe/customization-base"),
        &configuration.base_defaults.files,
        &identity,
    )?;
    materialize(&root, &configuration.active.files, &identity)
}

pub(crate) fn ensure(s: &Managed, targets: &[Target]) -> Result<Configuration> {
    let mut bindings = targets.to_vec();
    for target in &mut bindings {
        target.secret.clear();
        target.profile_id = None;
    }
    let resources = resource_revision(&s.id)?;
    let defaults = bundle(
        &appdata(&s.id),
        &s.image,
        &resources,
        &default_bindings(&appdata(&s.id), &bindings)?,
    )?;
    let mut configuration = read(&s.id)?.unwrap_or_else(|| Configuration {
        revision: String::new(),
        mode: "defaults".into(),
        active: defaults.clone(),
        defaults: defaults.clone(),
        base_defaults: defaults.clone(),
        bindings: bindings.clone(),
    });
    if configuration.active.image != s.image {
        return Err(conflict(
            "Recover the interrupted configuration upgrade before syncing",
        ));
    }
    let old_revision = configuration.revision.clone();
    configuration.bindings = bindings;
    configuration.active.resources = resources;
    configuration.defaults = defaults.clone();
    if configuration.mode == "defaults" {
        configuration.active = defaults.clone();
        configuration.base_defaults = defaults;
    }
    revision(&mut configuration);
    if old_revision != configuration.revision {
        commit(s, &configuration)?;
    }
    Ok(configuration)
}

pub(crate) fn for_run(
    key: &str,
    revision: Option<&str>,
    files: Option<Files>,
    bindings: &[Target],
) -> Result<Configuration> {
    let mut configuration =
        read(key)?.ok_or_else(|| conflict("Initialize Recyclarr configuration before syncing"))?;
    if revision.is_some_and(|revision| revision != configuration.revision) {
        return Err(conflict(
            "Configuration revision changed; reload or preview the current files",
        ));
    }
    if let Some(files) = files {
        if !diagnostics(&files, bindings).is_empty() {
            return Err(bad(
                "Invalid YAML file set; validate the draft before previewing",
            ));
        }
        configuration.active.files = files;
        configuration.mode = "customized".into();
    }
    configuration.bindings = bindings.to_vec();
    for target in &mut configuration.bindings {
        target.secret.clear();
        target.profile_id = None;
    }
    Ok(configuration)
}

pub(crate) async fn get(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    id(&key)?;
    let guard = runtime.0.try_service("recyclarr").ok();
    if guard.is_some() {
        updates::reconcile_recyclarr(&key).await?;
    }
    let s = load(&key)?;
    if s.kind != "recyclarr" {
        return Err(bad("Choose Recyclarr"));
    }
    match read(&key)? {
        Some(configuration) => {
            if guard.is_some() {
                repair(&s, &configuration)?;
            }
            Ok(Json(view(&configuration, candidate_at(&appdata(&key))?)))
        }
        None => Ok(Json(json!({"initialized":false,"mode":"defaults"}))),
    }
}

#[derive(Deserialize)]
pub(crate) struct EditorQuery {
    #[serde(default)]
    candidate: bool,
}

pub(crate) async fn editor(
    Path(key): Path<String>,
    axum::extract::Query(query): axum::extract::Query<EditorQuery>,
) -> Result<Json<Value>> {
    id(&key)?;
    let s = load(&key)?;
    if s.kind != "recyclarr" {
        return Err(bad("Choose Recyclarr"));
    }
    let candidate = if query.candidate {
        candidate_at(&appdata(&key))?
    } else {
        None
    };
    let image = candidate
        .as_ref()
        .map_or(s.image.as_str(), |candidate| &candidate.image);
    let metadata = engine(&format!("/images/{image}/json")).await?;
    let labels = &metadata["Config"]["Labels"];
    let source = labels["org.opencontainers.image.source"]
        .as_str()
        .unwrap_or("");
    let revision = labels["org.opencontainers.image.revision"]
        .as_str()
        .unwrap_or("");
    let revision = (source == "https://github.com/recyclarr/recyclarr"
        && revision.len() == 40
        && revision.bytes().all(|c| c.is_ascii_hexdigit()))
    .then_some(revision);
    let mut catalog = Vec::new();
    let root = candidate
        .as_ref()
        .map(candidate_root)
        .transpose()?
        .unwrap_or_else(|| appdata(&key));
    for kind in ["radarr", "sonarr"] {
        for folder in ["quality-profiles", "cf", "cf-groups"] {
            for row in resources_at(&root, kind, folder).unwrap_or_default() {
                if let (Some(id), Some(name)) = (row["trash_id"].as_str(), row["name"].as_str()) {
                    catalog.push(
                        json!({"label":id,"detail":format!("{kind} · {name}"),"type":"constant"}),
                    );
                }
            }
        }
    }
    Ok(Json(
        json!({"image":image,"version":labels["org.opencontainers.image.version"],"revision":revision,"catalog":catalog}),
    ))
}

fn candidate_root(candidate: &Candidate) -> Result<std::path::PathBuf> {
    id(&candidate.operation_id)?;
    let root = store::root().join("updates").join(&candidate.operation_id);
    let leaf: String = persisted(store::read(&root.join("recyclarr-candidate-root.json")))?;
    id(leaf
        .strip_prefix("recyclarr-validation-")
        .ok_or_else(unavailable)?)?;
    Ok(root.join(leaf).join("config"))
}

#[derive(Deserialize)]
pub(crate) struct Change {
    operation: String,
    revision: Option<String>,
    files: Option<Files>,
    #[serde(default)]
    targets: Vec<Target>,
    #[serde(default)]
    required_profiles: Vec<Value>,
    snapshots: Option<Value>,
}

pub(crate) async fn change(
    State(runtime): State<Runtime>,
    Path(key): Path<String>,
    Json(input): Json<Change>,
) -> Result<Json<Value>> {
    id(&key)?;
    let _guard = runtime.0.service("recyclarr").await;
    updates::reconcile_recyclarr(&key).await?;
    let s = load(&key)?;
    if s.kind != "recyclarr" || s.phase != "active" {
        return Err(conflict("Recyclarr is not ready"));
    }
    let d = bootstrap().await?;
    updates::verified(&s, &d).await?;
    if input.operation == "initialize" || input.operation == "reconcile" {
        return Ok(Json(view(
            &ensure(&s, &input.targets)?,
            candidate_at(&appdata(&key))?,
        )));
    }
    let mut configuration =
        read(&key)?.ok_or_else(|| conflict("Initialize configuration first"))?;
    if input.revision.as_deref() != Some(&configuration.revision) {
        return Err(conflict(
            "Configuration revision changed; reload the latest files before saving",
        ));
    }
    if input.operation == "capture" {
        let snapshots = input
            .snapshots
            .filter(Value::is_object)
            .ok_or_else(|| bad("Supply target snapshots"))?;
        persisted(store::write_json(
            &appdata(&key).join(".thelxinoe/target-snapshots.json"),
            &snapshots,
        ))?;
        persisted(store::write_json(
            &appdata(&key).join(".thelxinoe/required-profiles.json"),
            &input.required_profiles,
        ))?;
        return Ok(Json(view(&configuration, candidate_at(&appdata(&key))?)));
    }
    let files = if input.operation == "defaults" {
        configuration.defaults.files.clone()
    } else {
        input.files.ok_or_else(|| bad("Supply a YAML file set"))?
    };
    let problems = diagnostics(&files, &input.targets);
    if !problems.is_empty() {
        if input.operation == "validate" {
            return Ok(Json(json!({"valid":false,"diagnostics":problems})));
        }
        return Err(bad(
            "Invalid YAML file set; validate the draft to see its problems",
        ));
    }
    if input.operation == "inspect" {
        return Ok(Json(
            json!({"used_services":used_services(&files,&input.targets)}),
        ));
    }
    let profiles = if input.operation == "candidate" {
        let candidate = candidate_at(&appdata(&key))?
            .ok_or_else(|| conflict("Preflight a candidate image first"))?;
        configured_profiles_at(&candidate_root(&candidate)?, &files, &input.targets)?
    } else {
        configured_profiles(&key, &files, &input.targets)?
    };
    check_required(&profiles, &input.required_profiles)?;
    if input.operation == "candidate" {
        let mut candidate = candidate_at(&appdata(&key))?
            .ok_or_else(|| conflict("Preflight a candidate image first"))?;
        candidate.files = files;
        candidate.base_revision = configuration.revision.clone();
        candidate.valid = false;
        candidate.edited = true;
        candidate.diagnostics = json!([]);
        write_candidate(&appdata(&key), &candidate)?;
        updates::invalidate_recyclarr(&key)?;
        return Ok(Json(view(&configuration, Some(candidate))));
    }
    if !matches!(input.operation.as_str(), "save" | "defaults" | "validate") {
        return Err(bad("Unsupported configuration operation"));
    }
    let evidence = execute(
        key.clone(),
        Run {
            operation_id: thelxinoe_core::id(),
            image: s.image.clone(),
            resources: resource_revision(&key)?,
            targets: input.targets.clone(),
            preview: true,
            configuration_revision: Some(configuration.revision.clone()),
            files: Some(files.clone()),
            upstream: Value::Null,
        },
    )
    .await?
    .0;
    if evidence["state"] != "previewed" {
        if input.operation == "validate" {
            return Ok(Json(
                json!({"valid":false,"diagnostics":[{"file":"recyclarr.yml","line":1,"column":1,"message":"Recyclarr rejected this file set; inspect the preview output"}],"preview":evidence}),
            ));
        }
        return Err(conflict(
            "Recyclarr rejected the configuration; validate the draft to inspect its output",
        ));
    }
    if input.operation == "validate" {
        return Ok(Json(
            json!({"valid":true,"diagnostics":[],"preview":evidence}),
        ));
    }
    if input.operation == "defaults" {
        configuration.mode = "defaults".into();
        configuration.active = configuration.defaults.clone();
        configuration.base_defaults = configuration.defaults.clone();
    } else if files != configuration.active.files {
        if configuration.mode == "defaults" {
            configuration.base_defaults = configuration.defaults.clone();
        }
        configuration.mode = "customized".into();
        configuration.active.files = files;
    }
    configuration.bindings = input.targets;
    for target in &mut configuration.bindings {
        target.secret.clear();
        target.profile_id = None;
    }
    revision(&mut configuration);
    commit(&s, &configuration)?;
    Ok(Json(view(&configuration, candidate_at(&appdata(&key))?)))
}
