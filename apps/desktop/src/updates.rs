//! Native update trust is independent of the connected server's administrator.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;
use thelxinoe_releases::{Envelope, Manifest};

#[derive(Default)]
pub struct Runtime(tokio::sync::Mutex<()>);
const PUBLIC_KEY: &str = match option_env!("THELXINOE_RELEASE_PUBLIC_KEY") {
    Some(key) => key,
    None => include_str!("../../../releases/release.pub"),
};
async fn candidate(app: &tauri::AppHandle) -> Result<Option<Manifest>, String> {
    let response =
        crate::backend_request(app.clone(), "/release".into(), "GET".into(), None).await?;
    let value = response["body"]["envelope"].clone();
    if value.is_null() {
        return Ok(None);
    }
    let envelope: Envelope =
        serde_json::from_value(value).map_err(|_| "Invalid signed release envelope")?;
    let manifest = thelxinoe_releases::verify(&envelope, PUBLIC_KEY)
        .map_err(|_| "Release was not signed by the trusted publisher")?;
    let current = app.package_info().version.clone();
    let next = semver::Version::parse(&manifest.version).map_err(|_| "Invalid release version")?;
    if next <= current {
        return Ok(None);
    }
    if manifest.published_at > thelxinoe_core::now() + 300
        || manifest.expires_at <= thelxinoe_core::now()
    {
        return Err("Release manifest has expired or is not valid yet".into());
    }
    let health = crate::backend_request(app.clone(), "/health".into(), "GET".into(), None).await?;
    let api = health["body"]["api_version"]
        .as_u64()
        .ok_or("Server API version is unavailable")?;
    if !manifest.api.contains(api as u32) {
        return Err("Update the server before installing this desktop release".into());
    }
    Ok(Some(manifest))
}
#[tauri::command]
pub async fn desktop_update_check(app: tauri::AppHandle) -> Result<Value, String> {
    let release = candidate(&app).await?;
    Ok(
        json!({"installed":app.package_info().version.to_string(),"release":release.map(|m|json!({"version":m.version,"notes":m.notes,"bytes":m.windows_x64.bytes}))}),
    )
}
#[tauri::command]
pub async fn desktop_update_install(app: tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<Runtime>();
    let _guard = runtime
        .0
        .try_lock()
        .map_err(|_| "A desktop update is already running")?;
    let release = candidate(&app)
        .await?
        .ok_or("No newer signed desktop release is available")?;
    let endpoint = thelxinoe_releases::https(&release.windows_x64.url)
        .map_err(|_| "Invalid signed artifact URL")?
        .join("windows-x64.json")
        .map_err(|_| "Invalid desktop release endpoint")?;
    // This endpoint carries public metadata. Device credentials never enter the updater client.
    let mut builder = app
        .updater_builder()
        .pubkey(&release.windows_x64.updater_public_key)
        .endpoints(vec![endpoint])
        .map_err(|_| "Invalid desktop release endpoint")?
        .timeout(std::time::Duration::from_secs(60));
    if let Some(pem) = option_env!("THELXINOE_RELEASE_CA_PEM") {
        let cert = reqwest::Certificate::from_pem(pem.as_bytes())
            .map_err(|_| "Invalid publisher TLS certificate")?;
        builder = builder.configure_client(move |c| c.add_root_certificate(cert.clone()));
    }
    let updater = builder
        .build()
        .map_err(|_| "Could not initialize desktop updates")?;
    let update = updater
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
    let bytes = tokio::select! {
        result=update.download(move|chunk,total|{received=received.saturating_add(chunk as u64);if received>limit || total.is_some_and(|n|n!=limit){signal.notify_one();}let _=handle.emit("desktop-update-progress",json!({"received":received,"total":limit}));},||{})=>result.map_err(|_|"Desktop download or signature verification failed")?,
        _=overflow.notified()=>return Err("Desktop artifact exceeds its signed size".into()),
    };
    let hash = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if bytes.len() as u64 != limit || hash != release.windows_x64.sha256 {
        return Err("Desktop artifact hash does not match the signed release".into());
    }
    #[cfg(windows)]
    app.state::<crate::mpv::DesktopPlayback>()
        .player
        .stop()
        .await;
    update
        .install(bytes)
        .map_err(|_| "Windows could not start the signed desktop installer".into())
}
