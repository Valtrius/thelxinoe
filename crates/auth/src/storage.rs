//! Database operations for auth.
use super::*;

pub(super) async fn put(encrypted: Vec<u8>, db: &Database, scope: String) -> anyhow::Result<()> {
    db.write("auth.put", move |c| { c.execute("INSERT INTO secrets VALUES (?1,?2) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext", params![scope, encrypted])?; Ok(()) }).await
}

pub(super) async fn get(key: String, db: &Database) -> anyhow::Result<Option<Vec<u8>>> {
    db.read("auth.get", move |c| {
        Ok(c.query_row(
            "SELECT ciphertext FROM secrets WHERE scope=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?)
    })
    .await
}

pub(super) async fn issue_session(
    hashed: String,
    db: &Database,
    authorization: SessionAuthorization,
    transport: String,
    name: String,
) -> anyhow::Result<bool> {
    db.write("auth.issue_session", move |c| {
        let (user_id, expected_hash, authorizer, version, verified_at, remembered, client_password) = match authorization {
            SessionAuthorization::Password { user_id, expected_hash } => (user_id, Some(expected_hash), None, None, now(), None, None),
            SessionAuthorization::ApprovedSession { user_id, session_id } => (user_id, None, Some(session_id), None, 0, None, None),
            SessionAuthorization::Verified { user_id, auth_version, verified_at, remembered_device_id } => (user_id, None, None, Some(auth_version), verified_at, remembered_device_id, None),
            SessionAuthorization::ClientPassword { user_id, credential_id } => (user_id, None, None, None, 0, None, Some(credential_id)),
        };
        let inserted = c.execute(
            "INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen,verified_at,remembered_device_id,client_password_id)
             SELECT ?1,id,?3,?4,?5,?6,?7,?6,CASE WHEN ?9 IS NOT NULL THEN (SELECT verified_at FROM sessions WHERE id=?9) ELSE ?11 END,?12,?13 FROM users
             WHERE id=?2 AND NOT EXISTS(SELECT 1 FROM auth_recovery WHERE user_id=?2) AND (
             (?8 IS NOT NULL AND password_hash=?8 AND NOT EXISTS(SELECT 1 FROM auth_totp WHERE user_id=?2 AND enabled=1)) OR
             (?9 IS NOT NULL AND EXISTS(SELECT 1 FROM sessions WHERE id=?9 AND user_id=?2 AND expires_at>?6 AND verified_at>=?6-300)) OR
             (?10 IS NOT NULL AND auth_version=?10 AND (?12 IS NULL OR EXISTS(SELECT 1 FROM auth_remembered_devices WHERE id=?12 AND user_id=?2))) OR
             (?13 IS NOT NULL AND ?4='jellyfin' AND EXISTS(SELECT 1 FROM auth_client_passwords WHERE id=?13 AND user_id=?2)))",
            params![
                id(),
                user_id,
                hashed,
                transport,
                name,
                now(),
                if remembered.is_some() { 253402300799_i64 } else { now() + 30 * 86400 },
                expected_hash,
                authorizer,
                version,
                verified_at,
                remembered,
                client_password,
            ],
        )?;
        Ok(inserted == 1)
    })
    .await
}

pub(super) async fn resolve(
    hash: String,
    transport: String,
    db: &Database,
) -> anyhow::Result<Option<Principal>> {
    let result = db.read("auth.resolve", move |c| {
        let result = c.query_row("SELECT u.id,u.username,u.role,u.timezone,s.id,s.transport,s.last_seen FROM sessions s JOIN user_profiles u ON u.id=s.user_id WHERE token_hash=?1 AND transport=?2 AND expires_at>?3", params![hash,transport,now()], |r| Ok((Principal { user: user_row(r)?, session_id: r.get(4)?, transport: r.get(5)? },r.get::<_,i64>(6)?))).optional()?;
        Ok(result)
    }).await?;
    if let Some((principal, last_seen)) = &result
        && *last_seen < now() - 60
    {
        let session = principal.session_id.clone();
        db.write("auth.touch_session", move |c| {
            c.execute(
                "UPDATE sessions SET last_seen=?1 WHERE id=?2 AND last_seen<?3 AND expires_at>?1",
                params![now(), session, now() - 60],
            )?;
            Ok(())
        })
        .await?;
    }
    Ok(result.map(|(principal, _)| principal))
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
