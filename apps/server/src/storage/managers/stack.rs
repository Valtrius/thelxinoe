//! Database operations for managers.stack.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn list(db: &Database) -> anyhow::Result<Value> {
    db.read("managers.stack.list", |db|Ok(json!(db.prepare("SELECT id,kind,state,host_port,container_id,service_id,error,native_url,origin FROM stack_provisions ORDER BY created_at")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"host_port":r.get::<_,u16>(3)?,"container_id":r.get::<_,Option<String>>(4)?,"service_id":r.get::<_,Option<String>>(5)?,"error":r.get::<_,Option<String>>(6)?,"native_url":r.get::<_,String>(7)?,"origin":r.get::<_,String>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?))).await
}

pub(super) async fn install(
    db: &Database,
    input: Install,
    p: thelxinoe_core::Principal,
    key: String,
    credential: Vec<u8>,
) -> anyhow::Result<bool> {
    db.write("managers.stack.install", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1 UNION ALL SELECT 1 FROM manager_services WHERE kind=?1 UNION ALL SELECT 1 FROM support_services WHERE kind=?1)",[&input.kind],|r|r.get::<_,bool>(0))?{return Ok(false);}tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at,native_url) VALUES (?1,?2,?3,?4,?5,'queued',?6,?6,?7)",params![key,input.kind,p.user.id,input.host_port,credential,now(),input.native_url])?;tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.install',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(true)}).await
}

pub(super) async fn action_read_manager_services(
    db: &Database,
    c: String,
) -> anyhow::Result<Option<String>> {
    db.read("managers.stack.action_read_manager_services", move |db| {
        Ok(db
            .query_row(
                "SELECT id FROM manager_services WHERE container_id=?1",
                [c],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn action_write_stack_provisions(
    db: &Database,
    key: String,
) -> anyhow::Result<()> {
    db.write("managers.stack.action_write_stack_provisions", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND state='blocked')",[&key],|r|r.get::<_,bool>(0))? {
            tx.execute("UPDATE stack_provisions SET state='connecting',error=NULL WHERE id=?1",[&key])?;
            tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1 WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2 AND state IN ('failed','complete')",params![now(),key])?;
        }tx.commit()?;Ok(())}).await
}

pub(super) async fn action_write_audit(
    db: &Database,
    key: String,
    input: Action,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.stack.action_write_audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![p.user.id, format!("stack.{}", input.action), key, now()],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn progress(
    key: String,
    stage: String,
    db: &Database,
    container: Option<String>,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.stack.progress", move|db|{db.execute("UPDATE stack_provisions SET state=?1,container_id=COALESCE(?2,container_id),error=?3,updated_at=?4 WHERE id=?5",params![stage,container,error,now(),key])?;Ok(())}).await
}

pub(super) async fn provision_read_stack_provisions(
    lookup: String,
    db: &Database,
) -> std::prelude::v1::Result<
    (
        String,
        String,
        u16,
        Vec<u8>,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ),
    anyhow::Error,
> {
    db.read("managers.stack.provision_read_stack_provisions", move|db|Ok(db.query_row("SELECT kind,actor_id,host_port,credential,state,container_id,service_id,origin,native_url FROM stack_provisions WHERE id=?1",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u16>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?)))?)).await
}

pub(super) async fn provision_read_users(actor: String, db: &Database) -> anyhow::Result<bool> {
    db.read("managers.stack.provision_read_users", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND role='admin')",
            [actor],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn provision_write(
    service_id: String,
    container: String,
    kind: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.stack.provision_write", move |db| {
        let table = if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            "manager_services"
        } else {
            "support_services"
        };
        db.execute(
            &format!("UPDATE {table} SET container_id=?1,generation=?2 WHERE id=?3"),
            params![container, id(), service_id],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn provision_read_manager_services(
    integration: String,
    db: &Database,
) -> anyhow::Result<String> {
    db.read("managers.stack.provision_read_manager_services", move|db|Ok(db.query_row("SELECT name FROM manager_services WHERE id=?1 UNION ALL SELECT name FROM support_services WHERE id=?1",[integration],|r|r.get::<_,String>(0))?)).await
}

pub(super) async fn provision_write_stack_provisions(
    registered: Value,
    key: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.stack.provision_write_stack_provisions", move|db|{db.execute("UPDATE stack_provisions SET service_id=?1,state='connecting',error=NULL,updated_at=?2 WHERE id=?3",params![registered["id"].as_str(),now(),key])?;Ok(())}).await
}

pub(super) async fn adopt_preview(
    db: &Database,
    key: String,
) -> anyhow::Result<Option<(String, String, u16)>> {
    db.read("managers.stack.adopt_preview", move|db|Ok(db.query_row("SELECT kind,container_id,port FROM manager_services WHERE id=?1 UNION ALL SELECT kind,container_id,port FROM support_services WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u16>(2)?))).optional()?)).await
}

pub(super) async fn restore_original_read_stack_provisions(
    db: &Database,
    lookup: String,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("managers.stack.restore_original_read_stack_provisions", move|db|Ok(db.query_row("SELECT p.kind,p.service_id FROM stack_provisions p WHERE p.id=?1 AND p.origin='adopted' AND p.state='blocked' AND EXISTS(SELECT 1 FROM jobs j WHERE j.kind='stack.install' AND json_extract(j.payload,'$.id')=p.id AND j.state='failed')",[lookup],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await
}

pub(super) async fn restore_original_write_jobs(
    db: &Database,
    key: String,
    actor: thelxinoe_core::Principal,
    kind: String,
    integration: String,
    container: String,
) -> anyhow::Result<()> {
    db.write("managers.stack.restore_original_write_jobs", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let table=if matches!(kind.as_str(),"radarr"|"sonarr"|"lidarr"){"manager_services"}else{"support_services"};
        tx.execute(&format!("UPDATE {table} SET container_id=?1,generation=?2 WHERE id=?3"),params![container,id(),integration])?;
        tx.execute("UPDATE jobs SET state='complete',error=NULL WHERE kind='stack.install' AND json_extract(payload,'$.id')=?1",[&key])?;
        tx.execute("DELETE FROM stack_provisions WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.restore-original',?2,?3)",params![actor.user.id,key,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn adopt_read_manager_services(
    db: &Database,
    service_id: String,
) -> anyhow::Result<Option<(String, String, String)>> {
    db.read("managers.stack.adopt_read_manager_services", move|db|Ok(db.query_row("SELECT kind,container_id,'' FROM manager_services WHERE id=?1 UNION ALL SELECT kind,container_id,native_url FROM support_services WHERE id=?1",[service_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?)).await
}

pub(super) async fn adopt_write_stack_provisions(
    db: &Database,
    input: Adopt,
    p: thelxinoe_core::Principal,
    item: (String, String, String),
    key: String,
    credential: Vec<u8>,
) -> anyhow::Result<bool> {
    db.write("managers.stack.adopt_write_stack_provisions", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1)",[&item.0],|r|r.get::<_,bool>(0))?{return Ok(false);}tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,origin,created_at,updated_at,native_url) VALUES (?1,?2,?3,0,?4,'queued',?5,?6,'adopted',?7,?7,?8)",params![key,item.0,p.user.id,credential,item.1,input.service_id,now(),item.2])?;tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.adopt',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(true)}).await
}

pub(super) async fn retry(db: &Database, key: String) -> anyhow::Result<bool> {
    db.write("managers.stack.retry", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ready=tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions p JOIN jobs j ON json_extract(j.payload,'$.id')=p.id WHERE p.id=?1 AND p.state='blocked' AND j.kind='stack.install' AND j.state='failed')",[&key],|r|r.get::<_,bool>(0))?;
        if !ready {return Ok(false);}
        tx.execute("UPDATE stack_provisions SET state='queued',error=NULL,updated_at=?1 WHERE id=?2",params![now(),key])?;
        tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1 WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2",params![now(),key])?;
        tx.commit()?;Ok(true)
    }).await
}
