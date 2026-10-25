#[path = "../storage/managers/libraries.rs"]
mod storage;
#[cfg(test)]
#[path = "libraries_tests.rs"]
mod tests;

use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Store {
    errors: std::sync::RwLock<HashMap<String, String>>,
    pub(super) wake: tokio::sync::Notify,
}

pub(super) fn decorate(state: &AppState, rows: &mut [Value]) {
    let errors = state
        .managers
        .libraries
        .errors
        .read()
        .unwrap_or_else(|e| e.into_inner());
    for row in rows {
        row["library_error"] = json!(row["id"].as_str().and_then(|id| errors.get(id)));
    }
}

#[derive(Deserialize)]
struct Folder {
    path: String,
}

fn library_kind(kind: &str) -> &'static str {
    match kind {
        "radarr" => "movies",
        "sonarr" => "shows",
        _ => "music",
    }
}

async fn discover(state: &AppState, id: &str) -> Result<()> {
    // Reading roots grants no authority to change the connected application or container.
    let service = connected(state, id).await?;
    let connection = Connection::open(state, &service).await?;
    let mut folders: Vec<Folder> = connection.fetch("rootfolder", &[]).await?;
    folders.sort_by_key(|folder| folder.path.len());
    let mut errors = Vec::new();
    for folder in folders {
        let result = async {
            let path = storage_paths::directory(state, &service.media_source, &folder.path)
                .await?
                .ok_or_else(|| ApiError::conflict(
                    "Library folder is unavailable to Thelxinoe. Share this folder with the server inside its media mount"
                ))?;
            storage::ensure_root(
                &state.db,
                Some((service.id.clone(), service.generation.clone())),
                library_kind(&service.kind).into(),
                path,
            ).await?;
            Ok::<_, ApiError>(())
        }.await;
        if let Err(error) = result {
            errors.push(format!("{}: {}", folder.path, error.2));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::conflict(errors.join("; ")))
    }
}

async fn refresh(state: &AppState) -> anyhow::Result<()> {
    let services = storage::services(&state.db).await?;
    state
        .managers
        .libraries
        .errors
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|id, _| services.iter().any(|(key, _)| key == id));

    // Local-only libraries use the same directories as managed installations.
    for (manager, folder) in [("radarr", "movies"), ("sonarr", "tv"), ("lidarr", "music")] {
        if services.iter().any(|(_, kind)| kind == manager) {
            continue;
        }
        let path = state.config.media.join(folder);
        match tokio::fs::try_exists(&path).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                tracing::warn!(%error, folder, "Local library is unavailable");
                continue;
            }
        }
        match thelxinoe_catalog::approved_path(&path, &state.config.media) {
            Ok(path) => {
                if let Err(error) = storage::ensure_root(
                    &state.db,
                    None,
                    library_kind(manager).into(),
                    path.to_string_lossy().into_owned(),
                )
                .await
                {
                    tracing::warn!(error = %error.2, folder, "Local library could not be discovered");
                }
            }
            Err(error) => tracing::warn!(%error, folder, "Local library is unavailable"),
        }
    }

    // A stopped or slow manager must not delay discovery from the other services.
    let mut pending = tokio::task::JoinSet::new();
    for (id, _) in services {
        let state = state.clone();
        pending.spawn(async move {
            let result = discover(&state, &id).await;
            let mut errors = state.managers.libraries.errors.write().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(()) => { errors.remove(&id); }
                Err(error) => {
                    if errors.get(&id) != Some(&error.2) {
                        tracing::warn!(service = %id, error = %error.2, "Library discovery will retry");
                    }
                    errors.insert(id, error.2);
                }
            }
        });
    }
    while let Some(result) = pending.join_next().await {
        result?;
    }
    Ok(())
}

pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        if let Err(error) = refresh(&state).await {
            tracing::warn!(%error, "Library discovery will retry");
        }
        tokio::select! {
            () = state.managers.libraries.wake.notified() => {},
            () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
        }
    }
}
