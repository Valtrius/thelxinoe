#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod error;
mod models;
mod mpv;
mod tools;
mod updates;
mod utils;
use serde_json::Value;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
fn open_provider_url(app: tauri::AppHandle, value: String) -> Result<(), String> {
    let url = url::Url::parse(&value).map_err(|_| "Invalid provider URL")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(
            url.host_str(),
            Some("www.youtube.com" | "www.twitch.tv" | "kick.com")
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
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open your browser".into())
}

fn credential(app: &tauri::AppHandle) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&app.config().identifier, "server-session").map_err(|e| e.to_string())
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
    #[cfg(windows)]
    app.state::<mpv::DesktopPlayback>().player.stop().await;
    match credential(&app)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.to_string()),
    }
    let origin = url.origin().ascii_serialization();
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
    #[cfg(windows)]
    if path == "/auth/logout" && method == "POST" {
        app.state::<mpv::DesktopPlayback>().player.stop().await;
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains("..") || path.contains('#')
    {
        return Err("Invalid API path".into());
    }
    let base = server_url(app.clone())?;
    let auth = matches!(path.as_str(), "/auth/login" | "/setup") && method == "POST";
    if auth && let Some(value) = body.as_mut() {
        value["transport"] = Value::String("device".into());
        value["device_name"] = Value::String("Windows desktop".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client
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
        if let Some(token) = value["token"].as_str() {
            credential(&app)?
                .set_password(token)
                .map_err(|e| e.to_string())?;
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("token");
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
    tauri::Builder::default()
        .manage(mpv::DesktopPlayback::default())
        .manage(updates::Runtime::default())
        .setup(|app| {
            tools::initialize(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    #[cfg(windows)]
                    app.state::<mpv::DesktopPlayback>().player.stop().await;
                    let tools = &app.state::<tools::DesktopTools>().tools;
                    tools.close();
                    tools.wait_for_operations().await;
                    app.exit(0);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            server_url,
            change_server,
            backend_request,
            open_provider_url,
            open_youtube_linking,
            open_twitch_activation,
            updates::desktop_update_check,
            updates::desktop_update_install,
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
            mpv::mpv_command
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Thelxinoe");
}
