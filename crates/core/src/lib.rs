use serde::{Deserialize, Serialize};

pub mod operation_locks;
pub mod resource_locks;
pub mod service_release;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const API_VERSION: u32 = 1;

pub fn service_gateway_auth(kind: &str) -> bool {
    matches!(kind, "radarr" | "sonarr" | "lidarr" | "prowlarr")
}

/// Native UI mounts for newly provisioned services. Attached services retain
/// their explicitly configured URL base.
pub fn service_url_base(kind: &str) -> &'static str {
    match kind {
        "radarr" => "/services/radarr",
        "sonarr" => "/services/sonarr",
        "lidarr" => "/services/lidarr",
        "prowlarr" => "/services/prowlarr",
        "bazarr" => "/services/bazarr",
        _ => "",
    }
}

#[derive(Clone, Copy, Default, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NzbgetAdoptionFixes {
    #[serde(default)]
    pub rotate_logs: bool,
    #[serde(default)]
    pub cert_check: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Browse,
    Play,
    ManageOwnState,
    ManageUsers,
    ManageLibrary,
    ManageServer,
    InspectHistory,
}

impl Role {
    pub fn allows(self, capability: Capability) -> bool {
        self == Self::Admin
            || matches!(
                capability,
                Capability::Browse | Capability::Play | Capability::ManageOwnState
            )
    }
    pub fn as_str(self) -> &'static str {
        if self == Self::Admin { "admin" } else { "user" }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub timezone: String,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub user: User,
    pub session_id: String,
    pub transport: String,
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub mod activity;
