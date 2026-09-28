use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        (2..=3).contains(&args.len()),
        "Usage: verify-release envelope.json release.pub [installer.exe]"
    );
    let bytes = std::fs::read(&args[0])?;
    anyhow::ensure!(
        bytes.len() <= thelxinoe_releases::MAX_ENVELOPE,
        "Release envelope is too large"
    );
    let envelope = serde_json::from_slice(&bytes).context("Invalid release envelope")?;
    let manifest = thelxinoe_releases::verify(&envelope, &std::fs::read_to_string(&args[1])?)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    manifest.valid_at(now)?;
    if let Some(path) = args.get(2) {
        let artifact = std::fs::read(path)?;
        anyhow::ensure!(
            artifact.len() as u64 == manifest.windows_x64.bytes,
            "Installer size mismatch"
        );
        let hash = Sha256::digest(&artifact)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        anyhow::ensure!(
            hash == manifest.windows_x64.sha256,
            "Installer hash mismatch"
        );
        let public = String::from_utf8(STANDARD.decode(&manifest.windows_x64.updater_public_key)?)?;
        let signature = String::from_utf8(STANDARD.decode(&manifest.windows_x64.signature)?)?;
        minisign_verify::PublicKey::decode(&public)?.verify(
            &artifact,
            &minisign_verify::Signature::decode(&signature)?,
            true,
        )?;
    }
    println!("Verified signed release {}", manifest.version);
    Ok(())
}
