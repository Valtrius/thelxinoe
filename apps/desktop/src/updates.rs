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
}
struct Prepared {
    release: Manifest,
    update: Update,
    bytes: Vec<u8>,
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
            .filter(|v| v == "automatic")
            .unwrap_or_else(|| "notify".into());
        *value = json!({"installed":app.package_info().version.to_string(),"policy":policy,"phase":"idle","checked_at":null,"release":null,"error":null,"received":0,"total":0});
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
async fn compatibility(app: &tauri::AppHandle, release: &Manifest) -> Result<(), String> {
    // An offline server must not prevent maintaining this desktop.
    if let Ok(health) =
        crate::backend_request(app.clone(), "/health".into(), "GET".into(), None).await
        && health["status"] == 200
        && let Some(api) = health["body"]["api_version"].as_u64()
        && !u32::try_from(api)
            .ok()
            .is_some_and(|api| release.api.contains(api))
    {
        return Err("Update the connected server before installing this desktop release".into());
    }
    Ok(())
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
            Ok(change(
                &app,
                json!({"phase":phase,"release":release.map(|m| json!({"version":m.version,"notes":m.notes,"bytes":m.windows_x64.bytes})),"error":null}),
            ))
        }
        Err(error) => {
            runtime.prepared.lock().await.take();
            change(&app, json!({"release":null}));
            finish(&app, Err(error), "idle")
        }
    }
}
async fn download(app: &tauri::AppHandle, release: Manifest) -> Result<(), String> {
    let endpoint = thelxinoe_releases::https(&release.windows_x64.url)
        .and_then(|url| Ok(url.join("windows-x64.json")?))
        .map_err(|_| "Invalid desktop release endpoint")?;
    let mut builder = app
        .updater_builder()
        .pubkey(&release.windows_x64.updater_public_key)
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
    change(
        app,
        json!({"release":{"version":release.version,"notes":release.notes,"bytes":limit},"total":limit}),
    );
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
    install(&app).await
}
async fn playback_active(app: &tauri::AppHandle) -> bool {
    #[cfg(windows)]
    return !matches!(
        app.state::<crate::mpv::DesktopPlayback>()
            .player
            .view()
            .await
            .status
            .as_str(),
        "stopped" | "ended" | "error" | "failed" | ""
    );
    #[cfg(not(windows))]
    {
        let _ = app;
        false
    }
}
async fn install(app: &tauri::AppHandle) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let result = async {
        if playback_active(app).await {
            return Err("Stop desktop playback before restarting to install the update".into());
        }
        let mut prepared = runtime.prepared.lock().await;
        let pending = prepared
            .as_ref()
            .ok_or("Download and verify the desktop update first")?;
        pending
            .release
            .valid_at(thelxinoe_core::now())
            .map_err(|e| e.to_string())?;
        compatibility(app, &pending.release).await?;
        // The Windows installer exits the process without the normal close event.
        app.save_window_state(StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED)
            .map_err(|_| "Could not save desktop window state before restarting")?;
        change(app, json!({"phase":"installing","error":null}));
        let pending = prepared.take().ok_or("Desktop download is unavailable")?;
        pending
            .update
            .install(pending.bytes)
            .map_err(|_| "Windows could not start the signed desktop installer".to_string())
    }
    .await;
    let phase = if runtime.prepared.lock().await.is_some() {
        "ready"
    } else {
        "idle"
    };
    match result {
        Ok(()) => Ok(change(app, json!({"phase":"installing"}))),
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
            let _ = finish(&app, Err(error), "idle");
        }
    });
    Ok(status)
}
async fn apply(app: &tauri::AppHandle, expected: Option<String>) -> Result<(), String> {
    let release = candidate(app).await?;
    change(app, json!({"checked_at":thelxinoe_core::now()}));
    let Some(release) = release else {
        app.state::<Runtime>().prepared.lock().await.take();
        change(app, json!({"phase":"idle","release":null}));
        return Ok(());
    };
    if expected.as_ref().is_some_and(|v| *v != release.version) {
        change(
            app,
            json!({"release":{"version":release.version,"notes":release.notes,"bytes":release.windows_x64.bytes}}),
        );
        return Err("The available version changed. Click to install the new release".into());
    }
    compatibility(app, &release).await?;
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
        change(
            app,
            json!({"phase":"downloading","received":0,"total":release.windows_x64.bytes}),
        );
        download(app, release).await?;
    }
    while playback_active(app).await {
        change(app, json!({"phase":"waiting"}));
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    install(app).await?;
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
            let status = desktop_update_status(app.clone());
            if status["policy"] == "automatic"
                && status["phase"] == "idle"
                && status["error"].is_null()
                && !status["release"].is_null()
            {
                // Personal update preferences apply only to an authenticated desktop.
                if let Ok(session) =
                    crate::backend_request(app.clone(), "/auth/me".into(), "GET".into(), None).await
                    && session["status"] == 200
                {
                    let _ = desktop_update_apply(
                        app.clone(),
                        status["release"]["version"].as_str().map(str::to_owned),
                    )
                    .await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
