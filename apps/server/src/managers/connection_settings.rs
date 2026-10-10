use super::*;
#[path = "../storage/managers/connection_settings.rs"]
mod storage;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/services/{id}/connection",
            get(settings).put(save),
        )
        .route("/api/v1/admin/services/{id}/connection/test", post(test))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    port: u16,
    url_base: String,
    secret: Option<String>,
    username: Option<String>,
    /// Point the connection at another container, such as a renamed replacement.
    #[serde(default)]
    container_id: Option<String>,
    #[serde(default)]
    allow_unverified: bool,
}
struct Record {
    kind: String,
    container: String,
    credential: Vec<u8>,
}
async fn settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let record = storage::load(&state.db, key.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let username = if record.kind == "nzbget" {
        let credentials: support::Credentials = serde_json::from_slice(
            &state
                .secrets
                .decrypt(&format!("support:{key}"), &record.credential)?,
        )
        .map_err(|_| unavailable())?;
        Some(credentials.username)
    } else {
        None
    };
    Ok(Json(json!({"username":username})))
}
async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    edit(state, headers, key, input, true).await
}
async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    edit(state, headers, key, input, false).await
}
async fn edit(
    state: AppState,
    headers: HeaderMap,
    key: String,
    input: Edit,
    test: bool,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let initial = storage::load(&state.db, key.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let _guard = state.managers.guard.service(&initial.kind).await;
    let record = storage::load(&state.db, key.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    if input.port == 0 || (!access::supported(&record.kind) && !input.url_base.is_empty()) {
        return Err(ApiError::bad("Choose a valid internal port and URL Base"));
    }
    access::validate_base(&record.kind, &input.url_base)?;
    let manager = matches!(record.kind.as_str(), "radarr" | "sonarr" | "lidarr");
    let scope = format!("{}:{key}", if manager { "manager" } else { "support" });
    let plaintext = state.secrets.decrypt(&scope, &record.credential)?;
    let mut credentials = if manager {
        support::Credentials {
            username: String::new(),
            secret: String::from_utf8(plaintext).map_err(|_| unavailable())?,
        }
    } else {
        serde_json::from_slice::<support::Credentials>(&plaintext).map_err(|_| unavailable())?
    };
    if let Some(secret) = input.secret {
        credentials.secret = secret;
    }
    if let Some(username) = input.username {
        if record.kind != "nzbget" {
            return Err(ApiError::bad("This connection does not use a username"));
        }
        credentials.username = username;
    }
    let service = failures::label(&record.kind);
    if credentials.secret.len() > 1024
        || credentials.username.len() > 100
        || credentials.secret.chars().any(char::is_control)
        || credentials.username.chars().any(char::is_control)
        || (record.kind != "nzbget" && credentials.secret.is_empty())
        || (manager && !(16..=256).contains(&credentials.secret.len()))
    {
        return Err(ApiError::bad(if record.kind == "nzbget" {
            "Enter NZBGet's control username and password".to_owned()
        } else {
            format!("Enter the API key from {service}'s Settings → General")
        }));
    }
    let container = input
        .container_id
        .clone()
        .filter(|container| !container.is_empty())
        .unwrap_or_else(|| record.container.clone());
    if container != record.container {
        let attached = super::storage::attached(key.clone(), &state.db)
            .await?
            .ok_or_else(ApiError::not_found)?;
        if attached.owned {
            return Err(ApiError::conflict(format!(
                "Thelxinoe manages this {service} container; review it in Media services instead of choosing another one"
            )));
        }
    }
    let (base, media_source) =
        support::api_evidence(&state, &container, input.port, &record.kind).await?;
    let c = Connection {
        state: &state,
        base,
        url_base: input.url_base.clone(),
        key: credentials.secret.clone(),
        kind: record.kind.clone(),
    };
    let observed = docker(&state, &format!("containers/{container}")).await?;
    let container_name = observed["name"]
        .as_str()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_owned();
    let version = if observed["running"] == false {
        if test {
            return Err(ApiError::conflict(format!(
                "The {container_name} container is stopped; start it before testing the connection"
            )));
        }
        if !input.allow_unverified {
            return Err(failures::stopped(&container_name));
        }
        "Unverified".to_owned()
    } else {
        let version = if manager {
            let status = c.get("system/status").await?;
            if !status["appName"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&record.kind))
            {
                return Err(failures::identity(&record.kind, &status["appName"]));
            }
            status["version"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() < 100)
                .ok_or_else(|| failures::identity(&record.kind, &Value::Null))?
                .to_owned()
        } else {
            support::version(&c, &credentials).await?
        };
        if record.kind == "seerr" {
            c.get("settings/main").await?;
        } else if record.kind != "nzbget" {
            access::check_connection(&c).await?;
        }
        version
    };
    if test {
        return Ok(Json(json!({"healthy":true})));
    }
    let plaintext = if manager {
        credentials.secret.into_bytes()
    } else {
        serde_json::to_vec(&credentials).map_err(|_| unavailable())?
    };
    let encrypted = state.secrets.encrypt(&scope, &plaintext)?;
    if !storage::save(
        &state.db,
        key.clone(),
        record,
        (container, container_name),
        input.port,
        input.url_base,
        media_source,
        version,
        encrypted,
        actor.user.id,
    )
    .await?
    {
        return Err(ApiError::conflict(
            "Finish the current ownership or update operation before editing this connection, and choose a container no other service uses",
        ));
    }
    state.managers.connection_wake.notify_one();
    state
        .emit(None, "stack.changed", json!({"service_id":key}))
        .await?;
    Ok(Json(json!({"id":key})))
}
