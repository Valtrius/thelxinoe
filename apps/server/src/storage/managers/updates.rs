//! Database operations for managers.updates.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn policy(
    db: &Database,
    key: String,
    input: Policy,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.updates.policy", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;tx.execute("INSERT INTO service_update_policy(service_id,policy,window_start,window_end) VALUES (?1,?2,?3,?4) ON CONFLICT(service_id) DO UPDATE SET policy=excluded.policy,window_start=excluded.window_start,window_end=excluded.window_end",params![key,input.policy,input.window_start,input.window_end])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.update.policy',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Value> {
    db.read("managers.updates.list", |db|{
        let policies=db.prepare("SELECT service_id,policy,window_start,window_end,candidate,error,checked_at FROM service_update_policy WHERE service_id<>'default'")?.query_map([],|r|Ok(json!({"service_id":r.get::<_,String>(0)?,"policy":r.get::<_,String>(1)?,"window_start":r.get::<_,u8>(2)?,"window_end":r.get::<_,u8>(3)?,"candidate":r.get::<_,Option<String>>(4)?,"error":r.get::<_,Option<String>>(5)?,"checked_at":r.get::<_,i64>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=db.prepare("SELECT id,service_id,state,candidate,error,created_at FROM service_updates ORDER BY created_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"service_id":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"candidate":r.get::<_,Option<String>>(3)?,"error":r.get::<_,Option<String>>(4)?,"created_at":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let services=db.prepare("SELECT id,kind FROM stack_provisions WHERE state='complete'")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"policies":policies,"items":items,"services":services,"timezone":crate::timezones::server_zone(db)?.name()}))
    }).await
}

pub(super) async fn enqueue(
    service: String,
    key: String,
    db: &Database,
    actor: Option<String>,
) -> anyhow::Result<bool> {
    db.write("managers.updates.enqueue", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let available=tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND state='complete' AND service_id IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM service_updates WHERE service_id=?1 AND state IN ('queued','submitting','preparing','snapshotting','preflight','queued-activate','activating','recovery-snapshot','isolated-live-validation','recovery-required','queued-recover'))",[&service],|r|r.get::<_,bool>(0))?;
        if !available{return Ok(false);}
        tx.execute("INSERT INTO service_updates(id,service_id,actor_id,state,created_at,updated_at,automatic) VALUES (?1,?2,?3,'queued',?4,?4,?5)",params![key,service,actor,now(),actor.is_none()])?;
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.update',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key,"action":"preflight"}).to_string(),format!("update:{key}:preflight"),now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.update.preflight',?2,?3)",params![actor,key,now()])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn queue_action(
    db: &Database,
    key: String,
    action: String,
    actor: Option<String>,
) -> anyhow::Result<bool> {
    db.write("managers.updates.queue_action", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT state FROM service_updates WHERE id=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        let allowed=if action=="activate" {current.as_deref()==Some("ready")} else {current.as_deref().is_some_and(|s|["blocked","recovery-required"].contains(&s))};
        if !allowed{return Ok(false);}
        tx.execute("UPDATE service_updates SET state=?1,error=NULL,actor_id=?2,updated_at=?3,automatic=?5 WHERE id=?4",params![format!("queued-{action}"),actor,now(),key,actor.is_none()])?;
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.update',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key,"action":action}).to_string(),format!("update:{key}:{action}:{}",id()),now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor,format!("service.update.{action}"),key,now()])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn progress(
    key: String,
    stage: String,
    db: &Database,
    candidate: Option<String>,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.updates.progress", move|db|{db.execute("UPDATE service_updates SET state=?1,candidate=COALESCE(?2,candidate),error=?3,updated_at=?4 WHERE id=?5",params![stage,candidate,error,now(),key])?;Ok(())}).await
}

pub(super) async fn idle_read_stack_provisions(
    provision: String,
    db: &Database,
) -> anyhow::Result<(String, String)> {
    db.read("managers.updates.idle_read_stack_provisions", move |db| {
        Ok(db.query_row(
            "SELECT kind,service_id FROM stack_provisions WHERE id=?1 AND state='complete'",
            [provision],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?)
    })
    .await
}

pub(super) async fn idle_read_playback_sessions(db: &Database) -> anyhow::Result<bool> {
    db.read("managers.updates.idle_read_playback_sessions", |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1)",[now()-120],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn run_job_read_service_updates(
    lookup: String,
    db: &Database,
) -> std::prelude::v1::Result<
    (
        bool,
        String,
        Option<String>,
        Option<u32>,
        Option<u32>,
        String,
    ),
    anyhow::Error,
> {
    db.read("managers.updates.run_job_read_service_updates", move|db|Ok(db.query_row("SELECT u.automatic,u.service_id,p.policy,p.window_start,p.window_end,u.state FROM service_updates u LEFT JOIN service_update_policy p ON p.service_id=u.service_id WHERE u.id=?1",[lookup],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<u32>>(3)?,r.get::<_,Option<u32>>(4)?,r.get::<_,String>(5)?)))?)).await
}

pub(super) async fn run_job_write_jobs(job_id: String, db: &Database) -> anyhow::Result<()> {
    db.write("managers.updates.run_job_write_jobs", move |db| {
        db.execute(
            "UPDATE jobs SET state='queued',available_at=?1,started_at=NULL WHERE id=?2",
            params![now() + 60, job_id],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn run_locked(
    lookup: String,
    db: &Database,
) -> anyhow::Result<(String, String, bool)> {
    db.read("managers.updates.run_locked", move|db|Ok(db.query_row("SELECT service_id,state,automatic=1 OR EXISTS(SELECT 1 FROM users WHERE id=actor_id AND role='admin') FROM service_updates WHERE id=?1",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?)).await
}

pub(super) async fn reconnect(
    provision: String,
    container: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.updates.reconnect", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (kind, service) = tx.query_row(
            "SELECT kind,service_id FROM stack_provisions WHERE id=?1",
            [&provision],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?;
        let table = if ["radarr", "sonarr", "lidarr"].contains(&kind.as_str()) {
            "manager_services"
        } else {
            "support_services"
        };
        tx.execute(
            &format!("UPDATE {table} SET container_id=?1 WHERE id=?2"),
            params![container, service],
        )?;
        tx.execute(
            "UPDATE stack_provisions SET container_id=?1 WHERE id=?2",
            params![container, provision],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn check_releases_read_stack_provisions(
    db: &Database,
) -> std::prelude::v1::Result<
    Vec<(
        String,
        String,
        Option<String>,
        Option<u32>,
        Option<u32>,
        i64,
    )>,
    anyhow::Error,
> {
    db.read("managers.updates.check_releases_read_stack_provisions", |db|Ok(db.prepare("SELECT s.id,s.kind,p.policy,p.window_start,p.window_end,COALESCE(p.checked_at,0) FROM stack_provisions s LEFT JOIN service_update_policy p ON p.service_id=s.id WHERE s.state='complete'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<u32>>(3)?,r.get::<_,Option<u32>>(4)?,r.get::<_,i64>(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn check_releases_write_service_update_policy(
    start: u32,
    end: u32,
    key: String,
    found: Option<String>,
    mode: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.updates.check_releases_write_service_update_policy", move|db|{db.execute("INSERT INTO service_update_policy(service_id,policy,window_start,window_end,checked_at,candidate,error) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(service_id) DO UPDATE SET checked_at=excluded.checked_at,candidate=excluded.candidate,error=excluded.error",params![key,mode,start,end,now(),found,if found.is_none(){Some("Stable release discovery unavailable")}else{None}])?;Ok(())}).await
}

pub(super) async fn run(
    db: &Database,
) -> anyhow::Result<Vec<(String, Option<String>, Option<u32>, Option<u32>)>> {
    db.read("managers.updates.run", |db|Ok(db.prepare("SELECT u.id,p.policy,p.window_start,p.window_end FROM service_updates u LEFT JOIN service_update_policy p ON p.service_id=u.service_id WHERE u.state='ready'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<u32>>(2)?,r.get::<_,Option<u32>>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
