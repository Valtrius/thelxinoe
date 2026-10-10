//! Follow a connected service across Docker recreation without taking ownership of it.
//!
//! Compose, Watchtower and similar updaters replace a container while keeping its
//! name and mounts. Thelxinoe accepts the replacement only when it keeps the recorded
//! name, the exact mount evidence and the same service API identity and credentials.
use super::*;

pub(super) struct Attached {
    pub(super) kind: String,
    pub(super) container: String,
    pub(super) container_name: String,
    pub(super) port: u16,
    pub(super) url_base: String,
    pub(super) media_source: String,
    pub(super) credential: Vec<u8>,
    /// Installed and adopted services keep the identity their controller records.
    pub(super) owned: bool,
}

/// Return the container that now serves `id`, after `missing` disappeared from Docker.
pub(super) async fn follow(state: &AppState, id: &str, missing: &str) -> Result<String> {
    let record = storage::attached(id.to_owned(), &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let service = failures::label(&record.kind);
    if record.container != missing {
        // Another request already followed the replacement.
        return Ok(record.container);
    }
    let lost = |detail: &str| {
        ApiError(
            axum::http::StatusCode::CONFLICT,
            "container_missing",
            format!("{service}'s container no longer exists. {detail}"),
        )
    };
    if record.owned {
        return Err(lost("Review it in Media services."));
    }
    let name = record.container_name.as_str();
    if name.is_empty() {
        return Err(lost("Choose its current container in Edit connection."));
    }
    let containers = docker(state, "containers").await?;
    let Some(replacement) = containers["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| {
            row["names"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|n| n.as_str().map(|n| n.trim_start_matches('/')) == Some(name))
        })
        .and_then(|row| row["id"].as_str())
        .map(str::to_owned)
    else {
        return Err(lost(&format!(
            "No container is named {name} anymore; if it was recreated under another name, choose it in Edit connection."
        )));
    };
    let manager = matches!(record.kind.as_str(), "radarr" | "sonarr" | "lidarr");
    let (base, evidence) = if manager {
        evidence(state, &replacement, record.port).await?
    } else {
        support::api_evidence(state, &replacement, record.port, &record.kind).await?
    };
    if evidence != record.media_source {
        return Err(ApiError::conflict(format!(
            "{service}'s container was recreated as {name} with different storage mounts. Review the new layout in Edit connection before Thelxinoe uses it."
        )));
    }
    if docker(state, &format!("containers/{replacement}")).await?["running"] != true {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "dependency_unavailable",
            format!("{service}'s container was recreated as {name} and hasn't started yet"),
        ));
    }
    // The stored credential proves the replacement serves the same application state.
    let scope = format!("{}:{id}", if manager { "manager" } else { "support" });
    let plaintext = state.secrets.decrypt(&scope, &record.credential)?;
    let credentials = if manager {
        support::Credentials {
            username: String::new(),
            secret: String::from_utf8(plaintext).map_err(|_| unavailable())?,
        }
    } else {
        serde_json::from_slice(&plaintext).map_err(|_| unavailable())?
    };
    let c = Connection {
        state,
        base,
        url_base: record.url_base.clone(),
        key: credentials.secret.clone(),
        kind: record.kind.clone(),
    };
    if manager {
        let status = c.get("system/status").await?;
        access::check_reported_base(&record.kind, &record.url_base, &status)?;
    } else {
        support::version(&c, &credentials).await?;
    }
    if !storage::follow(
        &state.db,
        id.to_owned(),
        missing.to_owned(),
        replacement.clone(),
    )
    .await?
    {
        return match storage::attached(id.to_owned(), &state.db).await? {
            Some(current) if current.container != missing => Ok(current.container),
            _ => Err(lost(
                "Its replacement is already registered for another service; choose its container in Edit connection.",
            )),
        };
    }
    tracing::info!(service = %id, "Followed a recreated service container");
    state.managers.connection_wake.notify_one();
    state
        .emit(None, "stack.changed", json!({"service_id":id}))
        .await?;
    Ok(replacement)
}
