use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const API_VERSION: u32 = 1;

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regular_users_cannot_mutate_shared_resources() {
        for cap in [
            Capability::ManageUsers,
            Capability::ManageLibrary,
            Capability::ManageServer,
            Capability::InspectHistory,
        ] {
            assert!(!Role::User.allows(cap));
            assert!(Role::Admin.allows(cap));
        }
        assert!(Role::User.allows(Capability::Play));
    }
}
