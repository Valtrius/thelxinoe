//! Database operations for operations.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn backup_command_read_playback_sessions(db: &Database) -> anyhow::Result<bool> {
    db.read("operations.backup_command_read_playback_sessions", |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running')",[now()],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn backup_command_write_audit(
    p: thelxinoe_core::Principal,
    db: &Database,
    restore: Option<String>,
) -> anyhow::Result<()> {
    db.write("operations.backup_command_write_audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,'server',?3)",
            params![
                p.user.id,
                if restore.is_some() {
                    "backup.restore"
                } else {
                    "backup.create"
                },
                now()
            ],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn change_user(
    db: &Database,
    key: String,
    input: UserChange,
    p: thelxinoe_core::Principal,
    hash: Option<String>,
) -> anyhow::Result<i32> {
    db.write("operations.change_user", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        crate::authentication::authorize_admin(&tx,&p)?;
        let current = tx
            .query_row("SELECT role FROM users WHERE id=?1", [&key], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        let Some(current) = current else {
            return Ok(404);
        };
        if current == "admin"
            && input.role == thelxinoe_core::Role::User
            && tx.query_row("SELECT COUNT(*) FROM users WHERE role='admin'", [], |r| {
                r.get::<_, i64>(0)
            })? <= 1
        {
            return Ok(409);
        }
        tx.execute(
            "UPDATE users SET role=?1,password_hash=COALESCE(?2,password_hash),auth_version=auth_version+1 WHERE id=?3",
            params![input.role.as_str(), hash, key],
        )?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1", [&key])?;
        tx.execute("DELETE FROM auth_remembered_devices WHERE user_id=?1",[&key])?;
        tx.execute("DELETE FROM auth_attempts WHERE user_id=?1",[&key])?;
        tx.execute("DELETE FROM auth_desktop_requests WHERE user_id=?1",[&key])?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.update',?2,?3)",
            params![p.user.id, key, now()],
        )?;
        tx.commit()?;
        Ok(200)
    })
    .await
}

pub(super) async fn delete_user(
    db: &Database,
    key: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Option<Vec<String>>> {
    db.write("operations.delete_user", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        crate::authentication::authorize_admin(&tx,&p)?;
        let current=tx.query_row("SELECT role FROM users WHERE id=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        if current.is_none(){return Ok(None);}
        let playbacks=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1")?.query_map([&key],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        // Provisioned services survive removal of the administrator who installed them.
        tx.execute("UPDATE stack_provisions SET actor_id=?1 WHERE actor_id=?2",params![p.user.id,key])?;
        tx.execute("DELETE FROM events WHERE user_id=?1",[&key])?;
        tx.execute("UPDATE retention_policies SET trigger_users=(SELECT COALESCE(json_group_array(value),'[]') FROM json_each(trigger_users) WHERE value<>?1),updated_at=?2 WHERE EXISTS(SELECT 1 FROM json_each(trigger_users) WHERE value=?1)",params![key,now()])?;
        tx.execute("UPDATE audit SET actor_id=NULL WHERE actor_id=?1",[&key])?;
        tx.execute("DELETE FROM users WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.delete',?2,?3)",params![p.user.id,key,now()])?;
        tx.commit()?;Ok(Some(playbacks))
    }).await
}

pub(super) async fn dashboard(db: &Database) -> anyhow::Result<Value> {
    db.read("operations.dashboard", |db|{
        let playback=db.prepare("SELECT p.id,u.username,COALESCE(m.title,'Online playback'),p.mode,p.state,p.position,p.duration FROM playback_sessions p JOIN users u ON u.id=p.user_id LEFT JOIN media m ON m.id=p.media_id WHERE p.state IN ('ready','playing','paused') AND p.updated_at>?1 ORDER BY p.updated_at DESC LIMIT 100")?.query_map([now()-120],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"mode":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"position":r.get::<_,f64>(5)?,"duration":r.get::<_,f64>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let errors=db.prepare("SELECT id,kind,completed_at FROM jobs WHERE state='failed' ORDER BY completed_at DESC LIMIT 50")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"at":r.get::<_,Option<i64>>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let services=db.prepare("SELECT kind,version,checked_at,error IS NULL FROM manager_services UNION ALL SELECT kind,version,checked_at,error IS NULL FROM support_services")?.query_map([],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"version":r.get::<_,String>(1)?,"checked_at":r.get::<_,i64>(2)?,"healthy":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let support=db.query_row("SELECT value FROM settings WHERE key='operations.support'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str::<Value>(&v).ok()).unwrap_or_else(||json!({"items":[]}));
        Ok(json!({"playback":playback,"errors":errors,"services":services,"support":support}))
    }).await
}

pub(super) async fn diagnostics(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Value> {
    db.write("operations.diagnostics", move|db|{
        let schema=db.pragma_query_value(None,"user_version",|r|r.get::<_,u32>(0))?;
        let jobs=db.prepare("SELECT state,COUNT(*) FROM jobs GROUP BY state")?.query_map([],|r|Ok(json!({"state":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let check=db.query_row("PRAGMA quick_check",[],|r|r.get::<_,String>(0))?=="ok";
        db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'diagnostics.export','server',?2)",params![p.user.id,now()])?;
        Ok(json!({"format":1,"version":thelxinoe_core::VERSION,"schema":schema,"database_ok":check,"jobs":jobs,"created_at":now()}))
    }).await
}

pub(super) async fn devices(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("operations.devices", |db|Ok(db.prepare("SELECT s.id,u.username,s.name,s.transport,s.last_seen FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.expires_at>?1 ORDER BY s.last_seen DESC LIMIT 500")?.query_map([now()],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"transport":r.get::<_,String>(3)?,"last_seen":r.get::<_,i64>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
