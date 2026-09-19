use super::archive;
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone, Serialize, Deserialize)]
pub struct Selection {
    pub mode: String,
    pub path: String,
    pub version: String,
    pub digest: Option<String>,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            mode: "managed".into(),
            path: String::new(),
            version: String::new(),
            digest: None,
        }
    }
}
#[derive(Clone)]
pub struct ToolStore {
    pub root: PathBuf,
}
impl ToolStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("config/scripts"))?;
        Ok(Self { root })
    }
    pub fn configuration(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn selection(&self) -> Result<Selection> {
        let path = self.root.join("selection.json");
        if !path.exists() {
            return Ok(Selection::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn save(&self, value: &Selection) -> Result<()> {
        let tmp = self.root.join("selection.pending");
        std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(tmp, self.root.join("selection.json"))?;
        Ok(())
    }
    pub async fn select_custom(&self, path: PathBuf) -> Result<Selection> {
        let path = tokio::fs::canonicalize(path).await?;
        ensure!(
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
            "Choose an MPV executable"
        );
        let version = probe(&path).await?;
        let selected = Selection {
            mode: "custom".into(),
            path: path.to_string_lossy().into(),
            version,
            digest: None,
        };
        self.save(&selected)?;
        Ok(selected)
    }
    pub async fn install(&self) -> Result<Selection> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Thelxinoe/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(180))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let host = attempt.url().host_str().unwrap_or("");
                if attempt.previous().len() < 5
                    && attempt.url().scheme() == "https"
                    && [
                        "github.com",
                        "release-assets.githubusercontent.com",
                        "objects.githubusercontent.com",
                    ]
                    .contains(&host)
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        let release: serde_json::Value = client
            .get("https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            release["prerelease"] == false && !release["draft"].as_bool().unwrap_or(true),
            "No published MPV build is available"
        );
        let asset = release["assets"]
            .as_array()
            .and_then(|assets| {
                assets.iter().find(|a| {
                    a["name"].as_str().is_some_and(|n| {
                        n.strip_prefix("mpv-x86_64-")
                            .and_then(|s| s.strip_suffix(".7z"))
                            .and_then(|s| s.split_once("-git-"))
                            .is_some_and(|(date, commit)| {
                                date.len() == 8
                                    && date.bytes().all(|b| b.is_ascii_digit())
                                    && (7..=40).contains(&commit.len())
                                    && commit.bytes().all(|b| b.is_ascii_hexdigit())
                            })
                    })
                })
            })
            .context("MPV Windows x64 package is missing")?;
        let hash = asset["digest"]
            .as_str()
            .and_then(|d| d.strip_prefix("sha256:"))
            .context("Upstream package has no SHA-256 digest")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid package digest"
        );
        let size = asset["size"].as_u64().context("Package size missing")?;
        ensure!(
            size > 0 && size <= 256 * 1024 * 1024,
            "Package exceeds download limit"
        );
        let url = asset["browser_download_url"]
            .as_str()
            .context("Package URL missing")?;
        let parsed = url::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.host_str() == Some("github.com")
                && parsed
                    .path()
                    .starts_with("/shinchiro/mpv-winbuild-cmake/releases/download/"),
            "Unexpected package source"
        );
        let destination = self.root.join("versions").join(hash);
        tokio::fs::create_dir_all(self.root.join("versions")).await?;
        if !destination.exists() {
            let key = uuid::Uuid::new_v4().to_string();
            let package = self.root.join(format!("{key}.7z"));
            let staging = self.root.join(format!("staging-{key}"));
            let result = async {
                let response = client.get(url).send().await?.error_for_status()?;
                let mut stream = response.bytes_stream();
                let mut file = tokio::fs::File::create(&package).await?;
                let mut hasher = Sha256::new();
                let mut downloaded = 0;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    downloaded += chunk.len() as u64;
                    ensure!(downloaded <= size, "Download exceeds advertised size");
                    hasher.update(&chunk);
                    file.write_all(&chunk).await?;
                }
                file.sync_all().await?;
                drop(file);
                ensure!(
                    downloaded == size
                        && hasher
                            .finalize()
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                            == hash,
                    "Downloaded MPV package failed verification"
                );
                let archive_path = package.clone();
                let unpack = staging.clone();
                tokio::task::spawn_blocking(move || archive::extract(&archive_path, &unpack))
                    .await??;
                probe(&staging.join("mpv.exe")).await?;
                tokio::fs::rename(&staging, &destination).await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let _ = tokio::fs::remove_file(&package).await;
            if staging.exists() {
                let _ = tokio::fs::remove_dir_all(&staging).await;
            }
            result?;
        }
        let path = destination.join("mpv.exe");
        let selected = Selection {
            mode: "managed".into(),
            version: probe(&path).await?,
            path: path.to_string_lossy().into(),
            digest: Some(hash.into()),
        };
        self.save(&selected)?;
        Ok(selected)
    }
}
pub async fn probe(path: &Path) -> Result<String> {
    let mut command = Command::new(path);
    command
        .args(["--no-config", "--version"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    ensure!(output.status.success(), "MPV executable probe failed");
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .lines()
        .find(|s| s.starts_with("mpv "))
        .context("The selected executable did not identify as MPV")?;
    Ok(version.chars().take(200).collect())
}
