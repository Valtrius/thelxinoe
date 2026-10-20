#[path = "storage.rs"]
mod storage;

#[cfg(test)]
mod password_work_tests;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Result, anyhow, bail};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
use thelxinoe_core::{Principal, Role, User, id, now};
use thelxinoe_database::Database;

/// How long sign-in credentials last, in seconds.
pub mod lifetime {
    const DAY: i64 = 86400;
    /// A session ends after going this long unused; each use pushes its end back.
    pub const SESSION_IDLE: i64 = 30 * DAY;
    /// The longest a session or a remembered device lasts, however much it's used. Browsers
    /// keep no cookie longer than this either.
    pub const MAX: i64 = 400 * DAY;
    /// A remembered device asks for two-step sign-in again after going this long unused.
    pub const REMEMBERED_IDLE: i64 = 90 * DAY;
    /// A browser or client stays known to an account this long after it last signed in.
    pub const KNOWN: i64 = MAX;
}

pub fn token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[derive(Clone)]
pub struct SecretStore {
    cipher: Aes256Gcm,
}
impl SecretStore {
    pub fn open(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = directory.join("master.key");
        let mut key = [0u8; 32];
        match std::fs::File::open(&path) {
            Ok(mut file) => {
                file.read_exact(&mut key)?;
                let mut extra = [0];
                if file.read(&mut extra)? != 0 {
                    bail!("Invalid master key");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                OsRng.fill_bytes(&mut key);
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path)?;
                file.write_all(&key)?;
                file.sync_all()?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("Invalid key"))?,
        })
    }
    pub fn encrypt(&self, scope: &str, value: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: value,
                    aad: scope.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("Encryption failed"))?;
        Ok([nonce.as_slice(), ciphertext.as_slice()].concat())
    }
    pub fn decrypt(&self, scope: &str, value: &[u8]) -> Result<Vec<u8>> {
        if value.len() < 28 {
            bail!("Invalid encrypted secret");
        }
        self.cipher
            .decrypt(
                Nonce::from_slice(&value[..12]),
                Payload {
                    msg: &value[12..],
                    aad: scope.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("Secret authentication failed"))
    }
    pub async fn put(&self, db: &Database, scope: String, value: &[u8]) -> Result<()> {
        let encrypted = self.encrypt(&scope, value)?;
        storage::put(encrypted, db, scope).await
    }
    pub async fn get(&self, db: &Database, scope: &str) -> Result<Option<Vec<u8>>> {
        let key = scope.to_owned();
        let value: Option<Vec<u8>> = storage::get(key, db).await?;
        value.map(|v| self.decrypt(scope, &v)).transpose()
    }
}

pub fn validate_credentials(username: &str, password: &str) -> Result<()> {
    if username.len() < 2
        || username.len() > 64
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        bail!("Username must be 2–64 letters, numbers, dots, underscores or hyphens");
    }
    if password.chars().count() < 8 || password.len() > 256 {
        bail!("Password must contain at least 8 characters and no more than 256 bytes");
    }
    Ok(())
}
/// Runs password work on the blocking pool. The `permit`, one of the server's password slots,
/// is held until the work ends, even if the caller stops waiting for it.
async fn password_work<T: Send + 'static>(
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await?
}
pub async fn password_hash(
    password: String,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<String> {
    password_work(permit, move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|v| v.to_string())
            .map_err(|_| anyhow!("Password hashing failed"))
    })
    .await
}
pub async fn verify_password(
    password: String,
    hash: String,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<bool> {
    password_work(permit, move || {
        let parsed = PasswordHash::new(&hash).map_err(|_| anyhow!("Invalid password hash"))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
}
pub use storage::user_row;

#[derive(Serialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_seen: i64,
    pub remembered: bool,
}

#[derive(Serialize, Deserialize)]
pub enum SessionAuthorization {
    Password {
        user_id: String,
        expected_hash: String,
    },
    ApprovedSession {
        user_id: String,
        session_id: String,
    },
    Verified {
        user_id: String,
        auth_version: i64,
        verified_at: i64,
        remembered_device_id: Option<String>,
    },
    Oidc {
        user_id: String,
        auth_version: i64,
        verified_at: i64,
        provider_version: String,
        issuer: String,
        subject: String,
    },
    ClientPassword {
        user_id: String,
        credential_id: String,
    },
}
impl SessionAuthorization {
    pub fn user_id(&self) -> &str {
        match self {
            Self::Password { user_id, .. }
            | Self::ApprovedSession { user_id, .. }
            | Self::Verified { user_id, .. }
            | Self::Oidc { user_id, .. }
            | Self::ClientPassword { user_id, .. } => user_id,
        }
    }
}

pub async fn issue_session(
    db: &Database,
    authorization: SessionAuthorization,
    transport: String,
    name: String,
) -> Result<Option<String>> {
    let raw = token();
    let hashed = digest(&raw);
    Ok(
        storage::issue_session(hashed, db, authorization, transport, name)
            .await?
            .then_some(raw),
    )
}
pub async fn resolve(db: &Database, raw: &str, transport: &str) -> Result<Option<Principal>> {
    if raw.len() != 64 {
        return Ok(None);
    }
    let hash = digest(raw);
    let transport = transport.to_owned();
    storage::resolve(hash, transport, db).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn recent_session_authentication_does_not_wait_for_an_unrelated_writer() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("auth.db"))?;
        db.write("test.fixture", |db|{db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','reader','unused','user',?1)",[now()])?;Ok(())}).await?;
        let token = issue_session(
            &db,
            SessionAuthorization::Password {
                user_id: "user".into(),
                expected_hash: "unused".into(),
            },
            "device".into(),
            "test".into(),
        )
        .await?
        .unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let writer = db.clone();
        let writing = tokio::spawn(async move {
            writer
                .write("test.unrelated_write", move |db| {
                    let tx =
                        db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    let _ = entered.send(());
                    blocked.recv()?;
                    tx.commit()?;
                    Ok(())
                })
                .await
        });
        ready.await?;
        let principal = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resolve(&db, &token, "device"),
        )
        .await??;
        assert_eq!(principal.unwrap().user.id, "user");
        release.send(())?;
        writing.await??;
        db.shutdown().await?;
        Ok(())
    }
    // A session in use keeps going until the cap, a remembered one included, and its device
    // stays in use with it; left unused, it ends.
    #[tokio::test]
    async fn using_a_session_slides_its_end_up_to_the_cap() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("auth.db"))?;
        db.write("test.fixture", |db| {
            db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','reader','unused','user',1)", [])?;
            db.execute("INSERT INTO auth_remembered_devices(id,user_id,token_hash,name,created_at,last_seen) VALUES ('laptop','user','remembered','Laptop',1,1)", [])?;
            Ok(())
        })
        .await?;
        let token = issue_session(
            &db,
            SessionAuthorization::Verified {
                user_id: "user".into(),
                auth_version: 0,
                verified_at: 0,
                remembered_device_id: Some("laptop".into()),
            },
            "device".into(),
            "test".into(),
        )
        .await?
        .unwrap();
        let set = |sql: &'static str| {
            let db = db.clone();
            async move {
                db.write("test.age", move |db| Ok(db.execute(sql, [now()])?))
                    .await
            }
        };
        let ends = || async {
            db.read("test.ends", |db| {
                Ok(db.query_row(
                    "SELECT s.expires_at-?1,s.created_at,d.last_seen FROM sessions s JOIN auth_remembered_devices d ON d.id=s.remembered_device_id",
                    [now()],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
                )?)
            })
            .await
        };
        let near = |left: i64, expected: i64| (left - expected).abs() <= 5;
        assert!(near(ends().await?.0, lifetime::SESSION_IDLE));
        // A day before its end, using it pushes the end back by a whole idle lifetime.
        set("UPDATE sessions SET last_seen=?1-29*86400,expires_at=?1+86400").await?;
        assert!(resolve(&db, &token, "device").await?.is_some());
        let (left, _, device) = ends().await?;
        assert!(near(left, lifetime::SESSION_IDLE), "{left}");
        assert!(near(device, now()));
        // Near the cap, use can't push it past.
        set("UPDATE sessions SET created_at=?1-399*86400,last_seen=?1-120").await?;
        assert!(resolve(&db, &token, "device").await?.is_some());
        let (left, created, _) = ends().await?;
        assert!(near(left + now(), created + lifetime::MAX));
        assert!(left < lifetime::SESSION_IDLE);
        set("UPDATE sessions SET expires_at=?1-1").await?;
        assert!(resolve(&db, &token, "device").await?.is_none());
        db.shutdown().await?;
        Ok(())
    }
    #[test]
    fn secret_is_persistent_authenticated_and_bound_to_scope() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = SecretStore::open(dir.path())?;
        let mut data = store.encrypt("alice/google", b"secret")?;
        assert_eq!(
            SecretStore::open(dir.path())?.decrypt("alice/google", &data)?,
            b"secret"
        );
        assert!(store.decrypt("bob/google", &data).is_err());
        data[15] ^= 1;
        assert!(store.decrypt("alice/google", &data).is_err());
        Ok(())
    }
    #[tokio::test]
    async fn passwords_are_salted_and_verified() -> Result<()> {
        let a = password_hash("long passphrase".into(), None).await?;
        let b = password_hash("long passphrase".into(), None).await?;
        assert_ne!(a, b);
        assert!(verify_password("long passphrase".into(), a.clone(), None).await?);
        assert!(!verify_password("wrong".into(), a, None).await?);
        Ok(())
    }
    #[test]
    fn credentials_accept_eight_characters_without_composition_rules() {
        assert!(validate_credentials("admin", "12345678").is_ok());
        assert!(validate_credentials("admin", "1234567").is_err());
        assert!(validate_credentials("admin", "éééééééé").is_ok());
        assert!(validate_credentials("admin", "éééé").is_err());
        assert!(validate_credentials("admin", &"a".repeat(256)).is_ok());
        assert!(validate_credentials("admin", &"a".repeat(257)).is_err());
    }
}
