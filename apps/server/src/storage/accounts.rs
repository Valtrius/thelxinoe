//! Database operations for accounts.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn setup_status(db: &Database) -> anyhow::Result<i64> {
    db.read("accounts.setup_status", |c| {
        Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))?)
    })
    .await
}

pub(super) async fn respond_session(
    db: &Database,
    user_id: String,
) -> anyhow::Result<thelxinoe_core::User> {
    db.read("accounts.respond_session", move |db| {
        Ok(db.query_row(
            "SELECT id,username,role,timezone FROM user_profiles WHERE id=?1",
            [user_id],
            user_row,
        )?)
    })
    .await
}

pub(super) async fn setup(
    db: &Database,
    hash: String,
    uid: String,
    username: String,
) -> anyhow::Result<bool> {
    db
        .write("accounts.setup", move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? != 0 {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO users(id,username,password_hash,role,created_at) VALUES (?1,?2,?3,'admin',?4)",
                params![uid, username, hash, now()],
            )?;
            tx.execute(
                "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'setup',?1,?2)",
                params![uid, now()],
            )?;
            tx.commit()?;
            Ok(true)
        })
        .await
}

pub(super) async fn check_credentials(
    username: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("accounts.check_credentials", move |db| {
        Ok(db
            .query_row(
                "SELECT id,password_hash FROM users WHERE username=?1 AND password_hash IS NOT NULL",
                [username],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn allow_password_attempt(db: &Database, address: String) -> anyhow::Result<bool> {
    db.write("accounts.allow_password_attempt", move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM login_attempts WHERE window_start<?1",[now()-900])?;
        let count:i64=tx.query_row("INSERT INTO login_attempts VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=count+1 RETURNING count",params![address,now()],|r|r.get(0))?;
        tx.commit()?;Ok(count<=20)
    }).await
}

pub(super) async fn account_failures(db: &Database, key: String) -> anyhow::Result<i64> {
    db.read("accounts.account_failures", move |db| {
        Ok(db
            .query_row(
                "SELECT count FROM login_attempts WHERE address=?1 AND window_start>=?2",
                params![key, now() - 900],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    })
    .await
}

pub(super) async fn record_account_failure(db: &Database, key: String) -> anyhow::Result<()> {
    db.write("accounts.record_account_failure", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM login_attempts WHERE window_start<?1", [now() - 900])?;
        tx.execute(
            "INSERT INTO login_attempts VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=count+1",
            params![key, now()],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn logout(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<()> {
    db.write("accounts.logout", move |db| {
        db.execute("DELETE FROM sessions WHERE id=?1", [p.session_id])?;
        Ok(())
    })
    .await
}

pub(super) async fn sessions(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Vec<thelxinoe_auth::Session>> {
    db.read("accounts.sessions", move |db|{
        let mut query=db.prepare("SELECT id,name,transport,created_at,expires_at,last_seen,remembered_device_id IS NOT NULL FROM sessions WHERE user_id=?1 AND expires_at>?2 ORDER BY last_seen DESC")?;
        Ok(query.query_map(params![p.user.id,now()],|r|Ok(thelxinoe_auth::Session{id:r.get(0)?,name:r.get(1)?,transport:r.get(2)?,created_at:r.get(3)?,expires_at:r.get(4)?,last_seen:r.get(5)?,remembered:r.get(6)?}))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await
}

pub(super) async fn users(db: &Database) -> anyhow::Result<Vec<thelxinoe_core::User>> {
    db.read("accounts.users", |db| {
        Ok(db
            .prepare("SELECT id,username,role,timezone FROM user_profiles ORDER BY username")?
            .query_map([], user_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    })
    .await
}

pub(super) async fn create_user(
    db: &Database,
    user: NewUser,
    p: thelxinoe_core::Principal,
    hash: String,
) -> anyhow::Result<Option<String>> {
    db.write("accounts.create_user", move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;crate::authentication::authorize_admin(&tx,&p)?;let uid=id();
        let n=tx.execute("INSERT OR IGNORE INTO users(id,username,password_hash,role,created_at) VALUES (?1,?2,?3,?4,?5)",params![uid,user.username,hash,user.role.as_str(),now()])?;
        if n==0{return Ok(None);}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.create',?2,?3)",params![p.user.id,uid,now()])?;tx.commit()?;Ok(Some(uid))
    }).await
}

fn settings_view(db: &rusqlite::Connection) -> anyhow::Result<SettingsView> {
    let setting = |key: &str, default: &str| -> anyhow::Result<String> {
        Ok(db
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .unwrap_or_else(|| default.into()))
    };
    Ok(SettingsView {
        timezone: setting("timezone", "UTC")?,
        time_format: setting("time_format", "24h")?,
        transcoding: crate::transcoding::Policy {
            video_transcoding: setting("video_transcoding", "true")? == "true",
            audio_transcoding: setting("audio_transcoding", "true")? == "true",
        },
    })
}
pub(super) async fn settings(db: &Database) -> anyhow::Result<SettingsView> {
    db.read("accounts.settings", settings_view).await
}

pub(super) async fn save_settings(
    db: &Database,
    settings: Settings,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Option<SettingsView>> {
    db.write("accounts.save_settings", move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let authorized=tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.id=?1 AND s.user_id=?2 AND s.expires_at>?3 AND u.role='admin')",params![p.session_id,p.user.id,now()],|r|r.get::<_,bool>(0))?;
        if !authorized {return Ok(None);}
        if let Some(timezone)=settings.timezone {
            tx.execute("INSERT INTO settings VALUES ('timezone',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[timezone])?;
        }
        if let Some(format)=settings.time_format {
            tx.execute("INSERT INTO settings VALUES ('time_format',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[format])?;
        }
        for (key, value) in [("video_transcoding", settings.video_transcoding), ("audio_transcoding", settings.audio_transcoding)] {
            if let Some(value) = value {
                tx.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()])?;
            }
        }
        let current=settings_view(&tx)?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'settings.update','server',?2)",params![p.user.id,now()])?;
        tx.commit()?;
        Ok(Some(current))
    }).await
}
