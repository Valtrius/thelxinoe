#[path = "../storage/tools.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::{RwLock, atomic::Ordering},
    time::Duration,
};
use thelxinoe_core::{Capability, now};
pub(crate) use thelxinoe_tools::{Candidate, Executable, Generation, OnlineSnapshot};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{Notify, watch},
};

const PYTHON: &str = "/usr/local/bin/python3";
const PACKAGE: &str = include_str!("package.py");

#[derive(Debug)]
struct PackageFailure {
    message: String,
    retry_at: Option<i64>,
}
impl std::fmt::Display for PackageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for PackageFailure {}

pub struct Manager {
    pub runtime: thelxinoe_tools::Runtime,
    root: PathBuf,
    wake: Notify,
    validated: RwLock<HashSet<String>>,
    ready: watch::Sender<bool>,
}
impl Manager {
    pub(crate) async fn wait_ready(&self) {
        let mut ready = self.ready.subscribe();
        let _ = ready.wait_for(|ready| *ready).await;
    }

    fn validated(&self, generations: &[Generation]) {
        self.validated
            .write()
            .unwrap()
            .extend(generations.iter().map(|g| g.id.clone()));
    }

    pub(crate) fn temporary(&self) -> anyhow::Result<tempfile::TempDir> {
        tempfile::Builder::new()
            .prefix(".run-")
            .tempdir_in(&self.root)
            .context("Create tool execution directory")
    }

    pub fn new(state: &std::path::Path) -> Self {
        let root = state
            .canonicalize()
            .unwrap_or_else(|_| state.to_path_buf())
            .join("tools");
        Self {
            runtime: thelxinoe_tools::Runtime::new(root.clone()),
            root,
            wake: Notify::new(),
            validated: RwLock::new(HashSet::new()),
            ready: watch::channel(true).0,
        }
    }
}
#[derive(Clone, Serialize)]
pub(super) struct Tool {
    id: String,
    policy: String,
    channel: String,
    pinned: bool,
    revision: i64,
    installed: Option<String>,
    previous: Option<String>,
    held: Option<String>,
    candidate: Option<Candidate>,
    #[serde(skip)]
    candidate_json: Option<String>,
    checked_at: Option<i64>,
    next_check_at: i64,
    check_error: Option<String>,
    integrity_error: Option<String>,
}
#[derive(Clone, Serialize)]
pub(super) struct Job {
    id: String,
    tool: String,
    action: String,
    candidate: Option<Candidate>,
    #[serde(skip)]
    candidate_json: Option<String>,
    manual: bool,
    revision: i64,
    generation: Option<String>,
    stage: String,
    received: u64,
    total: u64,
    reason: Option<String>,
    error: Option<String>,
    validation: Option<Value>,
    retry_at: i64,
}
impl Job {
    fn pending(&self) -> bool {
        !["complete", "failed", "canceled"].contains(&self.stage.as_str())
    }
    fn startup(&self) -> bool {
        ["bootstrap", "validate"].contains(&self.action.as_str())
    }
}
#[derive(Clone, Deserialize)]
pub(super) struct Settings {
    policy: String,
    channel: String,
    pinned: bool,
}
#[derive(Deserialize)]
struct Action {
    candidate_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/tools", get(status))
        .route("/api/v1/admin/tools/check", post(check))
        .route("/api/v1/admin/tools/{tool}/settings", post(settings))
        .route("/api/v1/admin/tools/{tool}/{action}", post(action))
}
fn supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64")) && std::path::Path::new(PYTHON).is_file()
}
async fn tool(state: &AppState, id: &str) -> Result<Tool> {
    storage::inventory(&state.db)
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(ApiError::not_found)
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let default = crate::product::configured_policy(&state).await?;
    let generations = storage::generations(&state.db).await?;
    let jobs = storage::jobs(&state.db).await?;
    let summary = |id: &Option<String>| {
        id.as_ref().and_then(|id|generations.iter().find(|g|&g.id==id)).map(|g|json!({"id":g.id,"candidate_id":g.candidate.id,"version":g.candidate.version,"channel":g.candidate.channel}))
    };
    let items = storage::inventory(&state.db)
        .await?
        .into_iter()
        .map(|t| {
            let mut value = json!(t);
            value["installed"] = json!(summary(&t.installed));
            value["previous"] = json!(summary(&t.previous));
            value["effective_policy"] = json!(if t.policy == "inherit" {
                &default.policy
            } else {
                &t.policy
            });
            value["job"] = json!(jobs.iter().find(|j| j.tool == t.id));
            value
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"items":items,"supported":supported()})))
}
async fn settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Settings>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer)
        .await?
        .user
        .id;
    tool(&state, &id).await?;
    if !["inherit", "notify", "automatic"].contains(&input.policy.as_str())
        || !match id.as_str() {
            "yt-dlp" => ["nightly", "stable"].contains(&input.channel.as_str()),
            "deno" => input.channel == "lts",
            _ => input.channel == "stable",
        }
    {
        return Err(ApiError::bad("Invalid tool policy or channel"));
    }
    storage::settings(&state.db, id, input, actor).await?;
    state.tools.wake.notify_one();
    changed(&state).await?;
    Ok(Json(json!({"saved":true})))
}
async fn check(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer)
        .await?
        .user
        .id;
    if !supported() {
        return Err(ApiError::conflict(
            "Managed server tools require the Linux x86-64 server image",
        ));
    }
    let jobs = storage::jobs(&state.db).await?;
    for t in storage::inventory(&state.db).await? {
        if !jobs
            .iter()
            .any(|j| j.tool == t.id && j.action == "check" && j.pending())
        {
            storage::enqueue(
                &state.db,
                t,
                "check".into(),
                None,
                true,
                Some(actor.clone()),
            )
            .await?;
        }
    }
    state.tools.wake.notify_one();
    Ok(Json(json!({"accepted":true})))
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer)
        .await?
        .user
        .id;
    if !supported() {
        return Err(ApiError::conflict(
            "Managed server tools require the Linux x86-64 server image",
        ));
    }
    let t = tool(&state, &id).await?;
    if action == "check" {
        let queued = storage::enqueue(&state.db, t, action, None, true, Some(actor)).await?;
        state.tools.wake.notify_one();
        changed(&state).await?;
        return Ok(Json(json!({"accepted":true,"job_id":queued})));
    }
    if t.pinned {
        return Err(ApiError::conflict(
            "Unpin the tool before changing its installed version",
        ));
    }
    let generations = storage::generations(&state.db).await?;
    let candidate = match action.as_str() {
        "install" => t
            .candidate
            .clone()
            .filter(|c| Some(&c.id) == input.candidate_id.as_ref())
            .ok_or_else(|| {
                ApiError::conflict("The candidate changed; check for updates and retry")
            })?,
        "repair" | "rollback" => {
            let selected = if action == "repair" {
                &t.installed
            } else {
                &t.previous
            };
            generations
                .iter()
                .find(|g| {
                    Some(&g.id) == selected.as_ref()
                        && Some(&g.candidate.id) == input.candidate_id.as_ref()
                })
                .map(|g| g.candidate.clone())
                .ok_or_else(|| ApiError::conflict("The selected recovery version changed"))?
        }
        _ => return Err(ApiError::not_found()),
    };
    if candidate.channel != t.channel {
        return Err(ApiError::conflict(format!(
            "Select the {} channel before recovering this version",
            candidate.channel
        )));
    }
    let queued = storage::enqueue(&state.db, t, action, Some(candidate), true, Some(actor))
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    state.tools.wake.notify_one();
    changed(&state).await?;
    Ok(Json(json!({"job_id":queued})))
}
async fn changed(state: &AppState) -> anyhow::Result<()> {
    state.emit(None, "tools.changed", json!({})).await
}
pub(crate) async fn ready(state: &AppState) -> Result<OnlineSnapshot> {
    state.tools.runtime.online().map_err(|_|ApiError::conflict("Server tools are unavailable. An administrator can install or repair them in Settings → Server."))
}
pub(crate) async fn selection(state: &AppState) -> anyhow::Result<Option<OnlineSnapshot>> {
    Ok(state.tools.runtime.online().ok())
}
pub(crate) async fn verify(executable: &Executable) -> anyhow::Result<()> {
    executable.verify().await
}

async fn invoke(
    root: &std::path::Path,
    action: &str,
    args: &[String],
    input: Option<Value>,
    progress: Option<(&AppState, &Job)>,
) -> anyhow::Result<Value> {
    use process_wrap::tokio::*;
    use std::process::Stdio;
    let mut command = CommandWrap::with_new(PYTHON, |cmd| {
        cmd.args(["-I", "-B", "-u", "-c", PACKAGE, action])
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8");
    });
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    let mut child = command.spawn()?;
    if let Some(value) = input {
        child
            .stdin()
            .as_mut()
            .context("Package input missing")?
            .write_all(&serde_json::to_vec(&value)?)
            .await?;
    }
    child.stdin().take();
    let output = child.stdout().take().context("Package output missing")?;
    let mut output = BufReader::new(output);
    let operation = async {
        let mut result = None;
        loop {
            let mut line = Vec::new();
            let size = (&mut output)
                .take(2 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line)
                .await?;
            if size == 0 {
                break;
            }
            ensure!(
                size <= 2 * 1024 * 1024 && line.last() == Some(&b'\n'),
                "Package response exceeds its limit"
            );
            let value: Value = serde_json::from_slice(&line)?;
            if let Some(error) = value["error"].as_str() {
                return Err(PackageFailure {
                    message: error.into(),
                    retry_at: value["retry_at"].as_i64(),
                }
                .into());
            }
            if value.get("result").is_some() {
                result = Some(value["result"].clone());
            }
            if let Some((state, job)) = progress
                && let Some(stage) = value["stage"].as_str()
            {
                storage::progress(
                    &state.db,
                    job.id.clone(),
                    stage.into(),
                    value["received"].as_u64().unwrap_or(0),
                    value["total"].as_u64().unwrap_or(0),
                    None,
                    None,
                )
                .await?;
                changed(state).await?;
            }
        }
        ensure!(child.wait().await?.success(), "Package operation failed");
        result.context("Package operation returned no result")
    };
    tokio::time::timeout(Duration::from_secs(1200), operation)
        .await
        .context("Package operation timed out")?
}

pub async fn initialize(state: &AppState) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&state.tools.root).await?;
    storage::recover(&state.db).await?;
    if !supported() {
        state.tools.ready.send_replace(true);
        return Ok(());
    }
    state.tools.ready.send_replace(false);
    let (_, seeds) = seeds().await;
    let jobs = storage::jobs(&state.db).await?;
    for t in storage::inventory(&state.db).await? {
        if jobs
            .iter()
            .any(|j| j.tool == t.id && j.startup() && j.pending())
        {
            continue;
        }
        if t.installed.is_some() {
            storage::enqueue(&state.db, t, "validate".into(), None, true, None).await?;
        } else if let Some(generation) = seeds
            .iter()
            .find(|g| g.candidate.tool == t.id && g.candidate.channel == t.channel)
        {
            storage::enqueue(
                &state.db,
                t,
                "bootstrap".into(),
                Some(generation.candidate.clone()),
                true,
                None,
            )
            .await?;
        } else {
            storage::integrity(
                &state.db,
                t.id,
                Some("No verified image seed is available; check for updates to install".into()),
            )
            .await?;
        }
    }
    Ok(())
}

async fn seeds() -> (PathBuf, Vec<Generation>) {
    let root = std::env::var_os("THELXINOE_TOOLS_SEED")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/thelxinoe/tools"));
    let generations = tokio::fs::read(root.join("seed.json"))
        .await
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default();
    (root, generations)
}

async fn validate_current(state: &AppState, job: &Job, t: &Tool) -> anyhow::Result<()> {
    storage::progress(
        &state.db,
        job.id.clone(),
        "validating".into(),
        0,
        0,
        None,
        None,
    )
    .await?;
    changed(state).await?;
    let mut generation = storage::generations(&state.db)
        .await?
        .into_iter()
        .find(|g| Some(&g.id) == t.installed.as_ref())
        .context("Installed package is absent")?;
    let result = async {
        if t.id == "streamlink" {
            let abi = invoke(&state.tools.root, "runtime", &[], None, None).await?;
            if abi.as_str() != Some(&generation.python_abi) {
                let replacement: Generation = serde_json::from_value(
                    invoke(
                        &state.tools.root,
                        "rebuild",
                        &[state.tools.root.to_string_lossy().into()],
                        Some(json!(generation.candidate)),
                        Some((state, job)),
                    )
                    .await?,
                )?;
                storage::rebuild(&state.db, generation.id.clone(), replacement.clone()).await?;
                generation = replacement;
            }
        }
        verify_package(state, &generation).await
    }
    .await;
    storage::integrity(
        &state.db,
        t.id.clone(),
        result.as_ref().err().map(ToString::to_string),
    )
    .await?;
    if let Err(error) = result {
        let failed = tool(state, &t.id).await.map_err(|e| anyhow::anyhow!(e.2))?;
        if recover_previous(state, &failed).await.is_err() {
            return Err(error);
        }
    } else {
        state.tools.validated(std::slice::from_ref(&generation));
    }
    storage::progress(
        &state.db,
        job.id.clone(),
        "complete".into(),
        0,
        0,
        None,
        None,
    )
    .await?;
    refresh_invalid(state).await?;
    publish(state).await
}

async fn refresh_invalid(state: &AppState) -> anyhow::Result<()> {
    let generations = storage::generations(&state.db).await?;
    for tool in storage::inventory(&state.db).await? {
        if tool.integrity_error.is_some()
            && let Some(generation) = generations
                .iter()
                .find(|g| Some(&g.id) == tool.installed.as_ref())
            && verify_package(state, generation).await.is_ok()
        {
            // Repairing one member can also restore a dependent worker that
            // was unavailable solely because its companion failed validation.
            storage::integrity(&state.db, tool.id, None).await?;
            state.tools.validated(std::slice::from_ref(generation));
        }
    }
    Ok(())
}

async fn verify_package(state: &AppState, generation: &Generation) -> anyhow::Result<()> {
    invoke(
        &state.tools.root,
        "verify",
        &[state.tools.root.to_string_lossy().into()],
        Some(json!(generation)),
        None,
    )
    .await?;
    protocol(state, std::slice::from_ref(generation)).await
}

async fn recover_previous(state: &AppState, failed: &Tool) -> anyhow::Result<()> {
    let inventory = storage::inventory(&state.db).await?;
    let generations = storage::generations(&state.db).await?;
    let members = storage::recovery_group(&state.db, failed.clone()).await?;
    let mut tools = Vec::new();
    let mut proposed = Vec::new();
    for member in members {
        let tool = inventory
            .iter()
            .find(|t| Some(t.id.as_str()) == member["tool"].as_str())
            .context("Recovery member is unavailable")?;
        ensure!(
            !tool.pinned
                && tool.installed.as_deref() == member["after"].as_str()
                && tool.previous.as_deref() == member["before"].as_str(),
            "Recovery selection changed or is pinned"
        );
        let generation = generations
            .iter()
            .find(|g| Some(&g.id) == tool.previous.as_ref())
            .context("No previous working package")?;
        ensure!(
            generation.candidate.channel == tool.channel,
            "Recovery channel changed"
        );
        invoke(
            &state.tools.root,
            "verify",
            &[state.tools.root.to_string_lossy().into()],
            Some(json!(generation)),
            None,
        )
        .await?;
        proposed.push(generation.clone());
        tools.push(tool.clone());
    }
    protocol(state, &proposed).await?;
    ensure!(
        storage::restore_previous(
            &state.db,
            tools,
            format!(
                "Recovered previous version after local validation failed: {}",
                failed
                    .integrity_error
                    .as_deref()
                    .unwrap_or("Package validation failed")
            ),
        )
        .await?,
        "Recovery selection changed before activation"
    );
    state.tools.validated(&proposed);
    Ok(())
}

async fn recover_files(state: &AppState) -> anyhow::Result<()> {
    let mut temporary = tokio::fs::read_dir(&state.tools.root).await?;
    while let Some(entry) = temporary.next_entry().await? {
        if entry.file_name().to_string_lossy().starts_with(".run-")
            && entry.file_type().await?.is_dir()
        {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    let artifacts = state.tools.root.join("artifacts");
    if artifacts.exists() {
        let mut entries = tokio::fs::read_dir(artifacts).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name().to_string_lossy().ends_with(".part")
                && entry.file_type().await?.is_file()
            {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
    }
    let known = storage::generations(&state.db)
        .await?
        .into_iter()
        .map(|g| g.id)
        .collect::<HashSet<_>>();
    let directory = state.tools.root.join("packages");
    if !directory.exists() {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(&directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let staging = name.starts_with(".stage-") || name.starts_with(".seed-");
        let orphan = name.len() == 32
            && name.bytes().all(|b| b.is_ascii_hexdigit())
            && !known.contains(&name);
        if (staging || orphan) && entry.file_type().await?.is_dir() {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}
async fn publish(state: &AppState) -> anyhow::Result<()> {
    let selected = storage::inventory(&state.db)
        .await?
        .into_iter()
        .filter(|t| t.integrity_error.is_none())
        .filter_map(|t| t.installed.map(|id| (t.id, id)))
        .filter(|(_, id)| state.tools.validated.read().unwrap().contains(id))
        .collect::<BTreeMap<_, _>>();
    state
        .tools
        .runtime
        .publish(storage::generations(&state.db).await?, selected);
    crate::online::refresh_tools(state).await;
    Ok(())
}
pub(crate) async fn validate_installed(state: &AppState) -> anyhow::Result<()> {
    initialize(state).await?;
    if !supported() {
        return Ok(());
    }
    recover_files(state).await?;
    loop {
        let pending = storage::jobs(&state.db)
            .await?
            .into_iter()
            .filter(|j| j.startup() && j.pending())
            .collect::<Vec<_>>();
        if pending.is_empty() {
            break;
        }
        for job in &pending {
            operation(state, job).await?;
        }
        let next = storage::jobs(&state.db).await?;
        ensure!(
            pending.iter().any(|before| next
                .iter()
                .find(|after| after.id == before.id)
                .is_some_and(
                    |after| after.stage != before.stage || after.generation != before.generation
                )),
            "Tool preflight is waiting for an installation that cannot finish offline"
        );
    }
    for t in storage::inventory(&state.db).await? {
        ensure!(
            t.installed.is_some() && t.integrity_error.is_none(),
            "Tool preflight failed: {}",
            t.id
        );
    }
    Ok(())
}
async fn discover(state: &AppState, t: &Tool) -> anyhow::Result<Candidate> {
    if let Some(file) = std::env::var_os("THELXINOE_TOOLS_CATALOG") {
        let bytes = tokio::fs::read(file).await?;
        ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "Offline tool catalog exceeds its limit"
        );
        let catalog: Vec<Candidate> = serde_json::from_slice(&bytes)?;
        return catalog
            .into_iter()
            .find(|c| c.tool == t.id && c.channel == t.channel)
            .context("No matching release in the offline tool catalog");
    }
    serde_json::from_value(
        invoke(
            &state.tools.root,
            "discover",
            &[t.id.clone(), t.channel.clone()],
            None,
            None,
        )
        .await?,
    )
    .map_err(Into::into)
}
async fn operation(state: &AppState, job: &Job) -> anyhow::Result<()> {
    if job.retry_at > now() && job.action != "check" {
        return Ok(());
    }
    let t = tool(state, &job.tool)
        .await
        .map_err(|e| anyhow::anyhow!(e.2))?;
    if job.action == "validate" {
        return validate_current(state, job, &t).await;
    }
    if job.action == "check" {
        storage::progress(
            &state.db,
            job.id.clone(),
            "checking".into(),
            0,
            0,
            None,
            None,
        )
        .await?;
        changed(state).await?;
        match discover(state, &t).await {
            Ok(candidate) => storage::discover(&state.db, t, Some(candidate), None, None).await?,
            Err(error) => {
                let retry = error
                    .downcast_ref::<PackageFailure>()
                    .and_then(|e| e.retry_at);
                storage::discover(&state.db, t, None, Some(error.to_string()), retry).await?;
                return Err(error);
            }
        }
        storage::progress(
            &state.db,
            job.id.clone(),
            "complete".into(),
            0,
            0,
            None,
            None,
        )
        .await?;
        return Ok(());
    }
    let candidate = job
        .candidate
        .as_ref()
        .context("Tool operation has no frozen candidate")?;
    if t.channel != candidate.channel {
        storage::progress(
            &state.db,
            job.id.clone(),
            "canceled".into(),
            0,
            0,
            Some("Channel changed".into()),
            None,
        )
        .await?;
        return Ok(());
    }
    if job.generation.is_none() {
        let generation = if job.action == "rollback" {
            storage::generations(&state.db)
                .await?
                .into_iter()
                .find(|g| Some(&g.id) == t.previous.as_ref() && g.candidate.id == candidate.id)
                .context("Previous package changed")?
        } else if job.action == "bootstrap" {
            let (seed, seeds) = seeds().await;
            let generation = seeds
                .iter()
                .find(|g| &g.candidate == candidate)
                .context("The bundled package changed; check for updates to install")?;
            storage::progress(
                &state.db,
                job.id.clone(),
                "installing".into(),
                0,
                0,
                None,
                None,
            )
            .await?;
            changed(state).await?;
            serde_json::from_value(
                invoke(
                    &state.tools.root,
                    "bootstrap",
                    &[
                        state.tools.root.to_string_lossy().into(),
                        seed.to_string_lossy().into(),
                    ],
                    Some(json!(generation)),
                    Some((state, job)),
                )
                .await?,
            )?
        } else {
            serde_json::from_value(
                invoke(
                    &state.tools.root,
                    "prepare",
                    &[state.tools.root.to_string_lossy().into()],
                    Some(json!(candidate)),
                    Some((state, job)),
                )
                .await?,
            )?
        };
        storage::prepared(&state.db, job.id.clone(), generation).await?;
        state.tools.wake.notify_one();
        return Ok(());
    }
    activate(state, job, &t).await
}
async fn wait_reason(state: &AppState, job: &Job, t: &Tool) -> anyhow::Result<Option<String>> {
    let policy = crate::product::configured_policy(state)
        .await
        .map_err(|e| anyhow::anyhow!(e.2))?;
    let effective = if t.policy == "inherit" {
        &policy.policy
    } else {
        &t.policy
    };
    let reason = if t.pinned {
        Some("Pinned")
    } else if !job.manual && effective != "automatic" {
        Some("Waiting for Automatic policy or a manual install")
    } else if !job.manual && t.held.as_ref() == job.candidate.as_ref().map(|c| &c.id) {
        Some("Held after rollback")
    } else if !job.manual
        && !crate::timezones::in_server_window(
            state,
            u32::from(policy.window_start),
            u32::from(policy.window_end),
        )
        .await?
    {
        Some("Waiting for maintenance window")
    } else if storage::busy(&state.db, t.id.clone()).await?
        || t.id == "ffmpeg"
            && t.installed
                .as_ref()
                .is_some_and(|id| state.tools.runtime.referenced().contains(id))
        || state.online.extraction.available_permits() < 2
            && ["yt-dlp", "deno", "streamlink"].contains(&t.id.as_str())
    {
        Some("Waiting for active playback, downloads or extraction")
    } else {
        None
    };
    Ok(reason.map(str::to_owned))
}

async fn wait(state: &AppState, job: &Job, reason: String) -> anyhow::Result<()> {
    storage::progress(
        &state.db,
        job.id.clone(),
        "waiting".into(),
        job.received,
        job.total,
        Some(reason),
        None,
    )
    .await
}

async fn protocol(state: &AppState, proposed: &[Generation]) -> anyhow::Result<()> {
    let selected = storage::inventory(&state.db).await?;
    let generations = storage::generations(&state.db).await?;
    let package = |id: &str| {
        proposed
            .iter()
            .find(|g| g.candidate.tool == id)
            .or_else(|| {
                selected.iter().find(|t| t.id == id).and_then(|t| {
                    generations
                        .iter()
                        .find(|g| Some(&g.id) == t.installed.as_ref())
                })
            })
    };
    let input = if proposed.iter().any(|g| g.candidate.tool == "streamlink") {
        json!({"kind":"streamlink","streamlink":package("streamlink").context("Streamlink is unavailable")?,"script":include_str!("../online/streamlink_worker.py")})
    } else if proposed
        .iter()
        .any(|g| ["yt-dlp", "deno"].contains(&g.candidate.tool.as_str()))
    {
        json!({"kind":"youtube","yt_dlp":package("yt-dlp").context("Install yt-dlp to validate the YouTube runtime")?,"deno":package("deno").context("Install Deno to validate the YouTube runtime")?,"script":include_str!("../online/youtube_worker.py")})
    } else {
        return Ok(());
    };
    invoke(
        &state.tools.root,
        "protocol",
        &[state.tools.root.to_string_lossy().into()],
        Some(input),
        None,
    )
    .await?;
    Ok(())
}

async fn activate(state: &AppState, job: &Job, t: &Tool) -> anyhow::Result<()> {
    if let Some(reason) = wait_reason(state, job, t).await? {
        return wait(state, job, reason).await;
    }
    if job.retry_at > now() {
        return Ok(());
    }
    let generations = storage::generations(&state.db).await?;
    let generation = generations
        .iter()
        .find(|g| Some(&g.id) == job.generation.as_ref())
        .context("Prepared generation is absent")?
        .clone();
    let mut proposed = vec![generation];
    let mut selections = vec![(job.clone(), t.clone(), job.generation.clone().unwrap())];
    if let Some(peer) = match t.id.as_str() {
        "yt-dlp" => Some("deno"),
        "deno" => Some("yt-dlp"),
        _ => None,
    } && let Some(other) = storage::jobs(&state.db).await?.into_iter().find(|j| {
        j.tool == peer && j.pending() && !["check", "validate"].contains(&j.action.as_str())
    }) {
        let settings = tool(state, peer).await.map_err(|e| anyhow::anyhow!(e.2))?;
        if let Some(prepared) = generations
            .iter()
            .find(|g| Some(&g.id) == other.generation.as_ref())
        {
            if let Some(reason) = wait_reason(state, &other, &settings).await? {
                if job.action == "rollback" && other.action == "rollback"
                    || settings.installed.is_none()
                {
                    return wait(state, job, format!("Dependency {peer}: {reason}")).await;
                }
            } else {
                proposed.push(prepared.clone());
                selections.push((other, settings, prepared.id.clone()));
            }
        } else if job.action == "rollback" && other.action == "rollback"
            || settings.installed.is_none()
        {
            return wait(state, job, format!("Waiting for {peer} preparation")).await;
        }
    }
    for generation in &proposed {
        invoke(
            &state.tools.root,
            "verify",
            &[state.tools.root.to_string_lossy().into()],
            Some(json!(generation)),
            None,
        )
        .await?;
    }
    if let Err(error) = protocol(state, &proposed).await {
        storage::validation(
            &state.db,
            job.id.clone(),
            json!({"local_error":error.to_string()}),
            now() + 3600,
        )
        .await?;
        return wait(state, job, format!("Compatible runtime required: {error}")).await;
    }
    if !qualify(state, job, &proposed).await? {
        return Ok(());
    }
    // Recheck activity after validation; automatic policy/window is also checked
    // in the writer transaction, so concurrent preference saves always win.
    for (pending, settings, _) in &selections {
        if let Some(reason) = wait_reason(state, pending, settings).await? {
            return wait(state, job, reason).await;
        }
    }
    if storage::activate(&state.db, selections).await? {
        state.tools.validated(&proposed);
        publish(state).await?;
        // Detect only reproducible local failures across the publication boundary.
        // Remote-provider failures never enter this rollback path.
        for generation in &proposed {
            if let Err(error) = verify_package(state, generation).await {
                storage::integrity(
                    &state.db,
                    generation.candidate.tool.clone(),
                    Some(error.to_string()),
                )
                .await?;
                let failed = tool(state, &generation.candidate.tool)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.2))?;
                if let Err(recovery) = recover_previous(state, &failed).await {
                    tracing::warn!(%recovery,"Tool recovery requires administrator action");
                }
                publish(state).await?;
                break;
            }
        }
        refresh_invalid(state).await?;
        publish(state).await?;
    }
    Ok(())
}

async fn qualify(state: &AppState, job: &Job, proposed: &[Generation]) -> anyhow::Result<bool> {
    let selected = storage::inventory(&state.db).await?;
    let generations = storage::generations(&state.db).await?;
    let current = |id: &str| {
        selected.iter().find(|t| t.id == id).and_then(|t| {
            generations
                .iter()
                .find(|g| Some(&g.id) == t.installed.as_ref())
        })
    };
    let bundled = if job.action == "bootstrap" {
        let (_, seeds) = seeds().await;
        proposed.iter().all(|generation| {
            seeds
                .iter()
                .any(|seed| seed.candidate == generation.candidate)
        })
    } else {
        false
    };
    // Restoring the exact already-qualified artifacts remains possible offline.
    // A changed wheel graph or binary always gets fresh provider qualification.
    if proposed.iter().all(|g| {
        g.candidate.tool == "ffmpeg"
            || current(&g.candidate.tool)
                .is_some_and(|old| old.candidate.artifacts == g.candidate.artifacts)
    }) || job.action == "rollback"
        || bundled
    {
        return Ok(true);
    }
    let package = |id: &str| {
        proposed
            .iter()
            .find(|g| g.candidate.tool == id)
            .or_else(|| current(id))
    };
    let kind = if proposed.iter().any(|g| g.candidate.tool == "streamlink") {
        "streamlink"
    } else {
        "youtube"
    };
    let input = json!({"kind":kind,"candidate":{"yt-dlp":package("yt-dlp"),"deno":package("deno"),"streamlink":package("streamlink")},"current":{"yt-dlp":current("yt-dlp"),"deno":current("deno"),"streamlink":current("streamlink")},"youtube_script":include_str!("../online/youtube_worker.py"),"streamlink_script":include_str!("../online/streamlink_worker.py")});
    storage::progress(
        &state.db,
        job.id.clone(),
        "validating".into(),
        job.received,
        job.total,
        Some("Checking public extraction".into()),
        None,
    )
    .await?;
    changed(state).await?;
    let result = invoke(
        &state.tools.root,
        "probe",
        &[state.tools.root.to_string_lossy().into()],
        Some(input),
        None,
    )
    .await?;
    let ready = result["ready"].as_bool().unwrap_or(false);
    storage::validation(
        &state.db,
        job.id.clone(),
        result,
        if ready { 0 } else { now() + 3600 },
    )
    .await?;
    wait(state,job,if ready{"Provider validation complete"}else{"Public extraction is inconclusive; keeping the current version and retrying in one hour"}.into()).await?;
    Ok(ready)
}
async fn schedule(state: &AppState, startup: bool) -> anyhow::Result<()> {
    let jobs = storage::jobs(&state.db).await?;
    let generations = storage::generations(&state.db).await?;
    let policy = crate::product::configured_policy(state)
        .await
        .map_err(|e| anyhow::anyhow!(e.2))?;
    for t in storage::inventory(&state.db).await? {
        if (startup && t.check_error.is_none() || t.next_check_at <= now())
            && !jobs
                .iter()
                .any(|j| j.tool == t.id && j.action == "check" && j.pending())
        {
            storage::enqueue(&state.db, t.clone(), "check".into(), None, false, None).await?;
            continue;
        }
        if jobs.iter().any(|j| j.tool == t.id && j.pending()) {
            continue;
        }
        let automatic =
            t.policy == "automatic" || t.policy == "inherit" && policy.policy == "automatic";
        if automatic
            && !t.pinned
            && t.check_error.is_none()
            && let Some(candidate) = t.candidate.clone()
        {
            let current = generations
                .iter()
                .find(|g| Some(&g.id) == t.installed.as_ref());
            if current.is_none_or(|g| g.candidate.id != candidate.id)
                && t.held.as_ref() != Some(&candidate.id)
                && !jobs.iter().any(|j| {
                    j.tool == t.id
                        && j.stage == "failed"
                        && j.candidate.as_ref().is_some_and(|c| c.id == candidate.id)
                })
            {
                storage::enqueue(&state.db, t, "install".into(), Some(candidate), false, None)
                    .await?;
            }
        }
    }
    Ok(())
}
async fn collect(state: &AppState) -> anyhow::Result<()> {
    let retained = storage::retained(&state.db)
        .await?
        .into_iter()
        .chain(state.tools.runtime.referenced())
        .collect::<HashSet<_>>();
    let generations = storage::generations(&state.db).await?;
    let mut artifacts = HashSet::new();
    for generation in &generations {
        if retained.contains(&generation.id) {
            for artifact in &generation.candidate.artifacts {
                if let Some(digest) = artifact["sha256"].as_str() {
                    artifacts.insert(digest.to_owned());
                }
            }
        }
    }
    for job in storage::jobs(&state.db)
        .await?
        .into_iter()
        .filter(Job::pending)
    {
        if let Some(candidate) = job.candidate {
            for artifact in candidate.artifacts {
                if let Some(digest) = artifact["sha256"].as_str() {
                    artifacts.insert(digest.to_owned());
                }
            }
        }
    }
    // Keep downloaded inputs for the currently advertised candidate as well,
    // so an explicit retry does not need another network transfer.
    for tool in storage::inventory(&state.db).await? {
        if let Some(candidate) = tool.candidate {
            for artifact in candidate.artifacts {
                if let Some(digest) = artifact["sha256"].as_str() {
                    artifacts.insert(digest.to_owned());
                }
            }
        }
    }
    for generation in generations {
        if !retained.contains(&generation.id) && state.tools.runtime.forget(&generation.id) {
            // Forget closes admission to new in-memory leases. Recheck durable
            // references afterward: a download may have persisted its snapshot
            // and released its last lease since the initial retention read.
            if storage::retained(&state.db).await?.contains(&generation.id) {
                for artifact in &generation.candidate.artifacts {
                    if let Some(digest) = artifact["sha256"].as_str() {
                        artifacts.insert(digest.to_owned());
                    }
                }
                publish(state).await?;
                continue;
            }
            let path = state.tools.root.join("packages").join(&generation.id);
            ensure!(
                generation.id.len() == 32 && generation.id.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid package directory"
            );
            if path.exists() {
                tokio::fs::remove_dir_all(path).await?;
            }
            storage::remove_generation(&state.db, generation.id).await?;
        }
    }
    let directory = state.tools.root.join("artifacts");
    if directory.exists() {
        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().await?.is_file()
                && name.len() == 64
                && name.bytes().all(|b| b.is_ascii_hexdigit())
                && !artifacts.contains(&name)
            {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
    }
    Ok(())
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    if !supported() {
        std::future::pending::<()>().await;
    }
    {
        let _gate = state.release_gate.read().await;
        recover_files(&state).await?;
    }
    let mut startup = true;
    loop {
        let _gate = state.release_gate.read().await;
        if !state.release_quiescing.load(Ordering::SeqCst) {
            let preparing = storage::jobs(&state.db)
                .await?
                .iter()
                .any(|j| j.startup() && j.pending());
            if !preparing {
                state.tools.ready.send_replace(true);
                if let Err(error) = schedule(&state, startup).await {
                    tracing::warn!(%error,"Tool scheduling will retry");
                }
                startup = false;
            }
            // Separate bounded lane: transfers cannot block scans or general jobs.
            let pending = storage::jobs(&state.db)
                .await?
                .into_iter()
                .filter(Job::pending)
                .filter(|j| !preparing || j.startup() || j.manual)
                .collect::<Vec<_>>();
            for job in pending {
                if !storage::jobs(&state.db)
                    .await?
                    .iter()
                    .any(|current| current.id == job.id && current.pending())
                {
                    continue;
                }
                if let Err(error) = operation(&state, &job).await {
                    if job.startup() {
                        storage::integrity(&state.db, job.tool.clone(), Some(error.to_string()))
                            .await?;
                    }
                    if job.action != "check"
                        && let Some(retry_at) = error
                            .downcast_ref::<PackageFailure>()
                            .and_then(|e| e.retry_at)
                    {
                        storage::validation(
                            &state.db,
                            job.id.clone(),
                            json!({"transfer_error":error.to_string()}),
                            retry_at,
                        )
                        .await?;
                        wait(&state,&job,"Release service unavailable; the frozen candidate will retry automatically".into()).await?;
                        changed(&state).await?;
                        continue;
                    }
                    storage::progress(
                        &state.db,
                        job.id.clone(),
                        "failed".into(),
                        job.received,
                        job.total,
                        None,
                        Some(error.to_string()),
                    )
                    .await?;
                }
                changed(&state).await?;
            }
            if preparing
                && !storage::jobs(&state.db)
                    .await?
                    .iter()
                    .any(|j| j.startup() && j.pending())
            {
                state.tools.ready.send_replace(true);
                state.tools.wake.notify_one();
            }
            if let Err(error) = collect(&state).await {
                tracing::warn!(%error,"Tool retention will retry");
            }
        }
        drop(_gate);
        tokio::select! {_=state.tools.wake.notified()=>{},_=tokio::time::sleep(Duration::from_secs(30))=>{}}
    }
}
