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
