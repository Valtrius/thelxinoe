//! Official, digest-verified executable snapshots. Jobs keep their selected
//! paths; installing a newer bundle never overwrites a running executable.
use crate::{AppState, error::Result, security};
use anyhow::{Context, ensure};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use thelxinoe_core::{Capability, now};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Serialize, Deserialize)]
struct Asset {
    tool: String,
    version: String,
    name: String,
    url: String,
    digest: String,
    size: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Executable {
    pub version: String,
    pub path: PathBuf,
    pub digest: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Bundle {
    pub yt_dlp: Executable,
    pub deno: Executable,
}

pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let selected = selection(&state).await?;
    let job=state.db.call(|db|Ok(db.query_row("SELECT id,state,error FROM jobs WHERE kind='online.tools.install' ORDER BY created_at DESC,rowid DESC LIMIT 1",[],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"error":r.get::<_,Option<String>>(2)?}))).optional()?)).await?;
    Ok(Json(
        json!({"installed":selected.is_some(),"yt_dlp":selected.as_ref().map(|b|&b.yt_dlp.version),"deno":selected.as_ref().map(|b|&b.deno.version),"job":job}),
    ))
}
pub async fn request_install(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let queued = enqueue_install(&state, Some(p.user.id)).await?;
    Ok(Json(json!({"job_id":queued})))
}
async fn enqueue_install(state: &AppState, actor: Option<String>) -> anyhow::Result<String> {
    let queued=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id)=tx.query_row("SELECT id FROM jobs WHERE kind='online.tools.install' AND state IN ('queued','running') LIMIT 1",[],|r|r.get::<_,String>(0)).optional()?{return Ok(id);}
        let id=thelxinoe_core::id();
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'online.tools.install','{}',?1,'queued',?2,?2)",params![id,now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.tools.install',?2,?3)",params![actor,id,now()])?;
        tx.commit()?;Ok(id)
    }).await?;
    Ok(queued)
}
/// Playback dependencies belong to the server, not a client settings menu.
pub(super) async fn ready(state: &AppState) -> Result<Bundle> {
    if let Some(bundle) = selection(state).await? {
        return Ok(bundle);
    }
    enqueue_install(state, None).await?;
    Err(crate::error::ApiError(
        axum::http::StatusCode::CONFLICT,
        "playback_preparing",
        "The server is preparing YouTube playback. Try again shortly.".into(),
    ))
}
pub(super) async fn selection(state: &AppState) -> anyhow::Result<Option<Bundle>> {
    state
        .db
        .call(|db| {
            Ok(db
                .query_row(
                    "SELECT value FROM settings WHERE key='online.tools'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?
        .map(|v| serde_json::from_str(&v).context("Invalid managed tool selection"))
        .transpose()
}
fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("Thelxinoe/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5
                && attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()?)
}
fn asset_name(tool: &str) -> anyhow::Result<&'static str> {
    match (tool, std::env::consts::OS, std::env::consts::ARCH) {
        ("yt-dlp-module", _, _) => Ok("yt-dlp"),
        ("yt-dlp", "linux", "x86_64") => Ok("yt-dlp_linux"),
        ("yt-dlp", "linux", "aarch64") => Ok("yt-dlp_linux_aarch64"),
        ("yt-dlp", "windows", "x86_64") => Ok("yt-dlp.exe"),
        ("deno", "linux", "x86_64") => Ok("deno-x86_64-unknown-linux-gnu.zip"),
        ("deno", "linux", "aarch64") => Ok("deno-aarch64-unknown-linux-gnu.zip"),
        ("deno", "windows", "x86_64") => Ok("deno-x86_64-pc-windows-msvc.zip"),
        _ => anyhow::bail!("Managed online tools require Linux x64/arm64 or Windows x64"),
    }
}
fn repository(tool: &str) -> anyhow::Result<&'static str> {
    match tool {
        "yt-dlp" | "yt-dlp-module" => Ok("yt-dlp/yt-dlp"),
        "deno" => Ok("denoland/deno"),
        _ => anyhow::bail!("Unsupported managed tool"),
    }
}
fn valid_asset(asset: &Asset) -> bool {
    let Ok(repo) = repository(&asset.tool) else {
        return false;
    };
    let Ok(name) = asset_name(&asset.tool) else {
        return false;
    };
    !asset.version.is_empty()
        && asset.version.len() <= 40
        && asset
            .version
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        && asset.version != "."
        && asset.version != ".."
        && asset.digest.len() == 64
        && asset.digest.bytes().all(|c| c.is_ascii_hexdigit())
        && asset.size > 0
        && asset.size <= 128 * 1024 * 1024
        && asset.name == name
        && asset.url
            == format!(
                "https://github.com/{repo}/releases/download/{}/{name}",
                asset.version
            )
}
async fn latest(http: &reqwest::Client, tool: &str) -> anyhow::Result<Asset> {
    release(http, tool, "latest").await
}
async fn release(http: &reqwest::Client, tool: &str, endpoint: &str) -> anyhow::Result<Asset> {
    let response = http
        .get(format!(
            "https://api.github.com/repos/{}/releases/{endpoint}",
            repository(tool)?
        ))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Could not reach the official tool release service"))?;
    let (status, _, body) = super::bounded_response(response).await?;
    ensure!(status.is_success(), "Official tool release lookup failed");
    let value: Value = serde_json::from_slice(&body)?;
    ensure!(
        value["draft"] == false && value["prerelease"] == false,
        "Expected a published stable tool release"
    );
    let name = asset_name(tool)?;
    let asset = value["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["name"] == name))
        .context("Official release has no binary for this platform")?;
    let selected = Asset {
        tool: tool.into(),
        version: value["tag_name"].as_str().unwrap_or("").into(),
        name: name.into(),
        url: asset["browser_download_url"].as_str().unwrap_or("").into(),
        digest: asset["digest"]
            .as_str()
            .unwrap_or("")
            .strip_prefix("sha256:")
            .unwrap_or("")
            .into(),
        size: asset["size"].as_u64().unwrap_or(0),
    };
    ensure!(
        valid_asset(&selected),
        "Official tool release is missing a valid SHA-256 digest or asset address"
    );
    Ok(selected)
}
/// The official zipimport package includes the matching EJS scripts. Keep it
/// beside the managed tools, pinned to the selected CLI version, so extractor
/// updates do not require rebuilding the server's Python environment.
pub(super) async fn python_module(state: &AppState, bundle: &Bundle) -> anyhow::Result<Executable> {
    let version = &bundle.yt_dlp.version;
    ensure!(
        !version.is_empty()
            && version.len() <= 40
            && version != "."
            && version != ".."
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-')),
        "Invalid selected extractor version"
    );
    let root = state.config.state.join("tools");
    let modules = root.join("yt-dlp-module");
    tokio::fs::create_dir_all(&modules).await?;
    let manifest = modules.join(format!("{version}.json"));
    if let Ok(bytes) = tokio::fs::read(&manifest).await {
        let selected: Executable = serde_json::from_slice(&bytes)?;
        ensure!(
            selected.version == *version
                && selected.digest.len() == 64
                && selected.digest.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid extractor module snapshot"
        );
        let expected = modules
            .join(&selected.digest)
            .join("resident")
            .join(if cfg!(windows) {
                "yt-dlp.exe"
            } else {
                "yt-dlp"
            });
        ensure!(
            selected.path == tokio::fs::canonicalize(expected).await?,
            "Extractor module escaped its snapshot"
        );
        verify(&selected).await?;
        return Ok(selected);
    }
    let http = client()?;
    let asset = release(&http, "yt-dlp-module", &format!("tags/{version}")).await?;
    ensure!(
        asset.version == *version && asset.size <= 16 * 1024 * 1024,
        "Unexpected extractor module release"
    );
    let selected = install_asset(&http, &root, &asset, "resident").await?;
    let pending = manifest.with_extension("pending");
    tokio::fs::write(&pending, serde_json::to_vec(&selected)?).await?;
    tokio::fs::rename(pending, manifest).await?;
    Ok(selected)
}
pub async fn install(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let http = client()?;
    let assets: Vec<Asset> = if job.payload["assets"].is_array() {
        serde_json::from_value(job.payload["assets"].clone())?
    } else {
        let assets = vec![latest(&http, "yt-dlp").await?, latest(&http, "deno").await?];
        let payload = serde_json::to_string(&json!({"assets":assets}))?;
        let id = job.id.clone();
        state
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE jobs SET payload=?1 WHERE id=?2",
                    params![payload, id],
                )?;
                Ok(())
            })
            .await?;
        assets
    };
    ensure!(
        assets.len() == 2
            && assets[0].tool == "yt-dlp"
            && assets[1].tool == "deno"
            && assets.iter().all(valid_asset),
        "Invalid pinned tool manifest"
    );
    let root = state.config.state.join("tools");
    tokio::fs::create_dir_all(&root).await?;
    let bundle = Bundle {
        yt_dlp: install_asset(&http, &root, &assets[0], &job.id).await?,
        deno: install_asset(&http, &root, &assets[1], &job.id).await?,
    };
    for executable in [&bundle.yt_dlp, &bundle.deno] {
        verify(executable).await?;
        let result = super::process::run(
            &executable.path,
            &["--version".into()],
            Duration::from_secs(30),
            4096,
        )
        .await?;
        ensure!(
            result.success && !result.stdout.is_empty() && result.stderr.is_empty(),
            "Installed online tool failed its startup check"
        );
    }
    let value = serde_json::to_string(&bundle)?;
    state.db.call(move|db|{db.execute("INSERT INTO settings(key,value) VALUES ('online.tools',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;Ok(())}).await?;
    state.emit(None, "online.tools.changed", json!({})).await?;
    Ok(())
}
async fn install_asset(
    http: &reqwest::Client,
    root: &Path,
    asset: &Asset,
    install_id: &str,
) -> anyhow::Result<Executable> {
    let directory = root.join(&asset.tool).join(&asset.digest).join(install_id);
    tokio::fs::create_dir_all(&directory).await?;
    let manifest = directory.join("installed.json");
    if let Ok(bytes) = tokio::fs::read(&manifest).await {
        let selected: Executable = serde_json::from_slice(&bytes)?;
        ensure!(
            selected.path.parent() == Some(directory.canonicalize()?.as_path()),
            "Managed executable escaped its directory"
        );
        verify(&selected).await?;
        return Ok(selected);
    }
    ensure!(
        fs2::available_space(root)? > 512 * 1024 * 1024,
        "At least 512 MiB of free space is required to install online tools"
    );
    let pending = directory.join("asset.pending");
    let mut response = http
        .get(&asset.url)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Official tool download failed"))?;
    ensure!(
        response.status().is_success(),
        "Official tool download failed"
    );
    let mut output = tokio::fs::File::create(&pending).await?;
    let mut hash = Sha256::new();
    let mut size = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Official tool download interrupted"))?
    {
        size += chunk.len() as u64;
        ensure!(
            size <= asset.size,
            "Tool download exceeded its published size"
        );
        hash.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.sync_all().await?;
    drop(output);
    ensure!(
        size == asset.size && hex(&hash.finalize()) == asset.digest,
        "Tool download failed SHA-256 verification"
    );
    let name = if asset.tool == "deno" {
        if cfg!(windows) { "deno.exe" } else { "deno" }
    } else if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    // A crash before the manifest is published must not leave a read-only
    // extraction target that prevents this durable job from resuming.
    let executable = directory.join(name);
    let staged = directory.join("executable.pending");
    if asset.tool == "deno" {
        let source = pending.clone();
        let target = staged.clone();
        tokio::task::spawn_blocking(move || extract_deno(&source, &target, name)).await??;
        tokio::fs::remove_file(&pending).await?;
    } else {
        tokio::fs::rename(&pending, &staged).await?;
    }
    if tokio::fs::try_exists(&executable).await? {
        ensure!(
            digest(&staged).await? == digest(&executable).await?,
            "Interrupted installation contains a different executable; start a new installation"
        );
        tokio::fs::remove_file(&staged).await?;
    } else {
        tokio::fs::rename(&staged, &executable).await?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o555)).await?;
    }
    let executable = tokio::fs::canonicalize(executable).await?;
    let selected = Executable {
        version: asset.version.clone(),
        digest: digest(&executable).await?,
        path: executable,
    };
    let temporary = directory.join("installed.pending");
    tokio::fs::write(&temporary, serde_json::to_vec(&selected)?).await?;
    tokio::fs::rename(temporary, manifest).await?;
    Ok(selected)
}
fn extract_deno(source: &Path, target: &Path, name: &str) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(source)?)?;
    ensure!(archive.len() == 1, "Unexpected files in Deno archive");
    let file = archive.by_name(name)?;
    ensure!(
        !file.is_dir() && !file.is_symlink() && file.size() <= 256 * 1024 * 1024,
        "Invalid executable in Deno archive"
    );
    let mut output = std::fs::File::create(target)?;
    let expected = file.size();
    let copied = std::io::copy(
        &mut std::io::Read::take(file, 256 * 1024 * 1024 + 1),
        &mut output,
    )?;
    ensure!(
        copied == expected && copied <= 256 * 1024 * 1024,
        "Deno executable exceeds size limit"
    );
    output.sync_all()?;
    Ok(())
}
async fn digest(path: &Path) -> anyhow::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex(&hash.finalize()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) async fn verify(executable: &Executable) -> anyhow::Result<()> {
    ensure!(
        digest(&executable.path).await? == executable.digest,
        "Managed executable changed; reinstall online tools"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn concurrent_first_playback_queues_only_one_install() {
        let (_temp, state, _) = crate::online::oauth::tests::fixture().await;
        let (first, second) = tokio::join!(ready(&state), ready(&state));
        for result in [first, second] {
            let error = result
                .err()
                .expect("Playback must wait for its dependencies");
            assert_eq!(error.0, axum::http::StatusCode::CONFLICT);
            assert_eq!(error.1, "playback_preparing");
        }
        state.db.call(|db| {
            assert_eq!(db.query_row("SELECT COUNT(*) FROM jobs WHERE kind='online.tools.install' AND state='queued'", [], |r| r.get::<_, i64>(0))?, 1);
            db.execute("UPDATE jobs SET state='running' WHERE kind='online.tools.install'", [])?;
            Ok(())
        }).await.unwrap();
        assert_eq!(ready(&state).await.err().unwrap().1, "playback_preparing");
        state
            .db
            .call(|db| {
                assert_eq!(
                    db.query_row(
                        "SELECT COUNT(*) FROM jobs WHERE kind='online.tools.install'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    1
                );
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn changed_executable_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tool");
        tokio::fs::write(&path, b"original").await.unwrap();
        let executable = Executable {
            version: "test".into(),
            digest: digest(&path).await.unwrap(),
            path: path.clone(),
        };
        verify(&executable).await.unwrap();
        tokio::fs::write(path, b"replacement").await.unwrap();
        assert!(verify(&executable).await.is_err());
    }

    #[test]
    fn archive_paths_and_extra_files_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("asset.zip");
        let output = root.path().join("deno");
        for names in [vec!["../deno"], vec!["deno", "extra"], vec!["deno"]] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            for name in &names {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"executable").unwrap();
            }
            zip.finish().unwrap();
            assert_eq!(
                extract_deno(&archive, &output, "deno").is_ok(),
                names == vec!["deno"]
            );
        }
        assert_eq!(std::fs::read(output).unwrap(), b"executable");
    }

    #[test]
    fn release_descriptors_cannot_redirect_to_other_sources() {
        let name = asset_name("yt-dlp").unwrap();
        let mut asset = Asset {
            tool: "yt-dlp".into(),
            version: "2026.08.19".into(),
            name: name.into(),
            url: format!("https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/{name}"),
            digest: "a".repeat(64),
            size: 123,
        };
        assert!(valid_asset(&asset));
        asset.url = "https://example.com/yt-dlp".into();
        assert!(!valid_asset(&asset));
        asset.version = "../escape".into();
        assert!(!valid_asset(&asset));
    }

    #[tokio::test]
    async fn resident_module_is_pinned_and_verified_without_network() {
        let (_temp, state, _) = crate::online::oauth::tests::fixture().await;
        let bytes = b"module fixture";
        let hash = hex(&Sha256::digest(bytes));
        let root = state.config.state.join("tools/yt-dlp-module");
        let dir = root.join(&hash).join("resident");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        });
        tokio::fs::write(&path, bytes).await.unwrap();
        let selected = Executable {
            version: "2026.08.19".into(),
            path: tokio::fs::canonicalize(&path).await.unwrap(),
            digest: hash,
        };
        let manifest = root.join("2026.08.19.json");
        tokio::fs::write(&manifest, serde_json::to_vec(&selected).unwrap())
            .await
            .unwrap();
        let bundle = Bundle {
            yt_dlp: selected.clone(),
            deno: selected.clone(),
        };
        assert!(python_module(&state, &bundle).await.unwrap() == selected);
        let mut wrong_version = selected.clone();
        wrong_version.version = "2025.01.01".into();
        tokio::fs::write(&manifest, serde_json::to_vec(&wrong_version).unwrap())
            .await
            .unwrap();
        assert!(python_module(&state, &bundle).await.is_err());
        tokio::fs::write(&manifest, serde_json::to_vec(&selected).unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, b"replacement").await.unwrap();
        assert!(python_module(&state, &bundle).await.is_err());
        let mut traversal = bundle;
        traversal.yt_dlp.version = "../escape".into();
        assert!(python_module(&state, &traversal).await.is_err());
    }
}
