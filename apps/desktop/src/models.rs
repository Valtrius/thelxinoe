use crate::error::AppError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Default)]
pub struct AppSettings {
    pub mpv_path: Option<String>,
    pub ytdlp_path: Option<String>,
    pub streamlink_path: Option<String>,
    pub ffmpeg_path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableDiagnostic {
    pub kind: String,
    pub detected: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub warning: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    pub error: Option<AppError>,
}
