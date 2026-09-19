#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod mpv;
use serde_json::Value;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

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

fn credential() -> Result<keyring::Entry, String> {
    keyring::Entry::new("app.thelxinoe.desktop", "server-session").map_err(|e| e.to_string())
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
    match credential()?.delete_credential() {
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
    let base = server_url(app)?;
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
        .header("X-Thelxinoe-Client", "1");
    match credential()?.get_password() {
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
            credential()?
                .set_password(token)
                .map_err(|e| e.to_string())?;
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("token");
        }
    }
    if path == "/auth/logout" && status == 200 {
        match credential()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(serde_json::json!({"status":status,"body":value}))
}
fn main() {
    tauri::Builder::default()
        .manage(mpv::DesktopPlayback::default())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    #[cfg(windows)]
                    app.state::<mpv::DesktopPlayback>().player.stop().await;
                    app.exit(0);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            server_url,
            change_server,
            backend_request,
            open_youtube_linking,
            mpv::mpv_settings,
            mpv::mpv_install,
            mpv::mpv_custom,
            mpv::mpv_configuration,
            mpv::mpv_plugin,
            mpv::mpv_play,
            mpv::mpv_state,
            mpv::mpv_command
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Thelxinoe");
}
