//! Optional display metadata, bound to the discovered digest without pulling image layers.
use crate::templates::Template;
use anyhow::{Context, Result, ensure};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use thelxinoe_core::service_release::ServiceRelease;

const MANIFEST_TYPES: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";
type Cache = HashMap<&'static str, (Instant, ServiceRelease)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub async fn metadata(template: Template, image: &str) -> ServiceRelease {
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((checked, release)) = cache.lock().unwrap().get(template.kind) {
        let lifetime = if release.version.is_some() { 3600 } else { 300 };
        if release.image == image && checked.elapsed() < Duration::from_secs(lifetime) {
            return release.clone();
        }
    }
    let mut release = ServiceRelease {
        image: image.into(),
        version: None,
        build_version: None,
        release_notes_url: None,
    };
    if crate::templates::pinned_image(template)
        .ok()
        .flatten()
        .as_deref()
        == Some(image)
    {
        if let Ok(raw) = crate::docker::engine(&format!("/images/{image}/json")).await {
            return from_labels(template.kind, image, &raw["Config"]["Labels"]);
        }
        return release;
    }
    // Metadata availability must never prevent discovery of an image update.
    if let Ok(client) = http_client() {
        let lookup = async {
            let repository = registry_repository(template).context("Unsupported registry")?;
            let digest = image
                .strip_prefix(&format!("{}@", template.repository))
                .context("Unexpected image repository")?;
            let token = registry_token(&client, repository).await?;
            image_labels(&client, "https://ghcr.io", repository, &token, digest).await
        };
        if let Ok(Ok(labels)) = tokio::time::timeout(Duration::from_secs(20), lookup).await {
            release = from_labels(template.kind, image, &labels);
            if let Some(version) = &release.version {
                release.release_notes_url = release_notes(&client, template.kind, version).await;
            }
        }
    }
    cache
        .lock()
        .unwrap()
        .insert(template.kind, (Instant::now(), release.clone()));
    release
}

fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("Thelxinoe service updates")
        .timeout(Duration::from_secs(6))
        .https_only(true)
        .build()?)
}

fn registry_repository(template: Template) -> Option<&'static str> {
    // LinuxServer publishes the same content-addressed images to GHCR. Requesting
    // and verifying the original digest also rejects any difference between mirrors.
    template
        .repository
        .strip_prefix("ghcr.io/")
        .or_else(|| template.repository.strip_prefix("lscr.io/"))
}

async fn read_body(request: RequestBuilder) -> Result<Vec<u8>> {
    let mut response = request.send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "Metadata is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn registry_token(client: &Client, repository: &str) -> Result<String> {
    let bytes = read_body(client.get("https://ghcr.io/token").query(&[
        ("service", "ghcr.io".to_owned()),
        ("scope", format!("repository:{repository}:pull")),
    ]))
    .await?;
    let response: Value = serde_json::from_slice(&bytes)?;
    Ok(response["token"]
        .as_str()
        .context("Missing public registry token")?
        .into())
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 71
        && digest.starts_with("sha256:")
        && digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
}

async fn registry_json(
    client: &Client,
    base: &str,
    repository: &str,
    token: &str,
    endpoint: &str,
    digest: &str,
) -> Result<Value> {
    ensure!(valid_digest(digest), "Invalid digest");
    let bytes = read_body(
        client
            .get(format!("{base}/v2/{repository}/{endpoint}/{digest}"))
            .bearer_auth(token)
            .header("Accept", MANIFEST_TYPES),
    )
    .await?;
    ensure!(
        format!("sha256:{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(digest),
        "Metadata digest mismatch"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

async fn image_labels(
    client: &Client,
    base: &str,
    repository: &str,
    token: &str,
    digest: &str,
) -> Result<Value> {
    let mut manifest = registry_json(client, base, repository, token, "manifests", digest).await?;
    if let Some(platforms) = manifest["manifests"].as_array() {
        // Managed-service preflight currently supports Linux amd64 only.
        let platform = platforms
            .iter()
            .find(|item| {
                item["platform"]["os"] == "linux" && item["platform"]["architecture"] == "amd64"
            })
            .context("No supported image platform")?;
        let digest = platform["digest"]
            .as_str()
            .context("Missing platform digest")?;
        manifest = registry_json(client, base, repository, token, "manifests", digest).await?;
    }
    let digest = manifest["config"]["digest"]
        .as_str()
        .context("Missing config digest")?;
    let config = registry_json(client, base, repository, token, "blobs", digest).await?;
    ensure!(
        config["os"] == "linux" && config["architecture"] == "amd64",
        "Unsupported config platform"
    );
    Ok(config["config"]["Labels"].clone())
}

fn from_labels(kind: &str, image: &str, labels: &Value) -> ServiceRelease {
    let build = labels["org.opencontainers.image.version"]
        .as_str()
        .or_else(|| {
            labels["build_version"]
                .as_str()?
                .strip_prefix("Linuxserver.io version:- ")?
                .split_whitespace()
                .next()
        });
    let version = build.and_then(|build| {
        let value = build.strip_prefix('v').unwrap_or(build);
        let value = if kind == "seerr" {
            value
        } else {
            value.split_once("-ls").map_or(value, |(app, _)| app)
        };
        // Stable curated releases use numeric dotted versions. Unknown tags remain unknown.
        (value.len() <= 80
            && value.contains('.')
            && value
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())))
        .then(|| value.to_owned())
    });
    ServiceRelease {
        image: image.into(),
        build_version: version.as_ref().and(build.map(str::to_owned)),
        version,
        release_notes_url: None,
    }
}

async fn release_notes(client: &Client, kind: &str, version: &str) -> Option<String> {
    let repository = match kind {
        "radarr" => "Radarr/Radarr",
        "sonarr" => "Sonarr/Sonarr",
        "lidarr" => "Lidarr/Lidarr",
        "prowlarr" => "Prowlarr/Prowlarr",
        "bazarr" => "morpheus65535/bazarr",
        "nzbget" => "nzbgetcom/nzbget",
        "seerr" => "seerr-team/seerr",
        "recyclarr" => "recyclarr/recyclarr",
        _ => return None,
    };
    let tag = format!("v{version}");
    let bytes = read_body(client.get(format!(
        "https://api.github.com/repos/{repository}/releases/tags/{tag}"
    )))
    .await
    .ok()?;
    let release: Value = serde_json::from_slice(&bytes).ok()?;
    let url = format!("https://github.com/{repository}/releases/tag/{tag}");
    (release["tag_name"] == tag && release["html_url"] == url && release["draft"] == false)
        .then_some(url)
}

#[cfg(test)]
#[path = "service_releases_tests.rs"]
mod tests;
