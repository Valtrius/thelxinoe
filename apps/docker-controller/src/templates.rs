//! Curated, stable service definitions. The server cannot supply arbitrary images or Docker specs.
use serde::Serialize;
#[derive(Clone, Copy, Serialize)]
pub struct Template {
    pub kind: &'static str,
    pub repository: &'static str,
    pub port: Option<u16>,
    pub workload: &'static str,
    pub tag: &'static str,
    pub media: bool,
    pub digest: &'static str,
}
pub const TEMPLATES: [Template; 8] = [
    Template {
        kind: "recyclarr",
        repository: "ghcr.io/recyclarr/recyclarr",
        port: None,
        workload: "job",
        tag: "8",
        media: false,
        digest: "sha256:6e69e009e1cd7493ff6093e8e187b5d3788c75b4a2c0c5127b6a1beda1c19728",
    },
    Template {
        kind: "seerr",
        repository: "ghcr.io/seerr-team/seerr",
        port: Some(5055),
        workload: "daemon",
        tag: "latest",
        media: false,
        digest: "sha256:f4768de5f616248d723e05891f3345a1402123775d03bf0890dbfedc0831bda1",
    },
    Template {
        kind: "radarr",
        repository: "lscr.io/linuxserver/radarr",
        port: Some(7878),
        workload: "daemon",
        tag: "latest",
        media: true,
        digest: "sha256:c960f2b52ec6542dbe6707c5a21e696a7c74fd8b17997454f4d10a55dacee133",
    },
    Template {
        kind: "sonarr",
        repository: "lscr.io/linuxserver/sonarr",
        port: Some(8989),
        workload: "daemon",
        tag: "latest",
        media: true,
        digest: "sha256:a5c1a5fecbef946927ab90ad68df319ac5fe644057e5fc18cd993f01ac07b2b2",
    },
    Template {
        kind: "lidarr",
        repository: "lscr.io/linuxserver/lidarr",
        port: Some(8686),
        workload: "daemon",
        tag: "latest",
        media: true,
        digest: "sha256:8ab0fd370b604ae034d9a9c261a9d8d873bece33d9736852e7ce4f3566e4a35d",
    },
    Template {
        kind: "bazarr",
        repository: "lscr.io/linuxserver/bazarr",
        port: Some(6767),
        workload: "daemon",
        tag: "latest",
        media: true,
        digest: "sha256:d24bd0048c759a468970989e9df11a6b96a7628d556d00f923e60a35ba59237b",
    },
    Template {
        kind: "prowlarr",
        repository: "lscr.io/linuxserver/prowlarr",
        port: Some(9696),
        workload: "daemon",
        tag: "latest",
        media: false,
        digest: "sha256:c96b56d94d116a9f4de94bc23d3381689492e6c3cfb7435320e8d982e406f99a",
    },
    Template {
        kind: "nzbget",
        repository: "lscr.io/linuxserver/nzbget",
        port: Some(6789),
        workload: "daemon",
        tag: "latest",
        media: true,
        digest: "sha256:3b92679543623c8710fec557de7dd525d55bb5dfc8c59526f054123eaae0f995",
    },
];
pub fn find(kind: &str) -> Option<Template> {
    TEMPLATES.iter().find(|t| t.kind == kind).copied()
}

/// Repository references whose images are byte-identical to the curated repository.
pub fn sources(t: Template) -> Vec<String> {
    let mut sources = vec![t.repository.to_owned()];
    if let Some(short) = t.repository.strip_prefix("lscr.io/") {
        // LinuxServer publishes the same images to Docker Hub and GHCR.
        sources.extend([
            short.to_owned(),
            format!("docker.io/{short}"),
            format!("ghcr.io/{short}"),
        ]);
    }
    if t.kind == "seerr" {
        // Seerr mirrors its GHCR images to Docker Hub with identical digests.
        sources.extend(["seerr/seerr".to_owned(), "docker.io/seerr/seerr".to_owned()]);
    }
    sources
}

pub(crate) fn pinned_image(template: Template) -> crate::docker::Result<Option<String>> {
    let Ok(raw) = std::env::var("THELXINOE_CURATED_IMAGES") else {
        return Ok(None);
    };
    let invalid = || -> crate::docker::Failure {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Invalid curated image manifest".into(),
        )
    };
    let manifest: serde_json::Value = serde_json::from_str(&raw).map_err(|_| invalid())?;
    let entries = manifest.as_object().ok_or_else(invalid)?;
    if entries.len() != TEMPLATES.len() {
        return Err(invalid());
    }
    for (kind, value) in entries {
        let curated = find(kind).ok_or_else(invalid)?;
        let reference = value.as_str().ok_or_else(invalid)?;
        let (repository, digest) = reference.split_once('@').ok_or_else(invalid)?;
        if repository != curated.repository
            || !digest.starts_with("sha256:")
            || digest.len() != 71
            || !digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
    }
    Ok(Some(
        entries[template.kind]
            .as_str()
            .ok_or_else(invalid)?
            .to_owned(),
    ))
}

pub(crate) fn configured() -> crate::docker::Result<serde_json::Value> {
    let mut items = Vec::new();
    for template in TEMPLATES {
        let mut value = serde_json::to_value(template).map_err(|_| crate::docker::unavailable())?;
        if let Some(image) = pinned_image(template)? {
            value["digest"] = serde_json::json!(
                image
                    .split_once('@')
                    .ok_or_else(crate::docker::unavailable)?
                    .1
            );
        }
        items.push(value);
    }
    Ok(serde_json::json!({"items": items}))
}
