#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod error;
mod models;
mod mpv;
mod tools;
mod updates;
mod utils;
use serde_json::Value;
use tauri::{Manager, webview::PageLoadEvent};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};

#[tauri::command]
fn finish_startup(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can finish application startup".into());
    }
    reveal(&app, &window)
}
fn reveal(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.is_visible().map_err(|e| e.to_string())? {
        return Ok(());
    }
    if let Err(error) = window.restore_state(StateFlags::MAXIMIZED) {
        tracing::warn!(%error, "Could not restore desktop maximized state");
    }
    window.show().map_err(|e| e.to_string())?;
    if let Some(splashscreen) = app.get_webview_window("splashscreen")
        && let Err(error) = splashscreen.close()
    {
        tracing::warn!(%error, "Could not close desktop splashscreen");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(%error, "Could not focus desktop window");
    }
    Ok(())
}
/// The splash stays on top without a taskbar entry. If the frontend never finishes
/// startup, show the main window anyway so the app can be closed or updated.
fn reveal_after_timeout(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(40)).await;
        if let Some(window) = app.get_webview_window("main")
            && let Err(error) = reveal(&app, &window)
        {
            tracing::warn!(%error, "Could not show desktop window after startup timed out");
        }
    });
}

#[tauri::command]
fn open_provider_url(app: tauri::AppHandle, value: String) -> Result<(), String> {
    let url = url::Url::parse(&value).map_err(|_| "Invalid provider URL")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(
            url.host_str(),
            Some(
                "www.youtube.com"
                    | "www.twitch.tv"
                    | "kick.com"
                    | "console.cloud.google.com"
                    | "developers.google.com"
                    | "dev.twitch.tv"
                    | "docs.kick.com"
            )
        )
    {
        return Err("Invalid provider URL".into());
    }
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

#[tauri::command]
fn open_twitch_activation(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("https://www.twitch.tv/activate", None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

#[tauri::command]
async fn open_youtube_linking(app: tauri::AppHandle) -> Result<(), String> {
    let url = youtube_linking_url(&app).await?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum PublicPage {
    Home,
    Privacy,
}

#[tauri::command]
async fn open_public_page(app: tauri::AppHandle, page: PublicPage) -> Result<(), String> {
    let mut url = youtube_linking_url(&app).await?;
    url.set_path(match page {
        PublicPage::Home => "/about/index.html",
        PublicPage::Privacy => "/about/privacy.html",
    });
    url.set_query(None);
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

async fn youtube_linking_url(app: &tauri::AppHandle) -> Result<url::Url, String> {
    let response =
        backend_request(app.clone(), "/online/youtube".into(), "GET".into(), None).await?;
    if response["status"] != 200 {
        return Err("Sign in to Thelxinoe first".into());
    }
    let value = response["body"]["linking_url"]
        .as_str()
        .ok_or("Configure the server's public URL before linking YouTube")?;
    let url = url::Url::parse(value).map_err(|_| "Invalid server linking URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query() != Some("section=YouTube")
        || url.fragment().is_some()
    {
        return Err("Invalid server linking URL".into());
    }
    Ok(url)
}

/// One connection pool for every request to the server. `localhost` connects over
/// IPv4 first: Windows takes about two seconds to refuse `::1` when the server only
/// listens on IPv4, and every new connection would wait for that attempt first.
fn http() -> Result<reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .resolve_to_addrs(
                    "localhost",
                    &[
                        (std::net::Ipv4Addr::LOCALHOST, 0).into(),
                        (std::net::Ipv6Addr::LOCALHOST, 0).into(),
                    ],
                )
                .build()
                .map_err(|e| e.to_string())
        })
        .clone()
}
fn credential(app: &tauri::AppHandle) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&app.config().identifier, "server-session").map_err(|e| e.to_string())
}
fn remembered_credential(app: &tauri::AppHandle) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&app.config().identifier, "server-remember").map_err(|e| e.to_string())
}
/// Marks this desktop as one that signed in before, so others' failed guesses can't lock it out.
fn known_credential(app: &tauri::AppHandle) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&app.config().identifier, "server-known").map_err(|e| e.to_string())
}
#[tauri::command]
async fn open_auth_browser(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let response = backend_request(app.clone(), "/auth/methods".into(), "GET".into(), None).await?;
    let canonical = response["body"]["canonical_url"]
        .as_str()
        .ok_or("Authentication hostname is unavailable")?;
    let target = url::Url::parse(&url).map_err(|_| "Invalid authentication URL")?;
    if target.origin().ascii_serialization() != canonical
        || target.path() != "/"
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some_and(|f| f != "settings/account")
    {
        return Err("Invalid authentication URL".into());
    }
    app.opener()
        .open_url(target.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

#[tauri::command]
async fn open_service(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err("Invalid service".into());
    }
    let response = backend_request(
        app.clone(),
        format!("/admin/managers/{id}/access-ticket"),
        "POST".into(),
        None,
    )
    .await?;
    if response["status"] != 200 {
        return Err(response["body"]["error"]["message"]
            .as_str()
            .unwrap_or("Service access is unavailable")
            .to_owned());
    }
    if let Some(native) = response["body"]["native_url"].as_str() {
        let url = url::Url::parse(native).map_err(|_| "Invalid native service address")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Invalid native service address".into());
        }
        return app
            .opener()
            .open_url(url.as_str(), None::<&str>)
            .map_err(|_| "Could not open your browser".into());
    }
    let path = response["body"]["path"]
        .as_str()
        .ok_or("Invalid service link")?;
    let ticket = response["body"]["ticket"]
        .as_str()
        .ok_or("Invalid service link")?;
    if path != format!("/service-access/{id}")
        || ticket.is_empty()
        || ticket.len() > 256
        || !ticket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid service link".into());
    }
    let configured = server_url(app.clone())?;
    let origin = response["body"]["public_origin"]
        .as_str()
        .unwrap_or(&configured);
    let mut url = url::Url::parse(origin).map_err(|_| "Invalid server address")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Invalid server public address".into());
    }
    url.set_path(path);
    // The fragment is consumed by the browser bootstrap, never sent in the URL
    // to either server. The device credential stays in the OS keyring.
    url.set_fragment(Some(ticket));
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}
fn config_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("server-url"))
}
#[tauri::command]
fn server_url(app: tauri::AppHandle) -> Result<String, String> {
    Ok(std::fs::read_to_string(config_path(&app)?).unwrap_or("http://127.0.0.1:8484".into()))
}
#[tauri::command]
async fn change_server(app: tauri::AppHandle, value: String) -> Result<String, String> {
    let url = url::Url::parse(&value).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Enter an HTTP(S) origin without a path or credentials".into());
    }
    let origin = url.origin().ascii_serialization();
    if server_url(app.clone())?.trim_end_matches('/') == origin {
        return Ok(origin);
    }
    app.state::<mpv::DesktopPlayback>().player.stop(None).await;
    match credential(&app)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.to_string()),
    }
    for entry in [remembered_credential(&app)?, known_credential(&app)?] {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    std::fs::write(config_path(&app)?, &origin).map_err(|e| e.to_string())?;
    Ok(origin)
}
#[tauri::command]
async fn backend_request(
    app: tauri::AppHandle,
    path: String,
    method: String,
    mut body: Option<Value>,
) -> Result<Value, String> {
    if path == "/auth/logout" && method == "POST" {
        app.state::<mpv::DesktopPlayback>().player.stop(None).await;
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains("..") || path.contains('#')
    {
        return Err("Invalid API path".into());
    }
    let base = server_url(app.clone())?;
    let auth = matches!(
        path.as_str(),
        "/auth/login" | "/setup" | "/auth/totp" | "/auth/desktop/exchange"
    ) && method == "POST";
    if matches!(path.as_str(), "/auth/login" | "/setup")
        && method == "POST"
        && let Some(value) = body.as_mut()
    {
        value["transport"] = Value::String("device".into());
        value["device_name"] = Value::String("Windows desktop".into());
        if path == "/auth/login" {
            for (field, entry) in [
                ("remember_token", remembered_credential(&app)?),
                ("known_token", known_credential(&app)?),
            ] {
                match entry.get_password() {
                    Ok(raw) => value[field] = Value::String(raw),
                    Err(keyring::Error::NoEntry) => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
    }
    let mut request = http()?
        .request(
            method
                .parse::<reqwest::Method>()
                .map_err(|e| e.to_string())?,
            format!("{base}/api/v1{path}"),
        )
        .header("X-Thelxinoe-Client", "1")
        .header("X-Thelxinoe-API", thelxinoe_core::API_VERSION.to_string());
    match credential(&app)?.get_password() {
        Ok(token) => request = request.bearer_auth(token),
        Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.to_string()),
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Cannot connect to the configured server".to_string())?;
    let status = response.status().as_u16();
    let mut value: Value = response
        .json()
        .await
        .map_err(|_| "Server returned an invalid response".to_string())?;
    if auth && status == 200 {
        if server_url(app.clone())? != base {
            return Err("The configured server changed during sign-in".into());
        }
        if let Some(token) = value["token"].as_str() {
            credential(&app)?
                .set_password(token)
                .map_err(|e| e.to_string())?;
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("token");
            for (field, entry) in [
                ("remember_token", remembered_credential(&app)?),
                ("known_token", known_credential(&app)?),
            ] {
                if let Some(raw) = object
                    .remove(field)
                    .and_then(|v| v.as_str().map(str::to_owned))
                {
                    entry.set_password(&raw).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    if path == "/auth/logout" && status == 200 {
        match credential(&app)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(serde_json::json!({"status":status,"body":value}))
}
fn main() {
    let window_state = StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED;
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION | StateFlags::SIZE)
                .with_denylist(&["splashscreen"])
                .build(),
        )
        .manage(mpv::DesktopPlayback::default())
        .manage(updates::Runtime::default())
        .setup(|app| {
            // Desktop updates start first, so no other startup failure can strand a release.
            updates::start(app.handle().clone());
            reveal_after_timeout(app.handle().clone());
            tools::initialize(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_page_load(|webview, payload| {
            if webview.label() == "splashscreen"
                && matches!(payload.event(), PageLoadEvent::Finished)
                && let Err(error) = webview.window().show()
            {
                tracing::warn!(%error, "Could not show desktop splashscreen");
            }
        })
        .on_window_event(move |window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                if let Err(error) = app.save_window_state(window_state) {
                    tracing::warn!(%error, "Could not save desktop window state");
                }
                tauri::async_runtime::spawn(async move {
                    app.state::<mpv::DesktopPlayback>().player.stop(None).await;
                    if let Some(state) = app.try_state::<tools::DesktopTools>()
                        && let Ok(tools) = state.manager()
                    {
                        tools.close();
                        tools.wait_for_operations().await;
                    }
                    app.exit(0);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            finish_startup,
            server_url,
            change_server,
            backend_request,
            open_auth_browser,
            open_provider_url,
            open_youtube_linking,
            open_public_page,
            open_service,
            open_twitch_activation,
            updates::desktop_update_check,
            updates::desktop_update_status,
            updates::desktop_update_download,
            updates::desktop_update_policy,
            updates::desktop_update_install,
            updates::desktop_update_apply,
            tools::commands::tools_get,
            tools::commands::tools_check,
            tools::commands::tools_updates,
            tools::commands::tools_update,
            tools::commands::tools_set_preference,
            tools::commands::tools_refresh,
            tools::commands::tools_install,
            tools::commands::tools_activate,
            tools::commands::tools_rollback,
            tools::commands::tools_remove,
            tools::commands::tools_repair,
            tools::commands::mpv_set_preferences,
            tools::commands::mpv_config_files,
            tools::commands::mpv_config_read,
            tools::commands::mpv_config_save,
            tools::commands::mpv_config_restore,
            tools::commands::mpv_config_import,
            tools::commands::mpv_plugin_import,
            tools::commands::mpv_open_configuration_directory,
            tools::commands::mpv_options,
            tools::commands::mpv_test_configuration,
            mpv::mpv_play,
            mpv::mpv_state,
            mpv::mpv_sessions,
            mpv::mpv_command
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Thelxinoe");
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
    };

    // A server that also answers on ::1 is still reached over IPv4 first.
    #[tokio::test]
    async fn localhost_connects_over_ipv4_first() {
        let ipv4 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = ipv4.local_addr().unwrap().port();
        let ipv6 = TcpListener::bind(("::1", port)).await.ok();
        let (accepted, mut family) = mpsc::unbounded_channel();
        for listener in std::iter::once(ipv4).chain(ipv6) {
            let accepted = accepted.clone();
            tokio::spawn(async move {
                let (mut stream, peer) = listener.accept().await.unwrap();
                let _ = accepted.send(peer.is_ipv4());
                let mut request = [0; 1024];
                let _ = stream.read(&mut request).await;
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await
                    .unwrap();
            });
        }
        let response = crate::http()
            .unwrap()
            .get(format!("http://localhost:{port}/"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(family.recv().await, Some(true));
    }
}
