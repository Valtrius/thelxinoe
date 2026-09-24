use serde::{Deserialize, Serialize};

/// Display metadata for one immutable service image; never used to select an update.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceRelease {
    pub image: String,
    pub version: Option<String>,
    pub build_version: Option<String>,
    pub release_notes_url: Option<String>,
}
