use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode, header::HeaderMap};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    catalog,
    models::{Catalog, Package, Registry, ToolId},
    providers::provider,
};
use crate::error::{AppError, AppResult};

const MAX_METADATA: usize = 4 * 1024 * 1024;
const MAX_SCRIPT_ARCHIVE: usize = 8 * 1024 * 1024;
const VERSION_COUNT: usize = 10;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
    state: String,
}
#[derive(Deserialize)]
struct Commit {
    sha: String,
    commit: CommitInfo,
}
#[derive(Deserialize)]
struct CommitInfo {
    committer: Committer,
}
#[derive(Deserialize)]
struct Committer {
    date: String,
}

pub struct Discovery {
    pub packages: Vec<Package>,
    pub etag: Option<String>,
}

pub struct Github<'a> {
    pub client: &'a Client,
    pub retry_at: Option<String>,
}

impl Github<'_> {
    pub async fn discover(
        &mut self,
        tool: ToolId,
        previous: &[Package],
        etag: Option<&str>,
    ) -> AppResult<Discovery> {
        if let Some(until) = &self.retry_at {
            return Err(AppError::validation(format!(
                "GitHub update checks are paused until {until}."
            )));
        }
        let definition = provider(tool);
        let endpoint = if definition.uses_commits() {
            "commits?per_page=5"
        } else {
            "releases?per_page=20"
        };
        let mut request = self
            .client
            .get(format!(
                "https://api.github.com/repos/{}/{endpoint}",
                definition.repository
            ))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .timeout(Duration::from_secs(30));
        if !previous.is_empty()
            && let Some(etag) = etag
        {
            request = request.header("If-None-Match", etag);
        }
        let response = request.send().await.map_err(network_error)?;
        if let Some(until) = retry_time(response.status(), response.headers(), Utc::now()) {
            self.retry_at = Some(until.to_rfc3339());
        }
        if response.status() == StatusCode::NOT_MODIFIED {
            if previous.is_empty() {
                return Err(AppError::validation(
                    "GitHub returned an empty cached release list.",
                ));
            }
            return Ok(Discovery {
                packages: previous.to_vec(),
                etag: etag.map(str::to_string),
            });
        }
        if !response.status().is_success() {
            return Err(AppError::validation(match &self.retry_at {
                Some(until) => format!("GitHub update checks are paused until {until}."),
                None => format!("GitHub release lookup returned HTTP {}.", response.status()),
            }));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);
        let bytes = bounded_body(response, MAX_METADATA).await?;
        let packages = if definition.uses_commits() {
            self.commits(tool, &bytes, previous).await?
        } else {
            releases(tool, &bytes, previous)?
        };
        catalog::validate(&Catalog {
            packages: packages.clone(),
        })?;
        Ok(Discovery { packages, etag })
    }

    async fn commits(
        &self,
        tool: ToolId,
        bytes: &[u8],
        previous: &[Package],
    ) -> AppResult<Vec<Package>> {
        let commits: Vec<Commit> = serde_json::from_slice(bytes)
            .map_err(|e| AppError::validation(format!("Invalid upstream commit list: {e}")))?;
        if commits.is_empty() {
            return Err(AppError::validation(
                "The plugin repository returned no commits.",
            ));
        }
        let mut packages = Vec::new();
        for commit in commits.into_iter().take(5) {
            let url = commit_url(tool, &commit.sha)?;
            let cached = previous.iter().find(|p| p.url == url && p.tool == tool);
            let (hash, size) = if let Some(cached) = cached {
                (cached.sha256.clone(), cached.size)
            } else {
                // Commit archives have no release-asset digest. Fetch the small pinned
                // snapshot over HTTPS, then retain its SHA-256 for installation/repair.
                let response = self
                    .client
                    .get(&url)
                    .timeout(Duration::from_secs(30))
                    .send()
                    .await
                    .map_err(network_error)?
                    .error_for_status()
                    .map_err(network_error)?;
                let bytes = bounded_body(response, MAX_SCRIPT_ARCHIVE).await?;
                (
                    Sha256::digest(&bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                    bytes.len() as u64,
                )
            };
            packages.push(package(
                tool,
                &commit.sha[..12],
                url,
                (hash, size),
                "zip",
                &commit.commit.committer.date,
                previous,
            ));
        }
        mark_latest(&mut packages)?;
        Ok(packages)
    }
}

fn commit_url(tool: ToolId, sha: &str) -> AppResult<String> {
    if !provider(tool).uses_commits()
        || sha.len() != 40
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(AppError::validation(
            "The plugin snapshot needs a complete commit SHA.",
        ));
    }
    Ok(format!(
        "https://codeload.github.com/{}/zip/{sha}",
        provider(tool).repository
    ))
}

fn releases(tool: ToolId, bytes: &[u8], previous: &[Package]) -> AppResult<Vec<Package>> {
    let mut releases: Vec<Release> = serde_json::from_slice(bytes)
        .map_err(|e| AppError::validation(format!("Invalid upstream release list: {e}")))?;
    releases.retain(|r| {
        !r.draft
            && !r.prerelease
            && !matches!(r.tag_name.as_str(), "latest" | "nightly")
            && r.published_at.is_some()
    });
    releases.sort_by_key(|r| {
        std::cmp::Reverse(timestamp(r.published_at.as_deref().unwrap_or_default()))
    });
    let pattern = regex::Regex::new(provider(tool).asset_pattern)
        .map_err(|e| AppError::internal(e.to_string()))?;
    let mut packages = Vec::new();
    for release in releases {
        let mut assets: Vec<_> = release
            .assets
            .iter()
            .filter(|a| pattern.is_match(&a.name))
            .collect();
        // FFmpeg publishes several maintenance branches in one dated release.
        // Pick the newest numeric LGPL branch, without hard-coding its major version.
        if tool == ToolId::Ffmpeg {
            assets.sort_by_key(|a| std::cmp::Reverse(ffmpeg_branch(&pattern, &a.name)));
        } else if assets.len() > 1 {
            return Err(AppError::validation(
                "Upstream has multiple matching Windows packages; installation rules need updating.",
            ));
        }
        let Some(asset) = assets.first() else {
            continue;
        };
        if asset.state != "uploaded" {
            return Err(AppError::validation(
                "The newest upstream package is still being uploaded. Retry later.",
            ));
        }
        let hash = match asset.digest.as_deref() {
            Some(digest) => Some(
                digest
                    .strip_prefix("sha256:")
                    .filter(|h| catalog::valid_hash(h))
                    .ok_or_else(|| {
                        AppError::validation("Upstream returned an invalid SHA-256 digest.")
                    })?
                    .to_ascii_lowercase(),
            ),
            None => previous
                .iter()
                .find(|p| {
                    p.url == asset.browser_download_url && p.size == asset.size && p.tool == tool
                })
                .map(|p| p.sha256.clone()),
        };
        let Some(hash) = hash else {
            if packages.is_empty() {
                return Err(AppError::validation(
                    "The newest upstream package has no SHA-256 digest yet. Retry later.",
                ));
            }
            // Older assets uploaded before GitHub started publishing hashes remain
            // unavailable unless their hash is already in the application's metadata.
            continue;
        };
        let format = if asset.name.ends_with(".7z") {
            "7z"
        } else if asset.name.ends_with(".zip") {
            "zip"
        } else {
            "file"
        };
        packages.push(package(
            tool,
            &release.tag_name,
            asset.browser_download_url.clone(),
            (hash, asset.size),
            format,
            release.published_at.as_deref().unwrap_or_default(),
            previous,
        ));
        if packages.len() == VERSION_COUNT {
            break;
        }
    }
    if packages.is_empty() {
        return Err(AppError::validation(
            "No supported Windows x64 release was found upstream.",
        ));
    }
    mark_latest(&mut packages)?;
    Ok(packages)
}

fn ffmpeg_branch(pattern: &regex::Regex, name: &str) -> Vec<u32> {
    pattern
        .captures(name)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .split('.')
                .filter_map(|v| v.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn package(
    tool: ToolId,
    version: &str,
    url: String,
    integrity: (String, u64),
    format: &str,
    published: &str,
    previous: &[Package],
) -> Package {
    let (hash, size) = integrity;
    let definition = provider(tool);
    let version_id: String = version
        .chars()
        .take(85)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Retain existing IDs when bytes match; a republished asset gets its own ID.
    let id = previous
        .iter()
        .find(|p| {
            p.tool == tool && p.url == url && p.sha256.eq_ignore_ascii_case(&hash) && p.size == size
        })
        .map(|p| p.id.clone())
        .unwrap_or_else(|| format!("{}-{version_id}-{}", tool.key(), &hash[..12]));
    let homepage = format!("https://github.com/{}", definition.repository);
    let mut source = url::Url::parse(&homepage).expect("fixed GitHub repository URL");
    let reference = if definition.uses_commits() {
        url.rsplit('/').next().unwrap_or(version)
    } else {
        version
    };
    source
        .path_segments_mut()
        .expect("HTTPS URL")
        .extend(["tree", reference]);
    Package {
        id,
        tool,
        version: version.into(),
        channel: "recommended".into(),
        recommended: false,
        url,
        sha256: hash,
        size,
        format: format.into(),
        entry: definition.entry.into(),
        provider: definition.repository.into(),
        homepage,
        source_url: source.into(),
        license: definition.license.into(),
        published_at: published.into(),
        dependencies: definition.dependencies.to_vec(),
    }
}

fn mark_latest(packages: &mut [Package]) -> AppResult<()> {
    for package in packages.iter_mut() {
        if timestamp(&package.published_at).is_none() {
            return Err(AppError::validation("Invalid upstream publication date."));
        }
        package.recommended = false;
    }
    // Preserve upstream ordering for equal publication timestamps.
    packages.sort_by_key(|p| std::cmp::Reverse(timestamp(&p.published_at)));
    if let Some(first) = packages.first_mut() {
        first.recommended = true;
    }
    Ok(())
}

pub fn timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn update_available(
    package: &Package,
    preference: &super::models::ToolPreference,
    installed: &[super::models::InstalledPackage],
) -> bool {
    package.recommended
        && package.channel == preference.channel
        && Some(&package.id) != preference.active.as_ref()
        && !preference.held_versions.contains(&package.id)
        && !installed
            .iter()
            .find(|i| Some(&i.package.id) == preference.active.as_ref())
            .is_some_and(|active| {
                timestamp(&active.package.published_at) > timestamp(&package.published_at)
            })
}

pub fn merge(state: &mut Registry, tool: ToolId, discovered: Discovery, checked: &str) {
    let mut discovered = discovered;
    // Keep installed identities when fresh upstream metadata describes the same
    // bytes, including installations made by builds with a bundled catalog.
    for package in &mut discovered.packages {
        if let Some(installed) = state.installed.iter().find(|installed| {
            installed.package.tool == tool
                && installed.package.url == package.url
                && installed.package.size == package.size
                && installed
                    .package
                    .sha256
                    .eq_ignore_ascii_case(&package.sha256)
        }) {
            package.id = installed.package.id.clone();
        }
    }
    let catalog = state.catalog.get_or_insert_with(Catalog::default);
    catalog.packages.retain(|p| p.tool != tool);
    catalog.packages.extend(discovered.packages);
    state.upstream_etags.remove(&tool);
    if let Some(etag) = discovered.etag {
        state.upstream_etags.insert(tool, etag);
    }
    state.upstream_checked.insert(tool, checked.into());
}

fn retry_time(
    status: StatusCode,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let header = |name| headers.get(name).and_then(|h| h.to_str().ok());
    let exhausted = header("x-ratelimit-remaining") == Some("0");
    if !exhausted
        && !matches!(
            status,
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        )
    {
        return None;
    }
    let retry = header("retry-after").and_then(|s| {
        s.parse::<i64>()
            .ok()
            .and_then(|n| chrono::Duration::try_seconds(n.max(0)))
            .and_then(|duration| now.checked_add_signed(duration))
            .or_else(|| {
                DateTime::parse_from_rfc2822(s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            })
    });
    let reset = header("x-ratelimit-reset")
        .and_then(|s| s.parse().ok())
        .and_then(|n| DateTime::from_timestamp(n, 0));
    Some(
        retry
            .into_iter()
            .chain(reset)
            .max()
            .unwrap_or(now + chrono::Duration::hours(1))
            .max(now + chrono::Duration::minutes(1)),
    )
}

async fn bounded_body(mut response: reqwest::Response, limit: usize) -> AppResult<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(AppError::validation(
            "Upstream response exceeds its size limit.",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(AppError::validation(
                "Upstream response exceeds its size limit.",
            ));
        }
        bytes.extend(chunk);
    }
    Ok(bytes)
}

fn network_error(error: reqwest::Error) -> AppError {
    AppError::validation(format!("Upstream request failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn release(tool: ToolId, tag: &str, name: &str, date: &str) -> Value {
        json!({
            "tag_name": tag, "draft": false, "prerelease": false, "published_at": date,
            "assets": [{"name": name, "browser_download_url": format!("https://github.com/{}/releases/download/{tag}/{name}", provider(tool).repository),
                "size": 123, "digest": format!("sha256:{}", "a".repeat(64)), "state": "uploaded"}]
        })
    }
    fn parse(tool: ToolId, values: &[Value], previous: &[Package]) -> AppResult<Vec<Package>> {
        releases(tool, &serde_json::to_vec(values).unwrap(), previous)
    }
    #[test]
    fn releases_exclude_previews_and_mutable_tags_and_sort_by_publication() {
        let old = release(
            ToolId::Mpv,
            "20260801",
            "mpv-x86_64-20260801-git-abcd.7z",
            "2026-08-01T00:00:00Z",
        );
        let new = release(
            ToolId::Mpv,
            "20260901",
            "mpv-x86_64-20260901-git-cdef.7z",
            "2026-09-01T00:00:00Z",
        );
        let mut preview = new.clone();
        preview["prerelease"] = true.into();
        let mut draft = new.clone();
        draft["draft"] = true.into();
        let mut latest = new.clone();
        latest["tag_name"] = "latest".into();
        let mut arm = new.clone();
        arm["assets"][0]["name"] = "mpv-aarch64-20260901-git-cdef.7z".into();
        let mut v3 = new.clone();
        v3["assets"][0]["name"] = "mpv-x86_64-v3-20260901-git-cdef.7z".into();
        let packages = parse(
            ToolId::Mpv,
            &[old, preview, draft, latest, arm, v3, new],
            &[],
        )
        .unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].version, "20260901");
        assert!(packages[0].recommended);
        assert!(!packages[1].recommended);
        catalog::validate(&Catalog { packages }).unwrap();
    }
    #[test]
    fn ffmpeg_chooses_highest_numeric_lgpl_branch() {
        let mut data = release(
            ToolId::Ffmpeg,
            "autobuild-2026-09-11",
            "ffmpeg-n9.0-1-gabc-win64-lgpl-9.0.zip",
            "2026-09-11T00:00:00Z",
        );
        for name in [
            "ffmpeg-n10.0-1-gabc-win64-lgpl-10.0.zip",
            "ffmpeg-n11.0-1-gabc-win64-gpl-11.0.zip",
            "ffmpeg-n12.0-1-gabc-winarm64-lgpl-12.0.zip",
        ] {
            let asset = release(
                ToolId::Ffmpeg,
                "autobuild-2026-09-11",
                name,
                "2026-09-11T00:00:00Z",
            )["assets"][0]
                .clone();
            data["assets"].as_array_mut().unwrap().push(asset);
        }
        let packages = parse(ToolId::Ffmpeg, &[data], &[]).unwrap();
        assert!(packages[0].url.ends_with("win64-lgpl-10.0.zip"));
        catalog::validate(&Catalog { packages }).unwrap();
    }
    #[test]
    fn digests_are_required_and_republished_bytes_get_a_new_id() {
        let data = release(
            ToolId::Ytdlp,
            "2026.09.01",
            "yt-dlp.exe",
            "2026-09-01T00:00:00Z",
        );
        let previous = parse(ToolId::Ytdlp, std::slice::from_ref(&data), &[]).unwrap();
        let unchanged = parse(ToolId::Ytdlp, std::slice::from_ref(&data), &previous).unwrap();
        assert_eq!(previous[0].id, unchanged[0].id);
        let mut changed = data.clone();
        changed["assets"][0]["digest"] = format!("sha256:{}", "b".repeat(64)).into();
        let changed = parse(ToolId::Ytdlp, &[changed], &previous).unwrap();
        assert_ne!(previous[0].id, changed[0].id);
        let mut missing = data.clone();
        missing["assets"][0]["digest"] = Value::Null;
        assert!(parse(ToolId::Ytdlp, std::slice::from_ref(&missing), &[]).is_err());
        assert!(parse(ToolId::Ytdlp, &[missing], &previous).is_ok());
        let mut invalid = data;
        invalid["assets"][0]["digest"] = "sha256:bad".into();
        assert!(parse(ToolId::Ytdlp, &[invalid], &previous).is_err());
    }
    #[test]
    fn commit_snapshots_require_full_hashes() {
        for sha in [
            "main",
            "abcdef123456",
            "../../other",
            "000000000000000000000000000000000000000g",
        ] {
            assert!(commit_url(ToolId::Thumbfast, sha).is_err());
        }
        assert!(
            commit_url(ToolId::Thumbfast, &"a".repeat(40))
                .unwrap()
                .ends_with(&"a".repeat(40))
        );
    }
    #[test]
    fn rate_limit_honors_retry_after_and_reset_including_last_success() {
        let now = timestamp("2026-09-11T12:00:00Z").unwrap();
        let mut headers = HeaderMap::new();
        assert!(retry_time(StatusCode::OK, &headers, now).is_none());
        headers.insert("retry-after", "120".parse().unwrap());
        assert_eq!(
            retry_time(StatusCode::TOO_MANY_REQUESTS, &headers, now),
            Some(now + chrono::Duration::minutes(2))
        );
        headers.insert(
            "x-ratelimit-reset",
            (now.timestamp() + 3600).to_string().parse().unwrap(),
        );
        assert_eq!(
            retry_time(StatusCode::FORBIDDEN, &headers, now),
            Some(now + chrono::Duration::hours(1))
        );
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        assert_eq!(
            retry_time(StatusCode::OK, &headers, now),
            Some(now + chrono::Duration::hours(1))
        );
        headers.clear();
        headers.insert(
            "retry-after",
            "Fri, 11 Sep 2026 12:30:00 GMT".parse().unwrap(),
        );
        assert_eq!(
            retry_time(StatusCode::TOO_MANY_REQUESTS, &headers, now),
            Some(now + chrono::Duration::minutes(30))
        );
    }
    #[test]
    fn malformed_retry_headers_do_not_panic_or_disable_backoff() {
        let now = timestamp("2026-09-11T12:00:00Z").unwrap();
        for value in ["9223372036854775807", "invalid", "-100"] {
            let mut headers = HeaderMap::new();
            headers.insert("retry-after", value.parse().unwrap());
            assert!(retry_time(StatusCode::TOO_MANY_REQUESTS, &headers, now).unwrap() > now);
        }
    }
    #[test]
    fn partial_refresh_preserves_other_tools() {
        let mut state = Registry {
            catalog: Some(Catalog {
                packages: ToolId::ALL
                    .into_iter()
                    .map(super::super::providers::test_package)
                    .collect(),
            }),
            ..Default::default()
        };
        let original = state.catalog.as_ref().unwrap().packages.clone();
        let mut mpv = original
            .iter()
            .find(|p| p.tool == ToolId::Mpv)
            .unwrap()
            .clone();
        mpv.id = "new-mpv".into();
        merge(
            &mut state,
            ToolId::Mpv,
            Discovery {
                packages: vec![mpv],
                etag: Some("test-etag".into()),
            },
            "2026-09-11T12:00:00Z",
        );
        let catalog = state.catalog.unwrap();
        assert_eq!(
            catalog
                .packages
                .iter()
                .filter(|p| p.tool == ToolId::Mpv)
                .count(),
            1
        );
        for p in original.iter().filter(|p| p.tool != ToolId::Mpv) {
            assert!(
                catalog
                    .packages
                    .iter()
                    .any(|v| p.id == v.id && p.sha256 == v.sha256)
            );
        }
        assert_eq!(state.upstream_checked.len(), 1);
        assert_eq!(state.upstream_etags[&ToolId::Mpv], "test-etag");
    }
    #[test]
    fn disappearing_newer_release_does_not_offer_an_automatic_downgrade() {
        let mut old = super::super::providers::test_package(ToolId::Mpv);
        old.published_at = "2026-08-01T00:00:00Z".into();
        let mut new = old.clone();
        new.id = "new-mpv".into();
        new.published_at = "2026-09-01T00:00:00Z".into();
        let mut preference = super::super::models::ToolPreference {
            active: Some(new.id.clone()),
            ..Default::default()
        };
        let installed = vec![super::super::models::InstalledPackage {
            package: new.clone(),
            installed_at: String::new(),
            directory: String::new(),
            version_output: String::new(),
        }];
        assert!(!update_available(&old, &preference, &installed));
        preference.active = Some(old.id);
        assert!(update_available(&new, &preference, &[]));
        preference.held_versions.push(new.id.clone());
        assert!(!update_available(&new, &preference, &[]));
    }
}
