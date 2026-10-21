use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::process::Command;

use crate::error::AppError;
use crate::models::ExecutableDiagnostic;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub async fn detect_named_executable(kind: &str, configured: Option<&str>) -> ExecutableDiagnostic {
    detect_candidates(kind, configured, &executable_candidates(kind, configured)).await
}

pub(crate) fn executable_candidates(
    kind: &str,
    configured: Option<&str>,
) -> Vec<(PathBuf, String)> {
    let mut candidates = Vec::<(PathBuf, String)>::new();
    if let Some(configured) = configured.filter(|value| !value.trim().is_empty()) {
        candidates.push((PathBuf::from(configured), "configured".to_string()));
    }
    if candidates.is_empty() {
        for executable in executable_names(kind) {
            if let Ok(path) = which::which(executable) {
                candidates.push((path, "PATH".to_string()));
            }
        }
        candidates.extend(
            common_paths(kind)
                .into_iter()
                .map(|path| (path, "common Windows path".to_string())),
        );
    }

    candidates
}

pub(crate) async fn detect_candidates(
    kind: &str,
    configured: Option<&str>,
    candidates: &[(PathBuf, String)],
) -> ExecutableDiagnostic {
    let mut seen = HashSet::<OsString>::new();
    let mut last_error = None;
    for (path, source) in candidates {
        let key = path.as_os_str().to_os_string();
        if !seen.insert(key) || !path.is_file() {
            continue;
        }
        match executable_version(path).await {
            Ok(version) => {
                return ExecutableDiagnostic {
                    kind: kind.to_string(),
                    detected: true,
                    path: Some(path.display().to_string()),
                    version: Some(version),
                    source: Some(source.clone()),
                    warning: None,
                    update_available: false,
                    error: None,
                };
            }
            Err(error) => last_error = Some(error),
        }
    }
    let message = match kind {
        "mpv" => "MPV was not found or could not be started.",
        _ => "The playback executable was not found or could not be started.",
    };
    let action = match kind {
        "mpv" => "Install MPV or select mpv.exe in MPV settings.",
        _ => "Install or select the executable in MPV settings.",
    };
    ExecutableDiagnostic {
        kind: kind.to_string(),
        detected: false,
        path: configured.map(str::to_string),
        version: None,
        source: None,
        warning: None,
        update_available: false,
        error: Some(
            last_error
                .unwrap_or_else(|| AppError::playback(format!("{kind}_missing"), message, action)),
        ),
    }
}

async fn executable_version(path: &Path) -> Result<String, AppError> {
    let mut command = Command::new(path);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .map_err(|_| {
            AppError::playback(
                "executable_timeout",
                "The executable did not answer its version check.",
                "Check the selected executable and retry.",
            )
        })?
        .map_err(|error| {
            AppError::playback(
                "executable_failed",
                "The selected executable could not be started.",
                format!("Check the file and retry. {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(AppError::playback(
            "executable_rejected",
            "The selected executable rejected its version check.",
            format!(
                "Select the correct executable. Exit code: {}",
                output.status
            ),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().chars().take(160).collect())
        .ok_or_else(|| {
            AppError::playback(
                "executable_version_missing",
                "The selected executable returned no version text.",
                "Select the correct executable.",
            )
        })
}

fn executable_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "mpv" => &["mpv.exe", "mpv"],
        _ => &[],
    }
}

fn common_paths(kind: &str) -> Vec<PathBuf> {
    let executable = match kind {
        "mpv" => "mpv.exe",
        _ => return Vec::new(),
    };
    let mut paths = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(program_files).join(kind).join(executable));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        paths.push(
            PathBuf::from(local_app_data)
                .join("Programs")
                .join(kind)
                .join(executable),
        );
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        paths.push(
            PathBuf::from(user_profile)
                .join("scoop")
                .join("apps")
                .join(kind)
                .join("current")
                .join(executable),
        );
    }
    if let Some(chocolatey) = std::env::var_os("ChocolateyInstall") {
        paths.push(PathBuf::from(chocolatey).join("bin").join(executable));
    }
    paths
}
