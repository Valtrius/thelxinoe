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

#[cfg(windows)]
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
        match executable_version(kind, path).await {
            Ok(version) => {
                if let Err(error) = validate_executable_version(kind, &version) {
                    last_error = Some(error);
                    continue;
                }
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
        "yt-dlp" => "yt-dlp was not found or could not be started.",
        "streamlink" => "Streamlink was not found or could not be started.",
        "ffmpeg" => "FFmpeg was not found or could not be started.",
        _ => "The playback executable was not found or could not be started.",
    };
    let action = match kind {
        "mpv" => "Install MPV or select mpv.exe in MPV settings.",
        "yt-dlp" => "Install yt-dlp or select yt-dlp.exe in MPV settings.",
        "streamlink" => "Install Streamlink or select streamlink.exe in MPV settings.",
        "ffmpeg" => "Install FFmpeg or select ffmpeg.exe in MPV settings.",
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

fn validate_executable_version(kind: &str, version: &str) -> Result<(), AppError> {
    if kind != "streamlink" {
        return Ok(());
    }
    let version_number = version
        .split_whitespace()
        .find(|part| {
            part.chars()
                .next()
                .is_some_and(|value| value.is_ascii_digit())
        })
        .ok_or_else(|| unsupported_streamlink_version(version))?;
    let mut parts = version_number.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| unsupported_streamlink_version(version))?;
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| unsupported_streamlink_version(version))?;
    if (major, minor) < (7, 5) {
        return Err(unsupported_streamlink_version(version));
    }
    Ok(())
}

fn unsupported_streamlink_version(version: &str) -> AppError {
    AppError::playback(
        "streamlink_version_unsupported",
        "Streamlink 7.5 or newer is required for Twitch playback.",
        format!(
            "Update Streamlink so its Twitch plugin always filters embedded ads. Detected: {version}"
        ),
    )
}

async fn executable_version(kind: &str, path: &Path) -> Result<String, AppError> {
    let mut command = Command::new(path);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .arg(if matches!(kind, "ffmpeg" | "ffprobe") {
            "-version"
        } else {
            "--version"
        })
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
        "yt-dlp" => &["yt-dlp.exe", "yt-dlp"],
        "streamlink" => &["streamlink.exe", "streamlink"],
        "ffmpeg" => &["ffmpeg.exe", "ffmpeg"],
        "ffprobe" => &["ffprobe.exe"],
        "deno" => &["deno.exe"],
        _ => &[],
    }
}

fn common_paths(kind: &str) -> Vec<PathBuf> {
    let executable = match kind {
        "mpv" => "mpv.exe",
        "yt-dlp" => "yt-dlp.exe",
        "streamlink" => "streamlink.exe",
        "ffmpeg" => "ffmpeg.exe",
        _ => return Vec::new(),
    };
    let mut paths = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        let base = PathBuf::from(program_files);
        if kind == "streamlink" {
            paths.push(base.join("Streamlink").join("bin").join(executable));
            paths.push(base.join("Streamlink").join(executable));
        } else if kind == "ffmpeg" {
            paths.push(base.join("ffmpeg").join("bin").join(executable));
        } else {
            paths.push(base.join(kind).join(executable));
        }
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data).join("Programs");
        if kind == "streamlink" {
            paths.push(base.join("Streamlink").join("bin").join(executable));
            paths.push(base.join("Streamlink").join(executable));
        } else if kind == "ffmpeg" {
            paths.push(base.join("ffmpeg").join("bin").join(executable));
        } else {
            paths.push(base.join(kind).join(executable));
        }
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let current = PathBuf::from(user_profile)
            .join("scoop")
            .join("apps")
            .join(kind)
            .join("current");
        if kind == "ffmpeg" {
            paths.push(current.join("bin").join(executable));
        } else {
            paths.push(current.join(executable));
        }
    }
    if let Some(chocolatey) = std::env::var_os("ChocolateyInstall") {
        paths.push(PathBuf::from(chocolatey).join("bin").join(executable));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::validate_executable_version;

    #[test]
    fn streamlink_requires_automatic_ad_filtering_release() {
        assert!(validate_executable_version("streamlink", "streamlink 8.4.0").is_ok());
        assert!(validate_executable_version("streamlink", "streamlink 7.5.0").is_ok());
        assert!(validate_executable_version("streamlink", "streamlink 7.4.0").is_err());
        assert!(validate_executable_version("streamlink", "unknown").is_err());
    }
}
