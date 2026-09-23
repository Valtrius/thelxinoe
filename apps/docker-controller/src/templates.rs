//! Curated, stable service definitions. The server cannot supply arbitrary images or Docker specs.
use serde::Serialize;
#[derive(Clone, Copy, Serialize)]
pub struct Template {
    pub kind: &'static str,
    pub repository: &'static str,
    pub port: u16,
    pub media: bool,
    pub digest: &'static str,
}
pub const TEMPLATES: [Template; 7] = [
    Template {
        kind: "seerr",
        repository: "ghcr.io/seerr-team/seerr",
        port: 5055,
        media: false,
        digest: "sha256:f4768de5f616248d723e05891f3345a1402123775d03bf0890dbfedc0831bda1",
    },
    Template {
        kind: "radarr",
        repository: "lscr.io/linuxserver/radarr",
        port: 7878,
        media: true,
        digest: "sha256:c960f2b52ec6542dbe6707c5a21e696a7c74fd8b17997454f4d10a55dacee133",
    },
    Template {
        kind: "sonarr",
        repository: "lscr.io/linuxserver/sonarr",
        port: 8989,
        media: true,
        digest: "sha256:a5c1a5fecbef946927ab90ad68df319ac5fe644057e5fc18cd993f01ac07b2b2",
    },
    Template {
        kind: "lidarr",
        repository: "lscr.io/linuxserver/lidarr",
        port: 8686,
        media: true,
        digest: "sha256:8ab0fd370b604ae034d9a9c261a9d8d873bece33d9736852e7ce4f3566e4a35d",
    },
    Template {
        kind: "bazarr",
        repository: "lscr.io/linuxserver/bazarr",
        port: 6767,
        media: true,
        digest: "sha256:d24bd0048c759a468970989e9df11a6b96a7628d556d00f923e60a35ba59237b",
    },
    Template {
        kind: "prowlarr",
        repository: "lscr.io/linuxserver/prowlarr",
        port: 9696,
        media: false,
        digest: "sha256:c96b56d94d116a9f4de94bc23d3381689492e6c3cfb7435320e8d982e406f99a",
    },
    Template {
        kind: "nzbget",
        repository: "lscr.io/linuxserver/nzbget",
        port: 6789,
        media: true,
        digest: "sha256:3b92679543623c8710fec557de7dd525d55bb5dfc8c59526f054123eaae0f995",
    },
];
pub fn find(kind: &str) -> Option<Template> {
    TEMPLATES.iter().find(|t| t.kind == kind).copied()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_curated_immutable_services_are_accepted() {
        assert!(find("docker").is_none());
        assert!(find("radarr:develop").is_none());
        for t in TEMPLATES {
            assert!(
                t.repository.starts_with("lscr.io/linuxserver/")
                    || t.repository == "ghcr.io/seerr-team/seerr"
            );
            assert_eq!(t.digest.len(), 71);
            assert!(t.digest[7..].bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}
