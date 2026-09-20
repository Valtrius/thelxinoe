use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::ExecutableDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolId {
    Mpv,
    Ytdlp,
    Streamlink,
    Ffmpeg,
    Deno,
    Uosc,
    Thumbfast,
    SubSelect,
}

impl ToolId {
    pub const DESKTOP: [Self; 4] = [Self::Mpv, Self::Uosc, Self::Thumbfast, Self::SubSelect];
    pub const ALL: [Self; 8] = [
        Self::Mpv,
        Self::Ytdlp,
        Self::Streamlink,
        Self::Ffmpeg,
        Self::Deno,
        Self::Uosc,
        Self::Thumbfast,
        Self::SubSelect,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Mpv => "mpv",
            Self::Ytdlp => "yt-dlp",
            Self::Streamlink => "streamlink",
            Self::Ffmpeg => "ffmpeg",
            Self::Deno => "deno",
            Self::Uosc => "uosc",
            Self::Thumbfast => "thumbfast",
            Self::SubSelect => "sub-select",
        }
    }
    pub fn plugin(self) -> bool {
        matches!(self, Self::Uosc | Self::Thumbfast | Self::SubSelect)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolSource {
    #[default]
    System,
    Custom,
    Managed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePolicy {
    Automatic,
    #[default]
    Notify,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolPreference {
    pub source: ToolSource,
    pub custom_path: Option<String>,
    pub active: Option<String>,
    pub previous: Option<String>,
    pub pinned: bool,
    pub update_policy: UpdatePolicy,
    pub channel: String,
    pub enabled: bool,
    pub held_versions: Vec<String>,
}
impl Default for ToolPreference {
    fn default() -> Self {
        Self {
            source: ToolSource::System,
            custom_path: None,
            active: None,
            previous: None,
            pinned: false,
            update_policy: UpdatePolicy::Notify,
            channel: "recommended".into(),
            enabled: false,
            held_versions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: String,
    pub tool: ToolId,
    pub version: String,
    pub channel: String,
    pub recommended: bool,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub format: String,
    pub entry: String,
    pub provider: String,
    pub homepage: String,
    pub source_url: String,
    pub license: String,
    pub published_at: String,
    #[serde(default)]
    pub dependencies: Vec<ToolId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    pub package: Package,
    pub installed_at: String,
    pub directory: String,
    pub version_output: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MpvConfigSource {
    #[default]
    Native,
    Managed,
    Directory,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MpvPreferences {
    pub source: MpvConfigSource,
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Registry {
    pub tools: BTreeMap<ToolId, ToolPreference>,
    pub installed: Vec<InstalledPackage>,
    pub mpv: MpvPreferences,
    pub catalog: Option<Catalog>,
    pub last_checked: Option<String>,
    pub catalog_error: Option<String>,
    pub upstream_discovery: bool,
    pub upstream_etags: BTreeMap<ToolId, String>,
    pub upstream_checked: BTreeMap<ToolId, String>,
    pub upstream_retry_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub id: ToolId,
    pub preference: ToolPreference,
    pub diagnostic: Option<ExecutableDiagnostic>,
    pub checked_at: Option<String>,
    pub selected_path: Option<String>,
    pub imported_paths: Vec<String>,
    pub versions: Vec<Package>,
    pub installed: Vec<InstalledPackage>,
    pub in_use: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolOperation {
    pub tool: Option<ToolId>,
    pub package_id: Option<String>,
    pub phase: String,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUpdate {
    pub tool: ToolId,
    pub package_id: String,
    pub version: String,
    pub current_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsSnapshot {
    pub tools: Vec<ToolView>,
    pub mpv: MpvPreferences,
    pub operation: ToolOperation,
    pub last_checked: Option<String>,
    pub catalog_error: Option<String>,
    pub directory: String,
}
