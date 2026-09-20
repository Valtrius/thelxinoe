use super::{
    models::{Catalog, Package},
    providers::provider,
};
use crate::error::{AppError, AppResult};

pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;

pub fn validate(catalog: &Catalog) -> AppResult<()> {
    let mut ids = std::collections::HashSet::new();
    for package in &catalog.packages {
        if !ids.insert(&package.id) {
            return Err(AppError::validation("Duplicate tool package identifier."));
        }
        validate_package(package)?;
    }
    Ok(())
}

pub fn validate_package(p: &Package) -> AppResult<()> {
    if !safe_id(&p.id) || !valid_hash(&p.sha256) || p.size == 0 || p.size > MAX_PACKAGE_BYTES {
        return Err(AppError::validation("Invalid tool package metadata."));
    }
    let definition = provider(p.tool);
    let url = allowed_url(&p.url)?;
    let expected = if definition.uses_commits() {
        format!("/{}/zip/", definition.repository)
    } else {
        format!("/{}/releases/download/", definition.repository)
    };
    let suffix = url.path().strip_prefix(&expected).unwrap_or_default();
    let approved = if definition.uses_commits() {
        url.host_str() == Some("codeload.github.com")
            && suffix.len() == 40
            && suffix.bytes().all(|c| c.is_ascii_hexdigit())
            && p.format == "zip"
    } else {
        let parts: Vec<_> = suffix.split('/').collect();
        url.host_str() == Some("github.com")
            && parts.len() == 2
            && !parts[0].is_empty()
            && !matches!(parts[0], "latest" | "nightly")
            && regex::Regex::new(definition.asset_pattern)
                .map_err(|e| AppError::internal(e.to_string()))?
                .is_match(parts[1])
            && p.format
                == if parts[1].ends_with(".7z") {
                    "7z"
                } else if parts[1].ends_with(".zip") {
                    "zip"
                } else {
                    "file"
                }
    };
    let entry = super::archive::safe_relative(&p.entry)?;
    if !approved
        || !entry.ends_with(definition.entry)
        || p.provider != definition.repository
        || p.dependencies != definition.dependencies
        || p.channel != "recommended"
        || chrono::DateTime::parse_from_rfc3339(&p.published_at).is_err()
    {
        return Err(AppError::validation(
            "Package does not match this tool's upstream installation rules.",
        ));
    }
    Ok(())
}

pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 150
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !id.contains("..")
}

pub fn allowed_url(raw: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(raw).map_err(|_| AppError::validation("Invalid package URL."))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.host_str(), Some("github.com" | "codeload.github.com"))
    {
        return Err(AppError::validation(
            "The package URL is not an approved HTTPS source.",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_repository_architecture_hash_and_paths() {
        let package = super::super::providers::test_package(super::super::models::ToolId::Mpv);
        for bad in [
            package
                .url
                .replace("shinchiro/mpv-winbuild-cmake", "someone/other"),
            package.url.replace("x86_64-", "x86_64-v3-"),
            package.url.replace("github.com", "github.com.evil.test"),
            format!("{}?redirect=other", package.url),
        ] {
            let mut p = package.clone();
            p.url = bad;
            assert!(validate_package(&p).is_err());
        }
        let mut p = package.clone();
        p.sha256 = "bad".into();
        assert!(validate_package(&p).is_err());
        let mut p = package;
        p.entry = "../mpv.exe".into();
        assert!(validate_package(&p).is_err());
        assert!(!safe_id("../mpv"));
        assert!(!safe_id("C:mpv"));
    }
}
