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
    user_id: String,
    transport: String,
    name: String,
) -> anyhow::Result<()> {
    db.write("auth.issue_session", move |c| {
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
