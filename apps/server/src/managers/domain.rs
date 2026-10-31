use super::bindings;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Target {
    pub id: String,
    pub generation: String,
    pub path: String,
    pub root: String,
    pub size: u64,
    pub modified: String,
    pub fingerprint: String,
    pub ownership: String,
    pub claims: Vec<bindings::Claim>,
}

pub(super) struct ValidatedSelection {
    pub files: Vec<Target>,
}

pub(super) struct PreparedOperation {
    pub id: String,
    pub actor: Option<String>,
    pub media_id: String,
    pub action: MediaAction,
    pub selection: ValidatedSelection,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum MediaAction {
    Delete,
    Unmonitor,
    Monitor,
}
impl MediaAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Unmonitor => "unmonitor",
            Self::Monitor => "monitor",
        }
    }
}

/// What deleting a movie's files also does to its Radarr movie.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub(super) struct RadarrOptions {
    /// Remove the movie from Radarr once its files are gone.
    pub remove: bool,
    pub exclude: bool,
}
impl RadarrOptions {
    pub fn any(self) -> bool {
        self.remove || self.exclude
    }
}

#[derive(Deserialize, Serialize, Clone)]
pub(super) struct RetentionPolicy {
    pub enabled: bool,
    pub grace_seconds: i64,
    pub exclude_specials: bool,
    pub trigger_users: Vec<String>,
    #[serde(default = "default_video_limit")]
    pub storage_limit_bytes: i64,
    /// Add each deleted movie to Radarr's exclusion list.
    #[serde(default = "default_exclude")]
    pub exclude_from_lists: bool,
    /// Remove each deleted movie from Radarr.
    #[serde(default)]
    pub remove_from_manager: bool,
}
fn default_video_limit() -> i64 {
    100_000_000_000
}
fn default_exclude() -> bool {
    true
}

#[derive(Clone)]
pub(super) struct RetentionEligibility {
    pub stamp: String,
    pub user: String,
    pub users: Vec<String>,
    pub grace: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum RequestAction {
    Approve,
    Deny,
    Cancel,
    Reacquire,
}
impl RequestAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
            Self::Cancel => "cancel",
            Self::Reacquire => "reacquire",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum RequestState {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Adding,
    Searching,
    Requested,
    Available,
    Failed,
    Uncertain,
}
impl RequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::Adding => "adding",
            Self::Searching => "searching",
            Self::Requested => "requested",
            Self::Available => "available",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
        }
    }
}
pub(super) struct CreateAcquisition {
    pub user_id: String,
    pub administrator: bool,
    pub service_id: String,
    pub service_generation: String,
    pub external_id: String,
    pub title: String,
}
pub(super) struct DecideAcquisition {
    pub id: String,
    pub actor: String,
    pub administrator: bool,
    pub action: RequestAction,
}
pub(super) struct RequestReceipt {
    pub id: String,
    pub state: RequestState,
}
pub(super) struct AcquisitionRequest {
    pub service_id: String,
    pub service_generation: String,
    pub external_id: String,
    pub state: RequestState,
}
