use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use thelxinoe_auth::SessionAuthorization;
use thelxinoe_core::{Principal, now};
use thelxinoe_database::Database;

#[derive(Debug)]
pub(super) enum Fault {
    Unauthorized,
    Verify,
    LastMethod,
    Conflict,
    SelfRecovery,
    /// Rejected input, with a stable code and a user-facing explanation.
    Invalid(&'static str, &'static str),
    /// Conflicts with the account's current state, with a stable code and explanation.
    State(&'static str, &'static str),
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Fault {}
fn authorized(c: &Connection, p: &Principal, fresh: bool) -> Result<()> {
    let verified = c
        .query_row(
            "SELECT verified_at FROM sessions WHERE id=?1 AND user_id=?2 AND expires_at>?3",
            params![p.session_id, p.user.id, now()],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(Fault::Unauthorized)?;
    if fresh && verified < now() - 300 {
        return Err(Fault::Verify.into());
    }
    Ok(())
}
pub(crate) fn authorize_admin(c: &Connection, p: &Principal) -> Result<()> {
    authorized(c, p, true)?;
    if !c.query_row(
        "SELECT role='admin' FROM users WHERE id=?1",
        [&p.user.id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Fault::Unauthorized.into());
    }
    Ok(())
}
pub(super) async fn fresh(db: &Database, p: Principal) -> Result<()> {
    db.read("authentication.fresh", move |c| authorized(c, &p, true))
        .await
}
#[derive(Clone)]
pub(super) struct Account {
    pub id: String,
    pub username: String,
    pub password: Option<String>,
    pub version: i64,
    pub totp: Option<(Vec<u8>, bool)>,
}
fn account_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        username: r.get(1)?,
        password: r.get(2)?,
        version: r.get(3)?,
        totp: r
            .get::<_, Option<Vec<u8>>>(4)?
            .map(|s| (s, r.get::<_, bool>(5).unwrap_or(false))),
    })
}
pub(super) async fn account(db: &Database, user: String, by_name: bool) -> Result<Option<Account>> {
    db.read("authentication.account", move |c| Ok(c.query_row(if by_name {"SELECT u.id,u.username,u.password_hash,u.auth_version,t.secret,t.enabled FROM users u LEFT JOIN auth_totp t ON t.user_id=u.id WHERE u.username=?1"} else {"SELECT u.id,u.username,u.password_hash,u.auth_version,t.secret,t.enabled FROM users u LEFT JOIN auth_totp t ON t.user_id=u.id WHERE u.id=?1"}, [user], account_row).optional()?)).await
}
#[derive(Serialize)]
pub(super) struct PasskeyInfo {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}
#[derive(Serialize)]
pub(super) struct Methods {
    pub password: bool,
    pub totp: bool,
    pub passkeys: Vec<PasskeyInfo>,
    pub oidc: Option<String>,
    pub fresh: bool,
}
pub(super) async fn methods(db: &Database, p: Principal) -> Result<Methods> {
    db.read("authentication.methods",move |c| {
        authorized(c,&p,false)?;
        let (password,totp,fresh)=c.query_row("SELECT u.password_hash IS NOT NULL,EXISTS(SELECT 1 FROM auth_totp WHERE user_id=u.id AND enabled=1),s.verified_at>=?3-300 FROM users u JOIN sessions s ON s.user_id=u.id WHERE u.id=?1 AND s.id=?2",params![p.user.id,p.session_id,now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let passkeys=c.prepare("SELECT id,name,created_at FROM auth_passkeys WHERE user_id=?1 ORDER BY created_at")?.query_map([&p.user.id],|r|Ok(PasskeyInfo{id:r.get(0)?,name:r.get(1)?,created_at:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let oidc=c.query_row("SELECT o.issuer FROM auth_oidc_identities o JOIN auth_oidc_provider p ON p.issuer=o.issuer WHERE o.user_id=?1",[&p.user.id],|r|r.get(0)).optional()?;
        Ok(Methods{password,totp,passkeys,oidc,fresh})
    }).await
}
pub(super) struct Attempt {
    pub kind: String,
    pub user: Option<String>,
    pub session: Option<String>,
    pub version: Option<i64>,
    pub binding: Option<String>,
    pub payload: Vec<u8>,
}
pub(super) async fn attempt_put(
    db: &Database,
    hash: String,
    a: Attempt,
    p: Option<Principal>,
    require_fresh: bool,
) -> Result<()> {
    db.write("authentication.attempt.begin",move |c|{
        let tx=c.transaction()?;
        if let Some(p)=p {authorized(&tx,&p,require_fresh)?;}
        tx.execute("DELETE FROM auth_attempts WHERE expires_at<=?1",[now()])?;
        if tx.query_row("SELECT COUNT(*) FROM auth_attempts",[],|r|r.get::<_,i64>(0))?>10000 {return Err(Fault::Conflict.into());}
        tx.execute("INSERT INTO auth_attempts(token_hash,kind,user_id,session_id,auth_version,binding_hash,payload,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![hash,a.kind,a.user,a.session,a.version,a.binding,a.payload,now()+300])?;
        tx.commit()?;Ok(())
    }).await
}
fn read_attempt(c: &Connection, hash: &str, kind: &str) -> Result<Option<Attempt>> {
    Ok(c.query_row("SELECT kind,user_id,session_id,auth_version,binding_hash,payload FROM auth_attempts a WHERE token_hash=?1 AND kind=?2 AND expires_at>?3 AND failures<5 AND (user_id IS NULL OR EXISTS(SELECT 1 FROM users u WHERE u.id=a.user_id AND u.auth_version=a.auth_version)) AND (session_id IS NULL OR EXISTS(SELECT 1 FROM sessions s WHERE s.id=a.session_id AND s.user_id=a.user_id AND s.expires_at>?3))",params![hash,kind,now()],|r|Ok(Attempt{kind:r.get(0)?,user:r.get(1)?,session:r.get(2)?,version:r.get(3)?,binding:r.get(4)?,payload:r.get(5)?})).optional()?)
}
pub(super) async fn attempt(
    db: &Database,
    hash: String,
    kind: String,
    consume: bool,
) -> Result<Attempt> {
    if consume {
        db.write("authentication.attempt.consume", move |c| {
            let tx = c.transaction()?;
            let a = read_attempt(&tx, &hash, &kind)?.ok_or(Fault::Unauthorized)?;
            tx.execute("DELETE FROM auth_attempts WHERE token_hash=?1", [hash])?;
            tx.commit()?;
            Ok(a)
        })
        .await
    } else {
        db.read("authentication.attempt.read", move |c| {
            read_attempt(c, &hash, &kind)?.ok_or_else(|| Fault::Unauthorized.into())
        })
        .await
    }
}
/// Records a failed proof and returns the attempt's failure count.
pub(super) async fn failed_attempt(db: &Database, hash: String) -> Result<i64> {
    db.write("authentication.attempt.failed", move |c| {
        Ok(c.query_row(
            "UPDATE auth_attempts SET failures=failures+1 WHERE token_hash=?1 RETURNING failures",
            [hash],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(5))
    })
    .await
}
fn consume_step(c: &Connection, user: &str, step: u64, enabled: bool) -> Result<()> {
    let last: i64 = c
        .query_row(
            "SELECT last_step FROM auth_totp WHERE user_id=?1 AND enabled=?2",
            params![user, enabled],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Fault::Unauthorized)?;
    if last >= step as i64 {
        return Err(Fault::Invalid(
            "totp_reused",
            "This code was already used. Wait for the next code from your authenticator app.",
        )
        .into());
    }
    c.execute(
        "UPDATE auth_totp SET last_step=?1 WHERE user_id=?2",
        params![step as i64, user],
    )?;
    Ok(())
}
pub(super) async fn verify(
    db: &Database,
    p: Principal,
    version: i64,
    step: Option<u64>,
) -> Result<()> {
    db.write("authentication.verify", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, false)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND auth_version=?2)",
            params![p.user.id, version],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Fault::Unauthorized.into());
        }
        if let Some(step) = step {
            consume_step(&tx, &p.user.id, step, true)?;
        }
        tx.execute(
            "UPDATE sessions SET verified_at=?1 WHERE id=?2",
            params![now(), p.session_id],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) async fn remembered(
    db: &Database,
    user: String,
    hash: String,
) -> Result<Option<String>> {
    db.write("authentication.remembered.resolve",move |c|Ok(c.query_row("UPDATE auth_remembered_devices SET last_seen=?3 WHERE user_id=?1 AND token_hash=?2 RETURNING id",params![user,hash,now()],|r|r.get(0)).optional()?)).await
}
pub(super) struct Remember {
    pub id: String,
    pub hash: String,
    pub name: String,
}
pub(super) async fn finish_totp(
    db: &Database,
    hash: String,
    user: String,
    version: i64,
    step: u64,
    remember: Option<Remember>,
) -> Result<SessionAuthorization> {
    db.write("authentication.totp.login", move |c| {
        let tx = c.transaction()?;
        let a = read_attempt(&tx, &hash, "totp")?.ok_or(Fault::Unauthorized)?;
        if a.user.as_deref() != Some(&user) || a.version != Some(version) {
            return Err(Fault::Unauthorized.into());
        }
        consume_step(&tx, &user, step, true)?;
        tx.execute("DELETE FROM auth_attempts WHERE token_hash=?1", [hash])?;
        let device = remember
            .map(|r| {
                tx.execute(
                    "INSERT INTO auth_remembered_devices VALUES(?1,?2,?3,?4,?5,?5)",
                    params![r.id, user, r.hash, r.name, now()],
                )?;
                Ok::<_, anyhow::Error>(r.id)
            })
            .transpose()?;
        tx.commit()?;
        Ok(SessionAuthorization::Verified {
            user_id: user,
            auth_version: version,
            verified_at: now(),
            remembered_device_id: device,
        })
    })
    .await
}
fn revoke_other(c: &Connection, p: &Principal) -> Result<()> {
    c.execute(
        "UPDATE users SET auth_version=auth_version+1 WHERE id=?1",
        [&p.user.id],
    )?;
    c.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND id<>?2",
        params![p.user.id, p.session_id],
    )?;
    c.execute(
        "UPDATE sessions SET remembered_device_id=NULL,expires_at=MIN(expires_at,?2) WHERE id=?1",
        params![p.session_id, now() + 30 * 86400],
    )?;
    c.execute(
        "DELETE FROM auth_remembered_devices WHERE user_id=?1",
        [&p.user.id],
    )?;
    c.execute("DELETE FROM auth_attempts WHERE user_id=?1", [&p.user.id])?;
    c.execute(
        "DELETE FROM auth_desktop_requests WHERE user_id=?1 OR target_user_id=?1",
        [&p.user.id],
    )?;
    Ok(())
}
fn audit(c: &Connection, p: &Principal, action: &str, target: &str) -> Result<()> {
    c.execute(
        "INSERT INTO audit(actor_id,action,target,created_at) VALUES(?1,?2,?3,?4)",
        params![p.user.id, action, target, now()],
    )?;
    Ok(())
}
pub(super) fn totp_setup_missing() -> Fault {
    Fault::State(
        "totp_setup_missing",
        "Authenticator setup was restarted or canceled. Start setup again and scan the new QR code.",
    )
}
pub(super) async fn totp_start(db: &Database, p: Principal, secret: Vec<u8>) -> Result<()> {
    db.write("authentication.totp.begin",move |c|{
        let tx=c.transaction()?;authorized(&tx,&p,true)?;
        if !tx.query_row("SELECT password_hash IS NOT NULL FROM users WHERE id=?1",[&p.user.id],|r|r.get::<_,bool>(0))? {return Err(Fault::State("password_required","Set a password first. The authenticator app protects password sign-in.").into());}
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM auth_totp WHERE user_id=?1 AND enabled=1)",[&p.user.id],|r|r.get::<_,bool>(0))?{return Err(Fault::State("totp_enabled","An authenticator app is already enabled. Remove it before setting up a new one.").into());}
        tx.execute("INSERT INTO auth_totp(user_id,secret) VALUES(?1,?2) ON CONFLICT(user_id) DO UPDATE SET secret=excluded.secret,last_step=-1",params![p.user.id,secret])?;tx.commit()?;Ok(())
    }).await
}
pub(super) async fn totp_confirm(
    db: &Database,
    p: Principal,
    secret: Vec<u8>,
    step: u64,
) -> Result<()> {
    db.write("authentication.totp.confirm", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        match tx
            .query_row(
                "SELECT secret=?2,enabled FROM auth_totp WHERE user_id=?1",
                params![p.user.id, secret],
                |r| Ok((r.get::<_, bool>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?
        {
            Some((_, true)) => {
                return Err(Fault::State(
                    "totp_enabled",
                    "An authenticator app is already enabled for this account.",
                )
                .into());
            }
            Some((true, false)) => {}
            _ => return Err(totp_setup_missing().into()),
        }
        consume_step(&tx, &p.user.id, step, false)?;
        tx.execute(
            "UPDATE auth_totp SET enabled=1 WHERE user_id=?1",
            [&p.user.id],
        )?;
        revoke_other(&tx, &p)?;
        audit(&tx, &p, "auth.totp.enabled", &p.user.id)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) async fn remove_method(
    db: &Database,
    p: Principal,
    kind: String,
    key: String,
) -> Result<()> {
    db.write("authentication.method.remove",move |c|{
        let tx=c.transaction()?;authorized(&tx,&p,true)?;
        match kind.as_str(){
            "password"=>{tx.execute("UPDATE users SET password_hash=NULL WHERE id=?1",[&p.user.id])?;},
            "totp"=>{tx.execute("DELETE FROM auth_totp WHERE user_id=?1",[&p.user.id])?;},
            "passkey"=>{if tx.execute("DELETE FROM auth_passkeys WHERE id=?1 AND user_id=?2",params![key,p.user.id])?!=1{return Err(Fault::Unauthorized.into());}},
            "oidc"=>{tx.execute("DELETE FROM auth_oidc_identities WHERE user_id=?1",[&p.user.id])?;},
            _=>return Err(Fault::Conflict.into())
        }
        let count:i64=tx.query_row("SELECT (password_hash IS NOT NULL)+(SELECT COUNT(*) FROM auth_passkeys WHERE user_id=?1)+(SELECT COUNT(*) FROM auth_oidc_identities o JOIN auth_oidc_provider p ON p.issuer=o.issuer WHERE o.user_id=?1) FROM users WHERE id=?1",[&p.user.id],|r|r.get(0))?;
        if count==0 {return Err(Fault::LastMethod.into());}
        if kind=="password" {tx.execute("DELETE FROM auth_totp WHERE user_id=?1",[&p.user.id])?;}
        revoke_other(&tx,&p)?;audit(&tx,&p,"auth.method.removed",&kind)?;tx.commit()?;Ok(())
    }).await
}
pub(super) async fn password(
    db: &Database,
    p: Principal,
    hash: String,
    revoke_client_passwords: bool,
) -> Result<()> {
    db.write("authentication.password", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        tx.execute(
            "UPDATE users SET password_hash=?1 WHERE id=?2",
            params![hash, p.user.id],
        )?;
        revoke_other(&tx, &p)?;
        audit(&tx, &p, "user.password", &p.user.id)?;
        if revoke_client_passwords
            && tx.execute(
                "DELETE FROM auth_client_passwords WHERE user_id=?1",
                [&p.user.id],
            )? > 0
        {
            audit(&tx, &p, "auth.client-password.revoked-all", &p.user.id)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}
#[derive(Serialize)]
pub(super) struct Device {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen: i64,
}
pub(super) async fn devices(db: &Database, p: Principal, clients: bool) -> Result<Vec<Device>> {
    db.read("authentication.devices",move |c|{
        authorized(c,&p,false)?;
        Ok(c.prepare(if clients {"SELECT id,name,created_at,last_seen FROM auth_client_passwords WHERE user_id=?1 ORDER BY last_seen DESC"}else{"SELECT id,name,created_at,last_seen FROM auth_remembered_devices WHERE user_id=?1 ORDER BY last_seen DESC"})?.query_map([p.user.id],|r|Ok(Device{id:r.get(0)?,name:r.get(1)?,created_at:r.get(2)?,last_seen:r.get(3)?}))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}
pub(super) async fn forget(db: &Database, p: Principal, key: String, clients: bool) -> Result<()> {
    db.write("authentication.device.forget", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        tx.execute(
            if clients {
                "DELETE FROM sessions WHERE user_id=?1 AND client_password_id=?2"
            } else {
                "DELETE FROM sessions WHERE user_id=?1 AND remembered_device_id=?2"
            },
            params![p.user.id, key],
        )?;
        tx.execute(
            if clients {
                "DELETE FROM auth_client_passwords WHERE user_id=?1 AND id=?2"
            } else {
                "DELETE FROM auth_remembered_devices WHERE user_id=?1 AND id=?2"
            },
            params![p.user.id, key],
        )?;
        audit(
            &tx,
            &p,
            if clients {
                "auth.client-password.revoked"
            } else {
                "auth.device.forgotten"
            },
            &key,
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) async fn client_create(
    db: &Database,
    p: Principal,
    key: String,
    hash: String,
    name: String,
) -> Result<()> {
    db.write("authentication.client-password.create", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        tx.execute(
            "INSERT INTO auth_client_passwords VALUES(?1,?2,?3,?4,?5,?5)",
            params![key, p.user.id, hash, name, now()],
        )?;
        audit(&tx, &p, "auth.client-password.created", &key)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) async fn client_password(
    db: &Database,
    username: String,
    hash: String,
) -> Result<Option<SessionAuthorization>> {
    db.write("authentication.client-password.resolve",move |c|Ok(c.query_row("UPDATE auth_client_passwords SET last_seen=?3 WHERE token_hash=?1 AND user_id=(SELECT id FROM users WHERE username=?2) RETURNING user_id,id",params![hash,username,now()],|r|Ok(SessionAuthorization::ClientPassword{user_id:r.get(0)?,credential_id:r.get(1)?})).optional()?)).await
}
pub(super) async fn passkeys(db: &Database, user: String) -> Result<Vec<(String, String)>> {
    db.read("authentication.passkeys", move |c| {
        Ok(
            c.prepare("SELECT id,credential FROM auth_passkeys WHERE user_id=?1")?
                .query_map([user], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        )
    })
    .await
}
pub(super) async fn passkey_add(
    db: &Database,
    p: Principal,
    version: i64,
    key: String,
    name: String,
    credential: String,
) -> Result<()> {
    db.write("authentication.passkey.add", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND auth_version=?2)",
            params![p.user.id, version],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Fault::Unauthorized.into());
        }
        tx.execute(
            "INSERT INTO auth_passkeys VALUES(?1,?2,?3,?4,?5)",
            params![key, p.user.id, name, credential, now()],
        )?;
        audit(&tx, &p, "auth.passkey.added", &key)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) async fn passkey_used(
    db: &Database,
    user: String,
    version: i64,
    key: String,
    previous: String,
    credential: String,
) -> Result<()> {
    db.write("authentication.passkey.used",move|c|{
        if c.execute("UPDATE auth_passkeys SET credential=?1 WHERE id=?2 AND user_id=?3 AND credential=?4 AND EXISTS(SELECT 1 FROM users WHERE id=?3 AND auth_version=?5)",params![credential,key,user,previous,version])?!=1{return Err(Fault::Unauthorized.into());}Ok(())
    }).await
}
#[derive(Clone, Serialize)]
pub(super) struct Provider {
    pub discovery_url: String,
    pub issuer: String,
    pub client_id: String,
    #[serde(skip_serializing)]
    pub secret: Vec<u8>,
    pub label: String,
    pub version: String,
}
pub(super) async fn provider(db: &Database) -> Result<Option<Provider>> {
    db.read("authentication.oidc.provider",|c|Ok(c.query_row("SELECT discovery_url,issuer,client_id,client_secret,label,version FROM auth_oidc_provider WHERE id=1",[],|r|Ok(Provider{discovery_url:r.get(0)?,issuer:r.get(1)?,client_id:r.get(2)?,secret:r.get(3)?,label:r.get(4)?,version:r.get(5)?})).optional()?)).await
}
pub(super) async fn provider_save(
    db: &Database,
    p: Principal,
    provider: Option<Provider>,
) -> Result<()> {
    db.write("authentication.oidc.configure", move |c| {
        let tx=c.transaction()?; authorize_admin(&tx,&p)?;
        let issuer=provider.as_ref().map(|v|v.issuer.as_str());
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM users u JOIN auth_oidc_identities o ON o.user_id=u.id WHERE u.password_hash IS NULL AND NOT EXISTS(SELECT 1 FROM auth_passkeys WHERE user_id=u.id) AND (?1 IS NULL OR o.issuer<>?1))",[issuer],|r|r.get::<_,bool>(0))? {return Err(Fault::LastMethod.into());}
        if let Some(v)=provider {
            tx.execute("DELETE FROM auth_oidc_identities WHERE issuer<>?1",[&v.issuer])?;
            tx.execute("INSERT INTO auth_oidc_provider VALUES(1,?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET discovery_url=excluded.discovery_url,issuer=excluded.issuer,client_id=excluded.client_id,client_secret=excluded.client_secret,label=excluded.label,version=excluded.version",params![v.discovery_url,v.issuer,v.client_id,v.secret,v.label,v.version])?;
        } else { tx.execute("DELETE FROM auth_oidc_provider",[])?; }
        tx.execute("DELETE FROM auth_attempts WHERE kind='oidc'",[])?;
        audit(&tx,&p,"auth.oidc.configured","server")?; tx.commit()?; Ok(())
    }).await
}
pub(super) async fn oidc_account(
    db: &Database,
    issuer: String,
    subject: String,
) -> Result<Option<Account>> {
    db.read("authentication.oidc.identity",move|c|Ok(c.query_row("SELECT u.id,u.username,u.password_hash,u.auth_version,t.secret,t.enabled FROM users u JOIN auth_oidc_identities o ON o.user_id=u.id LEFT JOIN auth_totp t ON t.user_id=u.id WHERE o.issuer=?1 AND o.subject=?2",params![issuer,subject],account_row).optional()?)).await
}
pub(super) async fn oidc_link(
    db: &Database,
    p: Principal,
    version: i64,
    provider_version: String,
    issuer: String,
    subject: String,
) -> Result<()> {
    db.write("authentication.oidc.link",move |c|{let tx=c.transaction()?;authorized(&tx,&p,true)?;
        require_provider(&tx, &provider_version, &issuer)?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND auth_version=?2)",params![p.user.id,version],|r|r.get::<_,bool>(0))?{return Err(Fault::Unauthorized.into());}
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM auth_oidc_identities WHERE user_id=?1 OR (issuer=?2 AND subject=?3))",params![p.user.id,issuer,subject],|r|r.get::<_,bool>(0))?{return Err(Fault::Conflict.into());}
        tx.execute("INSERT INTO auth_oidc_identities VALUES(?1,?2,?3,?4)",params![issuer,subject,p.user.id,now()])?;audit(&tx,&p,"auth.oidc.linked",&p.user.id)?;tx.commit()?;Ok(())}).await
}
fn require_provider(c: &Connection, version: &str, issuer: &str) -> Result<()> {
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM auth_oidc_provider WHERE id=1 AND version=?1 AND issuer=?2)",
        params![version, issuer],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Fault::Unauthorized.into());
    }
    Ok(())
}
pub(super) async fn oidc_verify(
    db: &Database,
    p: Principal,
    version: i64,
    provider_version: String,
    issuer: String,
    subject: String,
) -> Result<()> {
    db.write("authentication.oidc.verify", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, false)?;
        require_provider(&tx, &provider_version, &issuer)?;
        if tx.execute(
            "UPDATE sessions SET verified_at=?1 WHERE id=?2 AND user_id=?3
             AND EXISTS(SELECT 1 FROM users WHERE id=?3 AND auth_version=?4)
             AND EXISTS(SELECT 1 FROM auth_oidc_identities WHERE user_id=?3 AND issuer=?5 AND subject=?6)",
            params![now(), p.session_id, p.user.id, version, issuer, subject],
        )? != 1 {
            return Err(Fault::Unauthorized.into());
        }
        tx.commit()?;
        Ok(())
    }).await
}
pub(super) async fn recovery(
    db: &Database,
    p: Option<Principal>,
    user: String,
    by_name: bool,
    hash: String,
) -> Result<String> {
    db.write("authentication.recovery", move |c| {
        let tx = c.transaction()?;
        if let Some(p) = &p {
            authorize_admin(&tx, p)?;
        }
        let user: String = tx
            .query_row(
                if by_name {
                    "SELECT id FROM users WHERE username=?1"
                } else {
                    "SELECT id FROM users WHERE id=?1"
                },
                [user],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Fault::Unauthorized)?;
        if p.as_ref().is_some_and(|p| p.user.id == user) {
            return Err(Fault::SelfRecovery.into());
        }
        tx.execute(
            "UPDATE users SET password_hash=NULL,auth_version=auth_version+1 WHERE id=?1",
            [&user],
        )?;
        for sql in [
            "DELETE FROM sessions WHERE user_id=?1",
            "DELETE FROM auth_passkeys WHERE user_id=?1",
            "DELETE FROM auth_totp WHERE user_id=?1",
            "DELETE FROM auth_oidc_identities WHERE user_id=?1",
            "DELETE FROM auth_remembered_devices WHERE user_id=?1",
            "DELETE FROM auth_client_passwords WHERE user_id=?1",
            "DELETE FROM auth_attempts WHERE user_id=?1",
            "DELETE FROM auth_desktop_requests WHERE user_id=?1 OR target_user_id=?1",
            "DELETE FROM auth_recovery WHERE user_id=?1",
        ] {
            tx.execute(sql, [&user])?;
        }
        tx.execute(
            "INSERT INTO auth_recovery VALUES(?1,?2,?3)",
            params![hash, user, now() + 600],
        )?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES(?1,'auth.recovery',?2,?3)",
            params![p.map(|p| p.user.id), user, now()],
        )?;
        tx.commit()?;
        Ok(user)
    })
    .await
}
pub(super) async fn recovery_account(db: &Database, hash: String) -> Result<Account> {
    db.read("authentication.recovery.read",move|c|Ok(c.query_row("SELECT u.id,u.username,u.password_hash,u.auth_version,NULL,0 FROM users u JOIN auth_recovery r ON r.user_id=u.id WHERE r.token_hash=?1 AND r.expires_at>?2",params![hash,now()],account_row).optional()?.ok_or(Fault::Unauthorized)?)).await
}
pub(super) async fn recovery_enroll(
    db: &Database,
    hash: String,
    password: Option<String>,
    passkey: Option<(String, String)>,
) -> Result<()> {
    db.write("authentication.recovery.enroll", move |c| {
        let tx = c.transaction()?;
        let user: String = tx
            .query_row(
                "DELETE FROM auth_recovery WHERE token_hash=?1 AND expires_at>?2 RETURNING user_id",
                params![hash, now()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Fault::Unauthorized)?;
        if let Some(password) = password {
            tx.execute(
                "UPDATE users SET password_hash=?1 WHERE id=?2",
                params![password, user],
            )?;
        }
        if let Some((key, credential)) = passkey {
            tx.execute(
                "INSERT INTO auth_passkeys VALUES(?1,?2,'Passkey',?3,?4)",
                params![key, user, credential, now()],
            )?;
        }
        tx.execute("DELETE FROM auth_attempts WHERE user_id=?1", [user])?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(super) struct DesktopRequest {
    pub key: String,
    pub secret: String,
    pub code: String,
    pub name: String,
    pub address: String,
}
pub(super) async fn desktop_start(
    db: &Database,
    request: DesktopRequest,
    target: Option<Principal>,
) -> Result<()> {
    db.write("authentication.desktop.begin", move |c| {
        let tx=c.transaction()?;
        if let Some(p)=&target {authorized(&tx,p,false)?;}
        tx.execute("DELETE FROM auth_desktop_requests WHERE expires_at<=?1",[now()])?;
        if tx.query_row("SELECT COUNT(*) FROM auth_desktop_requests",[],|r|r.get::<_,i64>(0))?>10000 {return Err(Fault::Conflict.into());}
        tx.execute("INSERT INTO auth_desktop_requests(id,secret_hash,code_hash,name,address,created_at,expires_at,target_user_id,target_session_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![request.key,request.secret,request.code,request.name,request.address,now(),now()+300,target.as_ref().map(|p|&p.user.id),target.as_ref().map(|p|&p.session_id)])?;
        tx.commit()?; Ok(())
    }).await
}
pub(super) struct DesktopInfo {
    pub name: String,
    pub verifying: Option<String>,
    pub address: String,
    pub created_at: i64,
}
fn desktop_ended() -> Fault {
    Fault::State(
        "desktop_request_ended",
        "This desktop request expired, was canceled or was already used. Start again from the desktop app.",
    )
}
pub(super) async fn desktop_info(db: &Database, key: String) -> Result<DesktopInfo> {
    db.read("authentication.desktop.info",move|c|Ok(c.query_row("SELECT d.name,u.username,d.address,d.created_at FROM auth_desktop_requests d LEFT JOIN users u ON u.id=d.target_user_id WHERE d.id=?1 AND d.expires_at>?2 AND d.user_id IS NULL AND d.failures<5",params![key,now()],|r|Ok(DesktopInfo{name:r.get(0)?,verifying:r.get(1)?,address:r.get(2)?,created_at:r.get(3)?})).optional()?.ok_or_else(desktop_ended)?)).await
}
pub(super) async fn desktop_approve(
    db: &Database,
    p: Principal,
    key: String,
    code: String,
) -> Result<()> {
    db.write("authentication.desktop.approve",move|c|{
        let tx=c.transaction()?;authorized(&tx,&p,true)?;
        let matches:bool=tx.query_row("SELECT code_hash=?2 FROM auth_desktop_requests WHERE id=?1 AND expires_at>?3 AND user_id IS NULL AND failures<5",params![key,code,now()],|r|r.get(0)).optional()?.ok_or_else(desktop_ended)?;
        if !matches {
            let failures:i64=tx.query_row("UPDATE auth_desktop_requests SET failures=failures+1 WHERE id=?1 RETURNING failures",[&key],|r|r.get(0))?;
            tx.commit()?;
            return Err(if failures>=5 {
                Fault::State("desktop_request_ended","Too many incorrect codes. Start again from the desktop app.")
            } else {
                Fault::Invalid("desktop_code_mismatch","That code doesn't match the one shown in the desktop app. Check it and try again.")
            }.into());
        }
        if tx.execute("UPDATE auth_desktop_requests SET user_id=?1,authorizer_session_id=?2,verified_at=(SELECT verified_at FROM sessions WHERE id=?2) WHERE id=?4 AND expires_at>?3 AND user_id IS NULL AND (target_user_id IS NULL OR target_user_id=?1) AND (target_session_id IS NULL OR EXISTS(SELECT 1 FROM sessions WHERE id=target_session_id AND user_id=?1 AND expires_at>?3))",params![p.user.id,p.session_id,now(),key])?!=1 {return Err(Fault::Unauthorized.into());}
        audit(&tx,&p,"auth.desktop.approved",&key)?;tx.commit()?;Ok(())
    }).await
}
pub(super) enum DesktopExchange {
    SignIn(SessionAuthorization, String),
    Verified,
}
pub(super) async fn desktop_exchange(
    db: &Database,
    key: String,
    secret: String,
    cancel: bool,
    principal: Option<Principal>,
) -> Result<Option<DesktopExchange>> {
    db.write("authentication.desktop.exchange",move|c|{
        let tx=c.transaction()?;
        if cancel {tx.execute("DELETE FROM auth_desktop_requests WHERE id=?1 AND secret_hash=?2",params![key,secret])?;tx.commit()?;return Ok(None);}
        let row=tx.query_row("SELECT d.user_id,d.authorizer_session_id,d.name,d.target_session_id,d.verified_at FROM auth_desktop_requests d JOIN sessions s ON s.id=d.authorizer_session_id AND s.user_id=d.user_id WHERE d.id=?1 AND d.secret_hash=?2 AND d.expires_at>?3 AND s.expires_at>?3 AND s.verified_at>=?3-300 AND d.verified_at>=?3-300",params![key,secret,now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?))).optional()?;
        let result=if let Some((user_id,session_id,name,target,verified_at))=row {
            let value=if let Some(target)=target {
                let p=principal.as_ref().ok_or(Fault::Unauthorized)?;
                authorized(&tx,p,false)?;
                if p.user.id!=user_id || p.session_id!=target {return Err(Fault::Unauthorized.into());}
                tx.execute("UPDATE sessions SET verified_at=?1 WHERE id=?2",params![verified_at,target])?;
                DesktopExchange::Verified
            } else {DesktopExchange::SignIn(SessionAuthorization::ApprovedSession{user_id,session_id},name)};
            tx.execute("DELETE FROM auth_desktop_requests WHERE id=?1",[&key])?;
            Some(value)
        } else {
            if !tx.query_row("SELECT EXISTS(SELECT 1 FROM auth_desktop_requests WHERE id=?1 AND secret_hash=?2 AND expires_at>?3 AND user_id IS NULL AND failures<5)",params![key,secret,now()],|r|r.get::<_,bool>(0))? {return Err(Fault::Unauthorized.into());}
            None
        };
        tx.commit()?;Ok(result)
    }).await
}
pub(super) async fn revoke_session(db: &Database, p: Principal, key: String) -> Result<()> {
    db.write("authentication.session.revoke", move |c| {
        let tx = c.transaction()?;
        authorized(&tx, &p, true)?;
        let device: Option<String> = tx
            .query_row(
                "SELECT remembered_device_id FROM sessions WHERE id=?1 AND (user_id=?2 OR ?3)",
                params![
                    key,
                    p.user.id,
                    p.user.role.allows(thelxinoe_core::Capability::ManageUsers)
                ],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(device) = device {
            tx.execute(
                "DELETE FROM sessions WHERE remembered_device_id=?1",
                [&device],
            )?;
            tx.execute("DELETE FROM auth_remembered_devices WHERE id=?1", [device])?;
        }
        tx.execute(
            "DELETE FROM sessions WHERE id=?1 AND (user_id=?2 OR ?3)",
            params![
                key,
                p.user.id,
                p.user.role.allows(thelxinoe_core::Capability::ManageUsers)
            ],
        )?;
        tx.execute("DELETE FROM auth_attempts WHERE session_id=?1", [&key])?;
        tx.execute(
            "DELETE FROM auth_desktop_requests WHERE authorizer_session_id=?1 OR target_session_id=?1",
            [&key],
        )?;
        audit(&tx, &p, "session.revoke", &key)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
