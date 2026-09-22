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
            "UPDATE users SET role=?1,password_hash=COALESCE(?2,password_hash) WHERE id=?3",
            params![input.role.as_str(), hash, key],
        )?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1", [&key])?;
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

pub(super) async fn notifications(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Vec<Value>> {
    db.read("operations.notifications", move|db| Ok(db.prepare("SELECT id,severity,message,created_at,read_at FROM notifications WHERE user_id=?1 ORDER BY created_at DESC,id LIMIT 200")?.query_map([p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"severity":r.get::<_,String>(1)?,"message":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"read_at":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn read(db: &Database, key: String, user: String) -> anyhow::Result<()> {
    db.write("operations.read", move |db| {
        db.execute(
            "UPDATE notifications SET read_at=?1 WHERE user_id=?2 AND (id=?3 OR ?3='all')",
            params![now(), user, key],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn observe(db: &Database) -> anyhow::Result<Vec<String>> {
    db.write("operations.observe", |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Fixed messages prevent private media titles, URLs and provider errors entering shared notifications.
        let observations=tx.prepare("SELECT 'job:'||id,'error','A background job failed. Open administration for details.' FROM jobs WHERE state='failed' AND completed_at>?1
          UNION ALL SELECT 'service:'||id,'error','An integration is unavailable. Check service health.' FROM manager_services WHERE error IS NOT NULL
          UNION ALL SELECT 'support:'||id,'error','A support service needs attention. Check download and indexer health.' FROM support_services WHERE error IS NOT NULL
          UNION ALL SELECT 'update:'||id||':'||state,'warning','A service update needs attention. Open service updates.' FROM service_updates WHERE state IN ('blocked','failed','incompatible','unable-to-verify','runtime-failure')
          UNION ALL SELECT 'health:'||json_extract(j.value,'$.id'),'warning','An indexer or download service needs attention. Open Support services.' FROM settings s,json_each(s.value,'$.items') j WHERE s.key='operations.support' AND json_extract(j.value,'$.problem')=1
          UNION ALL SELECT 'product-release:'||json_extract(value,'$.version'),'info','A signed Thelxinoe release is available. Open Product updates.' FROM settings WHERE key='product.release' AND json_extract(value,'$.version') IS NOT NULL
          UNION ALL SELECT 'product-update:'||json_extract(j.value,'$.id')||':'||json_extract(j.value,'$.stage'),'warning','A Thelxinoe update needs attention. Open Product updates.' FROM settings s,json_each(s.value,'$.items') j WHERE s.key='product.controller' AND json_extract(j.value,'$.stage') IN ('blocked','recovered','recovery-required','runtime-failure')")?.query_map([now()-7*86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let admins=tx.prepare("SELECT id FROM users WHERE role='admin'")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed=vec![];
        let previous=tx.prepare("SELECT source,occurrence,active FROM notification_conditions")?.query_map([],|r|Ok((r.get::<_,String>(0)?,(r.get::<_,i64>(1)?,r.get::<_,bool>(2)?))))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?;
        tx.execute("UPDATE notification_conditions SET active=0",[])?;
        for (source,severity,message) in &observations {
            let epoch=previous.get(source).map_or(1,|(n,active)|if *active{*n}else{n+1});
            tx.execute("INSERT INTO notification_conditions VALUES (?1,?2,1) ON CONFLICT(source) DO UPDATE SET occurrence=excluded.occurrence,active=1",params![source,epoch])?;
            let key=format!("{source}:{epoch}");
            for user in &admins {if tx.execute("INSERT OR IGNORE INTO notifications VALUES (?1,?2,?3,?4,?5,?6,NULL)",params![id(),user,key,severity,message,now()])?>0 && !changed.contains(user){changed.push(user.clone());}}
        }
        let personal=tx.prepare("SELECT user_id,'request:'||id||':'||state||':'||updated_at,'Your media request has changed. Open Requests for details.' FROM acquisition_requests WHERE updated_at>?1 AND state IN ('available','denied','failed','requested') UNION ALL SELECT user_id,'account:'||provider||':'||generation,'A linked account needs you to sign in again.' FROM online_accounts WHERE status='reconnect_required'")?.query_map([now()-7*86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (user,source,message) in personal {if tx.execute("INSERT OR IGNORE INTO notifications VALUES (?1,?2,?3,'info',?4,?5,NULL)",params![id(),user,source,message,now()])?>0 && !changed.contains(&user){changed.push(user);}}
        tx.execute("DELETE FROM notifications WHERE created_at<?1",[now()-90*86400])?;
        tx.commit()?;Ok(changed)
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
