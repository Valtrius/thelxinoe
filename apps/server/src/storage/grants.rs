//! Database operations for grants.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn artwork(
    db: &Database,
    secrets: thelxinoe_auth::SecretStore,
    p: Principal,
    resource: &'static str,
) -> anyhow::Result<String> {
    const DAY: i64 = 86400;
    let prefix = format!("{resource}-grant:");
    let scope = format!("{prefix}{}", p.session_id);
    fn cached(
        db: &rusqlite::Connection,
        secrets: &thelxinoe_auth::SecretStore,
        scope: &str,
    ) -> anyhow::Result<Option<String>> {
        let encrypted: Option<Vec<u8>> = db
            .query_row(
                "SELECT ciphertext FROM secrets WHERE scope=?1",
                [scope],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(encrypted) = encrypted {
            let (token, issued): (String, i64) =
                serde_json::from_slice(&secrets.decrypt(scope, &encrypted)?)?;
            if now() < issued + DAY {
                return Ok(Some(token));
            }
        }
        Ok(None)
    }
    let reader_secrets = secrets.clone();
    let reader_scope = scope.clone();
    if let Some(token) = db
        .read("grants.artwork", move |db| {
            cached(db, &reader_secrets, &reader_scope)
        })
        .await?
    {
        return Ok(token);
    }
    db.write("grants.artwork.issue", move |db| {
        let tx = db.transaction()?;
        if let Some(token) = cached(&tx, &secrets, &scope)? { return Ok(token); }
        let token = thelxinoe_auth::token();
        let issued = now();
        let encrypted = secrets.encrypt(&scope, &serde_json::to_vec(&(&token, issued))?)?;
        tx.execute("DELETE FROM secrets WHERE scope LIKE ?1 AND substr(scope,?2) NOT IN (SELECT id FROM sessions WHERE expires_at>?3)", params![format!("{prefix}%"), prefix.len() as i64 + 1, issued])?;
        tx.execute("DELETE FROM playback_grants WHERE expires_at<=?1", [issued])?;
        // A URL issued just before rotation still has a full day to load lazy images.
        tx.execute("INSERT INTO playback_grants VALUES (?1,?2,?3,?4,?5)", params![thelxinoe_auth::digest(&token), p.user.id, p.session_id, resource, issued + 2 * DAY])?;
        tx.execute("INSERT INTO secrets VALUES (?1,?2) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext", params![scope, encrypted])?;
        tx.commit()?;
        Ok(token)
    }).await
}

pub(super) async fn issue(
    hash: String,
    p: Principal,
    resource: String,
    db: &Database,
    ttl: i64,
) -> anyhow::Result<()> {
    db.write("grants.issue", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM playback_grants WHERE expires_at<=?1", [now()])?;
        tx.execute(
            "INSERT INTO playback_grants VALUES (?1,?2,?3,?4,?5)",
            params![hash, p.user.id, p.session_id, resource, now() + ttl],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn resolve(
    hash: String,
    resource: String,
    db: &Database,
    consume: bool,
) -> anyhow::Result<Option<Principal>> {
    fn query(
        db: &rusqlite::Connection,
        hash: &str,
        resource: &str,
    ) -> rusqlite::Result<Option<Principal>> {
        db.query_row("SELECT u.id,u.username,u.role,u.timezone,s.id,s.transport FROM playback_grants g JOIN sessions s ON s.id=g.session_id JOIN user_profiles u ON u.id=g.user_id WHERE g.token_hash=?1 AND g.resource=?2 AND g.expires_at>?3 AND s.expires_at>?3",params![hash,resource,now()],|r|Ok(Principal{user:thelxinoe_auth::user_row(r)?,session_id:r.get(4)?,transport:r.get(5)?})).optional()
    }
    // Every HLS segment, range request and artwork load checks a grant: those reads stay
    // off the single writer. Only a one-time grant is consumed in a write.
    if !consume {
        return db
            .read("grants.resolve", move |db| Ok(query(db, &hash, &resource)?))
            .await;
    }
    db.write("grants.consume", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let principal = query(&tx, &hash, &resource)?;
        tx.execute("DELETE FROM playback_grants WHERE token_hash=?1", [hash])?;
        tx.commit()?;
        Ok(principal)
    })
    .await
}

pub(super) async fn event_ticket(db: &Database) -> anyhow::Result<i64> {
    db.read("grants.event_ticket", |db| {
        Ok(
            db.query_row("SELECT COALESCE(MAX(id),0) FROM events", [], |r| {
                r.get::<_, i64>(0)
            })?,
        )
    })
    .await
}
