//! Translate inspected mount identities; native application paths remain unchanged.
use super::*;
#[derive(Serialize, Deserialize)]
struct Evidence {
    server: Vec<Value>,
    service: Vec<Value>,
    media: String,
}
pub(super) fn evidence(server: &Value, service: &Value, media: &str) -> Result<String> {
    if !clean_path(media) {
        return Err(ApiError::conflict(
            "The server media directory must be an absolute container path",
        ));
    }
    let mounts = |value: &Value| -> Result<Vec<Value>> {
        let mut result = value["mounts"].as_array().ok_or_else(unavailable)?.clone();
        for m in &mut result {
            if !m["destination"].as_str().is_some_and(clean_path) {
                return Err(unavailable());
            }
            if m["kind"] == "volume" {
                let subpath = m["subpath"].as_str().unwrap_or("").trim_matches('/');
                if subpath.split('/').any(|v| v == "." || v == "..")
                    || subpath.contains(['\\', '\0'])
                {
                    return Err(unavailable());
                }
                m["subpath"] = json!(subpath);
            }
            if m["kind"] == "bind" {
                m["source"] = json!(
                    m["source"]
                        .as_str()
                        .and_then(host_path)
                        .ok_or_else(unavailable)?
                );
            }
        }
        result.sort_by_key(|m| m["destination"].as_str().unwrap_or("").to_owned());
        Ok(result)
    };
    let server = mounts(server)?
        .into_iter()
        .filter(|m| {
            m["destination"]
                .as_str()
                .is_some_and(|p| suffix(p, media).is_some() || suffix(media, p).is_some())
        })
        .collect();
    serde_json::to_string(&Evidence {
        server,
        service: mounts(service)?,
        media: media.into(),
    })
    .map_err(|_| unavailable())
}
pub(super) async fn resolve(
    state: &AppState,
    evidence: &str,
    path: &str,
) -> Result<Option<String>> {
    if !clean_path(path) {
        return Err(ApiError::conflict(
            "The manager reported an unsafe file path",
        ));
    }
    let e: Evidence = serde_json::from_str(evidence).map_err(|_| unavailable())?;
    let Some(private) = e
        .service
        .iter()
        .filter(|m| {
            m["destination"]
                .as_str()
                .is_some_and(|root| suffix(path, root).is_some())
        })
        .max_by_key(|m| m["destination"].as_str().unwrap_or("").len())
    else {
        return Ok(None);
    };
    let relative = suffix(
        path,
        private["destination"].as_str().ok_or_else(unavailable)?,
    )
    .ok_or_else(unavailable)?;
    let source = private["source"].as_str().ok_or_else(unavailable)?;
    let physical = format!("{}{relative}", source.trim_end_matches('/'));
    let mut candidates = Vec::new();
    for public in &e.server {
        if private["kind"] != public["kind"] {
            continue;
        }
        let public_source = public["source"].as_str().ok_or_else(unavailable)?;
        let volume_path;
        let volume_root;
        let tail = if private["kind"] == "volume" {
            if private["name"] != public["name"] || !private["name"].is_string() {
                continue;
            }
            volume_path = format!(
                "{}{}",
                format!("/{}", private["subpath"].as_str().unwrap_or("")).trim_end_matches('/'),
                relative
            );
            volume_root = format!("/{}", public["subpath"].as_str().unwrap_or(""));
            let Some(tail) = suffix(&volume_path, &volume_root) else {
                continue;
            };
            tail
        } else if private["kind"] == "bind" {
            let Some(tail) = suffix(&physical, public_source) else {
                continue;
            };
            tail
        } else {
            continue;
        };
        let candidate = format!(
            "{}{tail}",
            public["destination"]
                .as_str()
                .ok_or_else(unavailable)?
                .trim_end_matches('/')
        );
        if suffix(&candidate, &e.media).is_none() {
            continue;
        }
        // A nested server mount may hide this location; its physical reference must agree.
        let visible = e
            .server
            .iter()
            .filter(|m| {
                m["destination"]
                    .as_str()
                    .is_some_and(|root| suffix(&candidate, root).is_some())
            })
            .max_by_key(|m| m["destination"].as_str().unwrap_or("").len());
        if visible != Some(public) {
            continue;
        }
        candidates.push(candidate);
    }
    candidates.sort();
    candidates.dedup();
    if candidates.len() != 1 {
        return Ok(None);
    }
    let path = std::path::PathBuf::from(&candidates[0]);
    let root = tokio::fs::canonicalize(&state.config.media)
        .await
        .map_err(|_| unavailable())?;
    let resolved = match tokio::fs::canonicalize(&path).await {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(ApiError::conflict(
                "Playback requires permission to read this media location",
            ));
        }
    };
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(ApiError::conflict(
            "The mapped file escapes approved media storage",
        ));
    }
    tokio::fs::File::open(&resolved).await.map_err(|_| {
        ApiError::conflict("Playback requires permission to read this media location")
    })?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
