use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Result, anyhow, bail};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
use thelxinoe_core::{Principal, Role, User, id, now};
use thelxinoe_database::Database;

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
        db.call(move |c| { c.execute("INSERT INTO secrets VALUES (?1,?2) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext", params![scope, encrypted])?; Ok(()) }).await
    }
    pub async fn get(&self, db: &Database, scope: &str) -> Result<Option<Vec<u8>>> {
        let key = scope.to_owned();
        let value: Option<Vec<u8>> = db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT ciphertext FROM secrets WHERE scope=?1",
                    [key],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await?;
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
    if password.len() < 12 || password.len() > 256 {
        bail!("Password must be 12–256 bytes");
    }
    Ok(())
}
pub async fn password_hash(password: String) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|v| v.to_string())
            .map_err(|_| anyhow!("Password hashing failed"))
    })
    .await?
}
pub async fn verify_password(password: String, hash: String) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&hash).map_err(|_| anyhow!("Invalid password hash"))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await?
}
pub fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        role: if r.get::<_, String>(2)? == "admin" {
            Role::Admin
        } else {
            Role::User
        },
        timezone: r.get(3)?,
    })
}

#[derive(Serialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_seen: i64,
}

pub async fn issue_session(
    db: &Database,
    user_id: String,
    transport: String,
    name: String,
) -> Result<String> {
    let raw = token();
    let hashed = digest(&raw);
    db.call(move |c| {
        c.execute(
            "INSERT INTO sessions VALUES (?1,?2,?3,?4,?5,?6,?7,?6)",
            params![
                id(),
                user_id,
                hashed,
                transport,
                name,
                now(),
                now() + 30 * 86400
            ],
        )?;
        Ok(())
    })
    .await?;
    Ok(raw)
}
pub async fn resolve(db: &Database, raw: &str, transport: &str) -> Result<Option<Principal>> {
    if raw.len() != 64 {
        return Ok(None);
    }
    let hash = digest(raw);
    let transport = transport.to_owned();
    db.call(move |c| {
        let result = c.query_row("SELECT u.id,u.username,u.role,u.timezone,s.id,s.transport,s.last_seen FROM sessions s JOIN users u ON u.id=s.user_id WHERE token_hash=?1 AND transport=?2 AND expires_at>?3", params![hash,transport,now()], |r| Ok((Principal { user: user_row(r)?, session_id: r.get(4)?, transport: r.get(5)? },r.get::<_,i64>(6)?))).optional()?;
        if let Some((p,last_seen)) = &result && *last_seen<now()-60 { c.execute("UPDATE sessions SET last_seen=?1 WHERE id=?2 AND last_seen<?3", params![now(),p.session_id,now()-60])?; }
        Ok(result.map(|(p,_)|p))
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn recent_session_authentication_does_not_wait_for_an_unrelated_writer() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("auth.db"))?;
        db.call(|db|{db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','reader','unused','user',?1)",[now()])?;Ok(())}).await?;
        let token = issue_session(&db, "user".into(), "device".into(), "test".into()).await?;
        let mut writer = db.connect()?;
        let _transaction =
            writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let principal = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resolve(&db, &token, "device"),
        )
        .await??;
        assert_eq!(principal.unwrap().user.id, "user");
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
        let a = password_hash("long passphrase".into()).await?;
        let b = password_hash("long passphrase".into()).await?;
        assert_ne!(a, b);
        assert!(verify_password("long passphrase".into(), a.clone()).await?);
        assert!(!verify_password("wrong".into(), a).await?);
        Ok(())
    }
}
