//! Publisher discovery and installer trust are independent of the connected server.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use thelxinoe_releases::Manifest;

#[derive(Default)]
pub struct Runtime {
    operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    status: std::sync::Mutex<Value>,
    prepared: tokio::sync::Mutex<Option<Prepared>>,
    backoff: std::sync::Mutex<Backoff>,
}
struct Prepared {
    release: Manifest,
    update: Update,
    bytes: Vec<u8>,
}
/// Automatic attempts back off after failures, so a blocked installer launch or a
/// broken artifact is not retried (and downloaded again) every few minutes.
#[derive(Default)]
struct Backoff {
    version: String,
    failures: u32,
    next: i64,
}
impl Backoff {
    fn allows(&self, version: &str, now: i64) -> bool {
        self.version != version || now >= self.next
    }
    fn failed(&mut self, version: &str, now: i64) {
        if self.version != version {
            *self = Self {
                version: version.into(),
                ..Self::default()
            };
        }
        self.failures = self.failures.saturating_add(1);
        self.next = now + (300i64 << (self.failures - 1).min(7)).min(6 * 3600);
    }
}
const PUBLIC_KEY: &str = match option_env!("THELXINOE_RELEASE_PUBLIC_KEY") {
    Some(key) => key,
    None => include_str!("../../../releases/release.pub"),
};
const CHANNEL: &str = match option_env!("THELXINOE_RELEASE_URL") {
    Some(url) => url,
    None => thelxinoe_releases::DEFAULT_CHANNEL,
};
const CA: Option<&str> = option_env!("THELXINOE_RELEASE_CA_PEM");

fn policy_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("update-policy"))
}
fn change(app: &tauri::AppHandle, patch: Value) -> Value {
    let runtime = app.state::<Runtime>();
    let mut value = runtime.status.lock().expect("desktop update status");
    if !value.is_object() {
        let policy = policy_path(app)
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .filter(|v| v == "automatic" || v == "notify")
            .unwrap_or_else(|| "automatic".into());
        *value = json!({"installed":app.package_info().version.to_string(),"policy":policy,"phase":"idle","checked_at":null,"release":null,"hold":null,"error":null,"received":0,"total":0});
    }
    for (key, field) in patch.as_object().into_iter().flatten() {
        value[key] = field.clone();
    }
    let status = value.clone();
    drop(value);
    let _ = app.emit("desktop-update-changed", &status);
    status
}
async fn candidate(app: &tauri::AppHandle) -> Result<Option<Manifest>, String> {
    let (_, manifest) = thelxinoe_releases::fetch(CHANNEL, PUBLIC_KEY, CA)
        .await
        .map_err(|_| "Could not check the signed desktop release channel")?;
    manifest
        .valid_at(thelxinoe_core::now())
        .map_err(|e| e.to_string())?;
    let next = semver::Version::parse(&manifest.version).map_err(|_| "Invalid release version")?;
    Ok((next > app.package_info().version).then_some(manifest))
}
fn summary(release: &Manifest) -> Value {
    json!({"version":release.version,"notes":release.notes,"bytes":release.windows_x64.bytes,"api":release.api.max})
}
/// A release's desktop speaks its `api.max`; a server accepts clients within its
/// `api_min..=api_max` (older servers report only `api_version`). An offline or
/// unknown server must not prevent maintaining this desktop.
fn server_hold(health: &Value, version: &str, api: u32) -> Option<String> {
    if health["status"] != 200 {
        return None;
    }
    let body = &health["body"];
    let current = body["api_version"].as_u64()?;
    let accepted =
        body["api_min"].as_u64().unwrap_or(current)..=body["api_max"].as_u64().unwrap_or(current);
    if accepted.contains(&u64::from(api)) {
        return None;
    }
    let behind = match (
        body["version"].as_str().map(semver::Version::parse),
        semver::Version::parse(version),
    ) {
        (Some(Ok(server)), Ok(release)) => server < release,
        _ => true,
    };
    Some(if behind {
        format!("Update the server to {version} first")
    } else {
        format!("The connected server does not support desktop {version}")
    })
}
async fn held(app: &tauri::AppHandle, version: &str, api: u32) -> Option<String> {
    let health = crate::backend_request(app.clone(), "/health".into(), "GET".into(), None)
        .await
        .ok()?;
    server_hold(&health, version, api)
}
fn finish(
    app: &tauri::AppHandle,
    result: Result<(), String>,
    phase: &str,
) -> Result<Value, String> {
    match result {
        Ok(()) => Ok(change(app, json!({"phase":phase,"error":null}))),
        Err(error) => {
            change(app, json!({"phase":"idle","error":error}));
            Err(error)
        }
    }
}
#[tauri::command]
pub fn desktop_update_status(app: tauri::AppHandle) -> Value {
    change(&app, json!({}))
}
#[tauri::command]
pub async fn desktop_update_check(app: tauri::AppHandle) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let _guard = runtime
        .operation
        .try_lock()
        .map_err(|_| "A desktop update operation is running")?;
    change(&app, json!({"phase":"checking","error":null}));
    let result = candidate(&app).await;
    change(&app, json!({"checked_at":thelxinoe_core::now()}));
    match result {
        Ok(release) => {
            let mut prepared = runtime.prepared.lock().await;
            if prepared.as_ref().is_some_and(|p| {
                release.as_ref().is_none_or(|r| {
                    r.version != p.release.version
                        || r.windows_x64.sha256 != p.release.windows_x64.sha256
                })
            }) {
                *prepared = None;
            }
            let phase = if prepared.is_some() { "ready" } else { "idle" };
            drop(prepared);
            let hold = match &release {
                Some(release) => held(&app, &release.version, release.api.max).await,
                None => None,
            };
            Ok(change(
                &app,
                json!({"phase":phase,"release":release.as_ref().map(summary),"hold":hold,"error":null}),
            ))
        }
        Err(error) => {
            // A failed check says nothing about a release already downloaded and verified:
            // keep it for this session and only report the error.
            let phase = if runtime.prepared.lock().await.is_some() {
                "ready"
            } else {
                "idle"
            };
            change(&app, json!({"phase":phase,"error":error}));
            Err(error)
        }
    }
}
async fn download(app: &tauri::AppHandle, release: Manifest) -> Result<(), String> {
    let endpoint = thelxinoe_releases::https(&release.windows_x64.url)
        .and_then(|url| Ok(url.join("windows-x64.json")?))
        .map_err(|_| "Invalid desktop release endpoint")?;
    // The installer must verify against the updater key built into this desktop. The manifest
    // names its key only so a release for a different key is refused up front.
    let trusted = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater["pubkey"].as_str())
        .map(str::trim)
        .unwrap_or_default();
    if trusted.is_empty() || trusted != release.windows_x64.updater_public_key.trim() {
        return Err("This release is signed for a different desktop updater key".into());
    }
    let mut builder = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|_| "Invalid desktop release endpoint")?
        .timeout(std::time::Duration::from_secs(1800));
    let certificate = CA
        .map(|pem| reqwest::Certificate::from_pem(pem.as_bytes()))
        .transpose()
        .map_err(|_| "Invalid publisher TLS certificate")?;
    builder = builder.configure_client(move |client| {
        let client = client
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5));
        if let Some(certificate) = &certificate {
            client.add_root_certificate(certificate.clone())
        } else {
            client
        }
    });
    let update = builder
        .build()
        .map_err(|_| "Could not initialize desktop updates")?
        .check()
        .await
        .map_err(|_| "Desktop release metadata is unavailable")?
        .ok_or("Desktop artifact is unavailable")?;
    if update.version != release.version
        || update.download_url.as_str() != release.windows_x64.url
        || update.signature != release.windows_x64.signature
    {
        return Err("Desktop metadata does not match the signed product release".into());
    }
    let limit = release.windows_x64.bytes;
    let overflow = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = overflow.clone();
    let mut received = 0u64;
    let handle = app.clone();
    change(app, json!({"release":summary(&release),"total":limit}));
    let bytes = tokio::select! {
        result=update.download(move|chunk,total| {
            received=received.saturating_add(chunk as u64);
            if received>limit || total.is_some_and(|n|n!=limit) { signal.notify_one(); }
            change(&handle,json!({"received":received}));
        },||{}) => result.map_err(|_|"Desktop download or signature verification failed")?,
        _=overflow.notified() => return Err("Desktop artifact exceeds its signed size".into()),
    };
    let hash = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if bytes.len() as u64 != limit || hash != release.windows_x64.sha256 {
        return Err("Desktop artifact hash does not match the signed release".into());
    }
    *app.state::<Runtime>().prepared.lock().await = Some(Prepared {
        release,
        update,
        bytes,
    });
    Ok(())
}
#[tauri::command]
pub async fn desktop_update_download(app: tauri::AppHandle) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let _guard = runtime
        .operation
        .try_lock()
        .map_err(|_| "A desktop update operation is running")?;
    runtime.prepared.lock().await.take();
    change(
        &app,
        json!({"phase":"downloading","error":null,"received":0,"total":0}),
    );
    let result = async {
        let release = candidate(&app)
            .await?
            .ok_or("No newer signed desktop release is available")?;
        download(&app, release).await
    }
    .await;
    finish(&app, result, "ready")
}
#[tauri::command]
pub async fn desktop_update_install(app: tauri::AppHandle) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let _guard = runtime
        .operation
        .try_lock()
        .map_err(|_| "A desktop update operation is running")?;
    install(&app, false).await
}
#[cfg(windows)]
async fn playback_active(app: &tauri::AppHandle) -> bool {
    app.state::<crate::mpv::DesktopPlayback>()
        .player
        .active()
        .await
}
async fn install(app: &tauri::AppHandle, wait_for_playback: bool) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let result = async {
        #[cfg(windows)]
        let playback = app.state::<crate::mpv::DesktopPlayback>();
        // Also waits for an accepted play request still resolving tools. New
        // playback cannot start after the idle check and before installer exit.
        #[cfg(windows)]
        let _playback_guard = loop {
            let guard = playback.player.restart_guard().await;
            if !playback_active(app).await {
                break guard;
            }
            if !wait_for_playback {
                return Err("Stop desktop playback before restarting to install the update".into());
            }
            drop(guard);
            change(app, json!({"phase":"waiting"}));
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        };
        #[cfg(not(windows))]
        let _ = wait_for_playback;
        let prepared = runtime.prepared.lock().await;
        let pending = prepared
            .as_ref()
            .ok_or("Download and verify the desktop update first")?;
        pending
            .release
            .valid_at(thelxinoe_core::now())
            .map_err(|e| e.to_string())?;
        if let Some(hold) = held(app, &pending.release.version, pending.release.api.max).await {
            return Ok(Some(hold));
        }
        // The updater bypasses CloseRequested. Drain package/configuration writes
        // and keep their locks until installation starts or fails.
        let tools = app.state::<crate::tools::DesktopTools>();
        let _tools_guard = tools.restart_guard().await;
        app.save_window_state(StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED)
            .map_err(|_| "Could not save desktop window state before restarting")?;
        change(app, json!({"phase":"installing","error":null}));
        // The updater hides every window before starting the installer. Keep the
        // verified download and show them again if Windows refuses to start it.
        let visible = app
            .webview_windows()
            .into_values()
            .filter(|window| window.is_visible().unwrap_or(false))
            .collect::<Vec<_>>();
        pending.update.install(&pending.bytes).map_err(|_| {
            for window in &visible {
                let _ = window.show();
            }
            "Windows could not start the signed desktop installer".to_string()
        })?;
        Ok::<_, String>(None)
    }
    .await;
    let phase = if runtime.prepared.lock().await.is_some() {
        "ready"
    } else {
        "idle"
    };
    match result {
        Ok(None) => Ok(change(app, json!({"phase":"installing"}))),
        // Held, not failed: the status shows what the server needs instead of an error.
        Ok(Some(hold)) => {
            change(app, json!({"phase":phase,"hold":hold}));
            Err(hold)
        }
        Err(error) => {
            change(app, json!({"phase":phase,"error":error}));
            Err(error)
        }
    }
}
#[tauri::command]
pub async fn desktop_update_apply(
    app: tauri::AppHandle,
    version: Option<String>,
) -> Result<Value, String> {
    let guard = app
        .state::<Runtime>()
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "A desktop update operation is running")?;
    let status = change(&app, json!({"phase":"checking","error":null}));
    // Keep the accepted operation alive when settings closes or the webview reloads.
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let result = apply(&app, version).await;
        if let Err(error) = result {
            let runtime = app.state::<Runtime>();
            if let Some(version) = change(&app, json!({}))["release"]["version"].as_str() {
                runtime
                    .backoff
                    .lock()
                    .expect("desktop update backoff")
                    .failed(version, thelxinoe_core::now());
            }
            // A verified download survives a failed installer launch.
            let phase = if runtime.prepared.lock().await.is_some() {
                "ready"
            } else {
                "idle"
            };
            change(&app, json!({"phase":phase,"error":error}));
        }
    });
    Ok(status)
}
async fn apply(app: &tauri::AppHandle, expected: Option<String>) -> Result<(), String> {
    let release = candidate(app).await?;
    change(app, json!({"checked_at":thelxinoe_core::now()}));
    let Some(release) = release else {
        app.state::<Runtime>().prepared.lock().await.take();
        change(app, json!({"phase":"idle","release":null,"hold":null}));
        return Ok(());
    };
    if expected.as_ref().is_some_and(|v| *v != release.version) {
        change(app, json!({"release":summary(&release)}));
        return Err("The available version changed. Click to install the new release".into());
    }
    let ready = app
        .state::<Runtime>()
        .prepared
        .lock()
        .await
        .as_ref()
        .is_some_and(|p| {
            p.release.version == release.version
                && p.release.windows_x64.sha256 == release.windows_x64.sha256
        });
    if !ready {
        app.state::<Runtime>().prepared.lock().await.take();
    }
    // Hold the update until the reachable server accepts it; the loop in `start`
    // installs it automatically once it does.
    let hold = held(app, &release.version, release.api.max).await;
    change(app, json!({"release":summary(&release),"hold":hold}));
    if hold.is_some() {
        change(app, json!({"phase":if ready { "ready" } else { "idle" }}));
        return Ok(());
    }
    if !ready {
        change(
            app,
            json!({"phase":"downloading","received":0,"total":release.windows_x64.bytes}),
        );
        download(app, release).await?;
    }
    install(app, true).await?;
    Ok(())
}
#[tauri::command]
pub fn desktop_update_policy(app: tauri::AppHandle, policy: String) -> Result<Value, String> {
    if !["notify", "automatic"].contains(&policy.as_str()) {
        return Err("Invalid desktop update policy".into());
    }
    std::fs::write(policy_path(&app)?, &policy).map_err(|e| e.to_string())?;
    Ok(change(&app, json!({"policy":policy})))
}
fn automatic_candidate(status: &Value) -> bool {
    status["policy"] == "automatic"
        && matches!(status["phase"].as_str(), Some("idle" | "ready"))
        && status["error"].is_null()
        && status["hold"].is_null()
        && !status["release"].is_null()
}
/// Re-evaluates a held release against the server, so it installs (Automatic) or
/// becomes installable (Notify) without a new publisher check once the server accepts it.
async fn recheck_hold(app: &tauri::AppHandle) {
    let status = change(app, json!({}));
    let (Some(version), Some(api)) = (
        status["release"]["version"].as_str(),
        status["release"]["api"]
            .as_u64()
            .and_then(|api| u32::try_from(api).ok()),
    ) else {
        return;
    };
    if status["hold"].is_null() {
        return;
    }
    let hold = json!(held(app, version, api).await);
    // Keep the result of any update operation that ran meanwhile.
    let runtime = app.state::<Runtime>();
    if let Ok(_guard) = runtime.operation.try_lock()
        && change(app, json!({}))["hold"] == status["hold"]
        && hold != status["hold"]
    {
        change(app, json!({"hold":hold}));
    }
}
pub fn start(app: tauri::AppHandle) {
    change(&app, json!({}));
    tauri::async_runtime::spawn(async move {
        loop {
            let status = desktop_update_status(app.clone());
            let interval = if status["error"].is_null() {
                6 * 3600
            } else {
                300
            };
            if status["checked_at"].as_i64().unwrap_or(0) + interval <= thelxinoe_core::now() {
                let _ = desktop_update_check(app.clone()).await;
            }
            recheck_hold(&app).await;
            let status = desktop_update_status(app.clone());
            let version = status["release"]["version"].as_str().map(str::to_owned);
            let due = version.as_deref().is_some_and(|version| {
                app.state::<Runtime>()
                    .backoff
                    .lock()
                    .expect("desktop update backoff")
                    .allows(version, thelxinoe_core::now())
            });
            // Personal update preferences apply to a signed-in desktop, and to one the
            // server rejects as incompatible (HTTP 426): updating is then the only way in.
            if automatic_candidate(&status)
                && due
                && let Ok(session) =
                    crate::backend_request(app.clone(), "/auth/me".into(), "GET".into(), None).await
                && matches!(session["status"].as_u64(), Some(200 | 426))
            {
                let _ = desktop_update_apply(app.clone(), version).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_resumes_a_verified_download_but_preserves_error_and_policy_guards() {
        let mut status = json!({"policy":"automatic","phase":"ready","error":null,"release":{"version":"1.0.0"}});
        assert!(automatic_candidate(&status));
        status["phase"] = json!("idle");
        assert!(automatic_candidate(&status));
        for phase in ["checking", "downloading", "waiting", "installing"] {
            status["phase"] = json!(phase);
            assert!(!automatic_candidate(&status));
        }
        status["phase"] = json!("ready");
        status["error"] = json!("Incompatible server");
        assert!(!automatic_candidate(&status));
        status["error"] = Value::Null;
        status["policy"] = json!("notify");
        assert!(!automatic_candidate(&status));
        status["policy"] = json!("automatic");
        status["release"] = Value::Null;
        assert!(!automatic_candidate(&status));
        status["release"] = json!({"version":"1.0.0"});
        status["hold"] = json!("Update the server to 1.0.0 first");
        assert!(!automatic_candidate(&status));
    }
    #[test]
    fn a_reachable_server_holds_only_releases_outside_its_accepted_range() {
        let health = |body: Value| json!({"status":200,"body":body});
        let server = health(json!({"version":"0.0.4","api_version":3,"api_min":2,"api_max":3}));
        assert_eq!(server_hold(&server, "0.0.5", 2), None);
        assert_eq!(server_hold(&server, "0.0.5", 3), None);
        assert_eq!(
            server_hold(&server, "0.0.5", 4).as_deref(),
            Some("Update the server to 0.0.5 first")
        );
        assert_eq!(
            server_hold(&server, "0.0.3", 1).as_deref(),
            Some("The connected server does not support desktop 0.0.3")
        );
        // Older servers report only the API version they speak.
        let legacy = health(json!({"version":"0.0.1","api_version":1}));
        assert_eq!(server_hold(&legacy, "0.0.2", 1), None);
        assert!(server_hold(&legacy, "0.0.2", 2).is_some());
        // A failing or unknown server never holds the desktop.
        let failing = json!({"status":503,"body":{"api_version":1}});
        assert_eq!(server_hold(&failing, "0.0.2", 2), None);
        assert_eq!(server_hold(&health(json!({})), "0.0.2", 2), None);
    }
    #[test]
    fn automatic_retries_back_off_per_release() {
        let mut backoff = Backoff::default();
        assert!(backoff.allows("0.0.2", 0));
        backoff.failed("0.0.2", 1000);
        assert!(!backoff.allows("0.0.2", 1299));
        assert!(backoff.allows("0.0.2", 1300));
        assert!(backoff.allows("0.0.3", 1000));
        backoff.failed("0.0.2", 1300);
        assert!(!backoff.allows("0.0.2", 1899));
        assert!(backoff.allows("0.0.2", 1900));
        for _ in 0..40 {
            backoff.failed("0.0.2", 0);
        }
        assert_eq!(backoff.next, 6 * 3600);
        backoff.failed("0.0.3", 0);
        assert_eq!((backoff.failures, backoff.next), (1, 300));
    }
}
