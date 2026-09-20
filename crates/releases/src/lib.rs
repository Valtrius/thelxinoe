//! Signed product releases shared by the server, controller and desktop.
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const FORMAT: u32 = 1;
pub const RECOVERY_PROTOCOL: u32 = 1;
pub const MAX_ENVELOPE: usize = 128 * 1024;
const DOMAIN: &[u8] = b"Thelxinoe release manifest v1\0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub payload: String,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    /// Registry reference pinned to its OCI manifest digest.
    pub reference: String,
    /// Platform-specific image configuration digest checked after pulling.
    pub config_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub signature: String,
    pub updater_public_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}
impl Range {
    pub fn contains(&self, value: u32) -> bool {
        (self.min..=self.max).contains(&value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryMode {
    InPlace,
    FullStateRestore,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub from: Range,
    pub target: u32,
    pub recovery: RecoveryMode,
    pub recovery_protocol: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub published_at: i64,
    pub expires_at: i64,
    pub server: Image,
    pub controller: Image,
    pub windows_x64: Desktop,
    pub api: Range,
    pub migration: Migration,
    pub notes: String,
}
pub fn digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn https(value: &str) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Release URLs require HTTPS without credentials or fragments"
    );
    Ok(url)
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == FORMAT, "Unsupported release manifest format");
        let version = semver::Version::parse(&self.version)?;
        ensure!(
            version.pre.is_empty() && version.build.is_empty(),
            "Only stable releases are supported"
        );
        ensure!(
            self.published_at > 0 && self.expires_at > self.published_at,
            "Invalid release validity period"
        );
        ensure!(
            self.api.min > 0 && self.api.min <= self.api.max,
            "Invalid API compatibility range"
        );
        ensure!(
            self.migration.from.min <= self.migration.from.max
                && self.migration.target >= self.migration.from.max,
            "Invalid migration range"
        );
        ensure!(
            self.migration.recovery_protocol > 0,
            "Missing recovery protocol"
        );
        for image in [&self.server, &self.controller] {
            let (repo, hash) = image
                .reference
                .split_once('@')
                .ok_or_else(|| anyhow::anyhow!("Image must have an immutable registry digest"))?;
            ensure!(
                !repo.is_empty()
                    && repo.len() < 256
                    && repo.contains('/')
                    && repo.bytes().all(|b| b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b"./:-_".contains(&b))
                    && digest(hash)
                    && digest(&image.config_digest),
                "Invalid immutable image reference"
            );
        }
        https(&self.windows_x64.url)?;
        ensure!(
            digest(&format!("sha256:{}", self.windows_x64.sha256))
                && (1..=2_000_000_000).contains(&self.windows_x64.bytes),
            "Invalid desktop artifact identity"
        );
        ensure!(
            !self.windows_x64.signature.is_empty()
                && self.windows_x64.signature.len() < 4096
                && !self.windows_x64.updater_public_key.is_empty()
                && self.windows_x64.updater_public_key.len() < 4096,
            "Missing desktop signature"
        );
        ensure!(self.notes.len() <= 16000, "Release notes are too long");
        Ok(())
    }
    pub fn candidate(&self, current: &str, schema: u32, now: i64) -> Result<()> {
        self.validate()?;
        ensure!(
            semver::Version::parse(&self.version)? > semver::Version::parse(current)?,
            "Release must be newer than the accepted generation"
        );
        ensure!(
            self.published_at <= now + 300 && self.expires_at > now,
            "Release manifest is not currently valid"
        );
        ensure!(
            self.migration.from.contains(schema),
            "Release cannot migrate the current database"
        );
        ensure!(
            self.migration.recovery_protocol == RECOVERY_PROTOCOL,
            "This controller cannot recover the proposed release"
        );
        Ok(())
    }
}
/// Verify the exact payload bytes before parsing. The trust key is deployment configuration,
/// never a key supplied by the manifest or an HTTP caller.
pub fn verify(envelope: &Envelope, public_key: &str) -> Result<Manifest> {
    ensure!(
        envelope.payload.len() <= MAX_ENVELOPE && envelope.signature.len() < 256,
        "Release manifest is too large"
    );
    let key: [u8; 32] = STANDARD
        .decode(public_key.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid release trust key"))?;
    let key = VerifyingKey::from_bytes(&key)?;
    let payload = STANDARD.decode(&envelope.payload)?;
    let signature = Signature::from_slice(&STANDARD.decode(&envelope.signature)?)?;
    let mut signed = DOMAIN.to_vec();
    signed.extend_from_slice(&payload);
    key.verify_strict(&signed, &signature)?;
    let manifest: Manifest = serde_json::from_slice(&payload)?;
    manifest.validate()?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn fixture() -> Manifest {
        let image = Image {
            reference: format!(
                "registry.example/thelxinoe/server@sha256:{}",
                "a".repeat(64)
            ),
            config_digest: format!("sha256:{}", "b".repeat(64)),
        };
        Manifest {
            format: 1,
            version: "0.2.0".into(),
            published_at: 1000,
            expires_at: 2000,
            server: image.clone(),
            controller: image,
            windows_x64: Desktop {
                url: "https://example.com/installer.exe".into(),
                sha256: "c".repeat(64),
                bytes: 100,
                signature: "signature".into(),
                updater_public_key: "public".into(),
            },
            api: Range { min: 1, max: 1 },
            migration: Migration {
                from: Range { min: 24, max: 25 },
                target: 26,
                recovery: RecoveryMode::FullStateRestore,
                recovery_protocol: 1,
            },
            notes: String::new(),
        }
    }
    #[test]
    fn signature_covers_exact_manifest_and_trust_key() {
        let key = SigningKey::from_bytes(&[31; 32]);
        let payload = serde_json::to_vec(&fixture()).unwrap();
        let mut signed = DOMAIN.to_vec();
        signed.extend_from_slice(&payload);
        let mut envelope = Envelope {
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(key.sign(&signed).to_bytes()),
        };
        let public = STANDARD.encode(key.verifying_key().to_bytes());
        assert_eq!(verify(&envelope, &public).unwrap().version, "0.2.0");
        assert!(
            verify(
                &envelope,
                &STANDARD.encode(SigningKey::from_bytes(&[32; 32]).verifying_key().to_bytes())
            )
            .is_err()
        );
        envelope.payload = STANDARD.encode(
            String::from_utf8(payload)
                .unwrap()
                .replace("0.2.0", "0.3.0"),
        );
        assert!(verify(&envelope, &public).is_err());
    }
    #[test]
    fn rejects_downgrades_expiry_unsupported_recovery_and_mutable_images() {
        let mut manifest = fixture();
        assert!(manifest.candidate("0.1.0", 25, 1500).is_ok());
        assert!(manifest.candidate("0.2.0", 25, 1500).is_err());
        assert!(manifest.candidate("0.1.0", 23, 1500).is_err());
        assert!(manifest.candidate("0.1.0", 25, 2001).is_err());
        manifest.migration.recovery_protocol = 2;
        assert!(manifest.candidate("0.1.0", 25, 1500).is_err());
        manifest.server.reference = "registry.example/server:latest".into();
        assert!(manifest.validate().is_err());
        assert!(https("http://example.com/update").is_err());
        assert!(https("https://user:secret@example.com/update").is_err());
    }
}
