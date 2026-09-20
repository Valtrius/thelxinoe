mod documents;
use documents::*;
mod probe;
mod runtime;
mod schema;
use super::{
    ToolManager, archive,
    models::{MpvConfigSource, MpvPreferences, Registry, ToolId},
    process::ProcessJob,
};
use crate::{
    error::{AppError, AppResult},
    models::AppSettings,
};
use probe::Probe;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    process::{Child, Command},
};

const MAX_TEXT: u64 = 1024 * 1024;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDocument {
    pub name: String,
    pub text: String,
    pub revision: String,
    pub has_backup: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvSchema {
    pub executable: String,
    pub version: String,
    pub options: Vec<Value>,
}

impl ToolManager {
    fn config_root(&self) -> PathBuf {
        self.root.join("config").join("mpv")
    }

    pub(super) fn plugin_script_names(id: ToolId) -> [String; 3] {
        [
            id.key().into(),
            format!("{}.lua", id.key()),
            format!("{}.js", id.key()),
        ]
    }

    pub(super) fn imported_plugin_paths(&self, id: ToolId) -> Vec<String> {
        Self::plugin_script_names(id)
            .into_iter()
            .map(|name| self.config_root().join("scripts").join(name))
            .filter(|path| path.exists())
            .map(|path| path.display().to_string())
            .collect()
    }

    pub fn mpv_configuration_directory(&self) -> AppResult<PathBuf> {
        let state = self.state();
        match state.mpv.source {
            MpvConfigSource::Managed => {
                let path = self.config_root();
                fs::create_dir_all(&path)?;
                Ok(path)
            }
            MpvConfigSource::Directory => state
                .mpv
                .directory
                .map(PathBuf::from)
                .filter(|path| path.is_dir())
                .ok_or_else(|| {
                    AppError::validation("The selected configuration folder is missing.")
                }),
            MpvConfigSource::Native => Err(AppError::validation(
                "MPV discovers its normal configuration. Select a folder to open it here.",
            )),
        }
    }

    pub async fn set_mpv_preferences(&self, preferences: MpvPreferences) -> AppResult<()> {
        let _guard = self.configuration.lock().await;
        if preferences.source == MpvConfigSource::Directory
            && preferences
                .directory
                .as_deref()
                .is_none_or(|p| !Path::new(p).is_absolute() || !Path::new(p).is_dir())
        {
            return Err(AppError::validation(
                "Choose an existing MPV configuration directory.",
            ));
        }
        fs::create_dir_all(self.config_root())?;
        self.edit(|s| {
            s.mpv = preferences;
            Ok(())
        })
    }
}
