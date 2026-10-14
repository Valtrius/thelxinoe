//! Database operations for managers.stack.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn list(db: &Database) -> anyhow::Result<Value> {
    db.read("managers.stack.list", |db|Ok(json!(db.prepare("SELECT id,kind,state,host_port,container_id,service_id,error,native_url,origin,operation,hold,transfer_id FROM stack_provisions ORDER BY created_at")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"host_port":r.get::<_,Option<u16>>(3)?,"container_id":r.get::<_,Option<String>>(4)?,"service_id":r.get::<_,Option<String>>(5)?,"error":r.get::<_,Option<String>>(6)?,"native_url":r.get::<_,String>(7)?,"origin":r.get::<_,String>(8)?,"operation":r.get::<_,Option<String>>(9)?,"hold":r.get::<_,Option<String>>(10)?,"transfer_id":r.get::<_,Option<String>>(11)?})))?.collect::<rusqlite::Result<Vec<_>>>()?))).await
}

/// A fresh installation is Owned from the start and held until setup completes.
pub(super) async fn install(
    db: &Database,
    input: Install,
    p: thelxinoe_core::Principal,
    key: String,
    credential: Option<Vec<u8>>,
) -> anyhow::Result<bool> {
    db.write("managers.stack.install", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;if tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1 UNION ALL SELECT 1 FROM manager_services WHERE kind=?1 AND enabled=1 UNION ALL SELECT 1 FROM support_services WHERE kind=?1 UNION ALL SELECT 1 FROM recyclarr_connections WHERE ?1='recyclarr' UNION ALL SELECT 1 FROM service_transfers WHERE kind=?1 AND state NOT IN ('review','complete','cancelled','returned'))",[&input.kind],|r|r.get::<_,bool>(0))?{return Ok(false);}tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at,native_url,hold) VALUES (?1,?2,?3,?4,?5,'queued',?6,?6,?7,'setup')",params![key,input.kind,p.user.id,input.host_port,credential,now(),input.native_url])?;tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'stack.install',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(true)}).await
}

pub(super) async fn action_read_service(
    db: &Database,
    c: String,
) -> anyhow::Result<Option<(String, String)>> {
    db.read("managers.stack.action_read_service", move |db| {
        Ok(db
            .query_row(
                "SELECT kind,id FROM manager_services WHERE container_id=?1 AND enabled=1 UNION ALL SELECT kind,id FROM support_services WHERE container_id=?1",
                [c],
                |r| Ok((r.get(0)?,r.get(1)?)),
            )
            .optional()?)
    })
    .await
}

/// A replaced or restarted Owned container is held again until its API is reverified. Every
/// Owned service, installed or transferred, gets the same durable verification job.
pub(super) async fn action_write_stack_provisions(
    db: &Database,
    key: String,
    container: String,
) -> anyhow::Result<()> {
    db.write("managers.stack.action_write_stack_provisions", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (kind, integration, previous, state): (String, Option<String>, Option<String>, String) = tx.query_row(
            "SELECT kind,service_id,container_id,state FROM stack_provisions WHERE id=?1", [&key],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        anyhow::ensure!(state != "retiring", "Installation is being retired");
        let replaced = previous.as_deref() != Some(container.as_str());
        if replaced && let Some(integration) = &integration {
            let table = if matches!(kind.as_str(), "radarr"|"sonarr"|"lidarr") { "manager_services" } else { "support_services" };
            tx.execute(&format!("UPDATE {table} SET container_id=?1,generation=?2,checked_at=0,operational=0,verified=0,error='Verifying replacement API' WHERE id=?3"), params![container,id(),integration])?;
        }
        tx.execute("UPDATE stack_provisions SET container_id=?1,updated_at=?2 WHERE id=?3", params![container,now(),key])?;
        if state == "blocked" || replaced {
            tx.execute("UPDATE stack_provisions SET state='connecting',hold='setup',error=NULL WHERE id=?1",[&key])?;
            if tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1,started_at=NULL WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2 AND state IN ('failed','complete')",params![now(),key])? == 0 {
                // A transferred service never had an installation job; one that is already queued keeps it.
                tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) SELECT ?1,'stack.install',?2,?3,'queued',?4,?4 WHERE NOT EXISTS(SELECT 1 FROM jobs WHERE kind='stack.install' AND json_extract(payload,'$.id')=?5)",params![id(),json!({"id":key}).to_string(),format!("stack:{key}"),now(),key])?;
            }
        }tx.commit()?;Ok(())}).await
}

pub(super) async fn begin_retirement(
    db: &Database,
    key: String,
    operation: String,
) -> anyhow::Result<bool> {
    db.write("managers.stack.begin_retirement", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ready: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND state IN ('complete','blocked','retiring') AND (hold IS NULL OR hold='setup') AND (operation IS NULL OR operation=?2)) AND NOT EXISTS(SELECT 1 FROM jobs WHERE kind='stack.install' AND json_extract(payload,'$.id')=?1 AND state IN ('queued','running')) AND NOT EXISTS(SELECT 1 FROM service_updates WHERE service_id=?1 AND state NOT IN ('committed','rolled-back','blocked'))", params![key,operation], |r|r.get(0))?;
        if ready {
            tx.execute("UPDATE stack_provisions SET state='retiring',updated_at=?1,operation=?3 WHERE id=?2",params![now(),key,operation])?;
            // Normal workers stop before the controller removes or retires the container.
            tx.execute("UPDATE manager_services SET operational=0 WHERE id=(SELECT service_id FROM stack_provisions WHERE id=?1)",[&key])?;
            tx.execute("UPDATE support_services SET operational=0 WHERE id=(SELECT service_id FROM stack_provisions WHERE id=?1)",[&key])?;
        }
        tx.commit()?;
        Ok(ready)
    }).await
}

pub(super) async fn retire(
    db: &Database,
    key: String,
    actor: String,
    remove: bool,
) -> anyhow::Result<()> {
    db.write("managers.stack.retire", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let integration: Option<(String,Option<String>)> = tx.query_row(
            "SELECT kind,service_id FROM stack_provisions WHERE id=?1 AND state='retiring'", [&key], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((kind, Some(service))) = integration {
            let retain_missing = if remove { "false" } else { "true" };
            tx.execute("UPDATE settings SET value=json_set(value,'$.enabled',json('false'),'$.cleanup',json(CASE WHEN json_extract(value,'$.source')=?1 THEN 'false' ELSE 'true' END),'$.state',CASE WHEN json_extract(value,'$.source')=?1 THEN 'disconnected' ELSE 'disconnecting' END,'$.error',NULL,'$.next_attempt',0,'$.retain_missing',json(?2)) WHERE key LIKE 'services.connection.%' AND (json_extract(value,'$.source')=?1 OR json_extract(value,'$.target')=?1)",params![&service,retain_missing])?;
            if matches!(kind.as_str(),"radarr"|"sonarr"|"lidarr") {
                // Keep historical requests/bindings, but stop acquisition and
                // leave previously claimed media unresolved until reviewed.
                tx.execute("UPDATE manager_services SET enabled=0,operational=0,error='Installation retired',checked_at=0 WHERE id=?1",[&service])?;
                tx.execute("UPDATE acquisition_requests SET state='cancelled',error='Installation retired',updated_at=?1 WHERE service_id=?2 AND state IN ('pending','approved','adding','searching','uncertain')",params![now(),service])?;
                tx.execute("UPDATE media_files SET ownership='unresolved' WHERE id IN (SELECT file_id FROM manager_bindings WHERE service_id=?1)",[&service])?;
            } else {
                tx.execute("DELETE FROM support_services WHERE id=?1",[&service])?;
            }
            if remove {
                // Retain the identity referenced by acquisition history, but erase
                // connection credentials and preferences from the removed service.
                tx.execute("UPDATE manager_services SET credential=X'',defaults='{}',container_id='removed:'||id,port=0,media_source='',version='',evidence='{}' WHERE id=?1",[&service])?;
                tx.execute("DELETE FROM settings WHERE key LIKE 'services.connection.%' AND json_extract(value,'$.source')=?1",[&service])?;
            }
        }
        if remove {
            tx.execute("DELETE FROM service_updates WHERE service_id=?1",[&key])?;
        }
        tx.execute("DELETE FROM service_update_policy WHERE service_id=?1",[&key])?;
        tx.execute("UPDATE jobs SET state='complete',error=NULL WHERE kind='stack.install' AND json_extract(payload,'$.id')=?1",[&key])?;
        tx.execute("UPDATE jobs SET state='complete',error='Installation retired' WHERE kind='recyclarr.sync' AND json_extract(payload,'$.id') IN (SELECT id FROM recyclarr_runs WHERE provision_id=?1)",[&key])?;
        tx.execute("DELETE FROM stack_provisions WHERE id=?1 AND state='retiring'",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor,if remove {"stack.remove"} else {"stack.retire"},key,now()])?;
        tx.commit()?;
        Ok(())
    }).await
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

pub(super) struct Provision {
    pub(super) kind: String,
    pub(super) actor: String,
    pub(super) host_port: Option<u16>,
    pub(super) credential: Option<Vec<u8>>,
    pub(super) state: String,
    pub(super) service: Option<String>,
    pub(super) origin: String,
    pub(super) native_url: String,
    pub(super) hold: Option<String>,
}
pub(super) async fn provision_read_stack_provisions(
    lookup: String,
    db: &Database,
) -> anyhow::Result<Provision> {
    db.read("managers.stack.provision_read_stack_provisions", move|db|Ok(db.query_row("SELECT kind,actor_id,host_port,credential,state,service_id,origin,native_url,hold FROM stack_provisions WHERE id=?1",[lookup],|r|Ok(Provision{kind:r.get(0)?,actor:r.get(1)?,host_port:r.get(2)?,credential:r.get(3)?,state:r.get(4)?,service:r.get(5)?,origin:r.get(6)?,native_url:r.get(7)?,hold:r.get(8)?}))?)).await
}

pub(super) async fn login(key: String, db: &Database) -> anyhow::Result<Option<(String, String)>> {
    db.read("managers.stack.login", move |db| {
        Ok(db
            .query_row(
                "SELECT kind,service_id FROM stack_provisions WHERE id=?1 AND service_id IS NOT NULL",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    })
    .await
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
    db.write("managers.stack.provision_write_stack_provisions", move|db|{db.execute("UPDATE stack_provisions SET service_id=?1,state='connecting',error=NULL,updated_at=?2 WHERE id=?3 AND hold='setup'",params![registered["id"].as_str(),now(),key])?;Ok(())}).await
}

/// Setup completion and operational-hold release commit together.
pub(super) async fn complete_setup(db: &Database, key: String) -> anyhow::Result<()> {
    db.write("managers.stack.complete_setup", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (kind, service, container, origin): (String, Option<String>, Option<String>, String) = tx.query_row(
            "SELECT kind,service_id,container_id,origin FROM stack_provisions WHERE id=?1 AND state='connecting' AND hold='setup'",
            [&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        if let Some(service) = &service {
            let table = if matches!(kind.as_str(), "radarr"|"sonarr"|"lidarr") { "manager_services" } else { "support_services" };
            anyhow::ensure!(tx.execute(&format!("UPDATE {table} SET operational=1,verified=1,error=NULL WHERE id=?1 AND container_id IS ?2"), params![service, container])? == 1, "Service changed during setup");
        } else {
            anyhow::ensure!(kind == "recyclarr", "Setup has no registered service");
        }
        tx.execute("UPDATE stack_provisions SET state='complete',hold=NULL,error=NULL,updated_at=?1 WHERE id=?2", params![now(), key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,?1,?2,?3)", params![if origin == "installed" {"stack.setup.complete"} else {"stack.reverified"}, key, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn retry(db: &Database, key: String) -> anyhow::Result<bool> {
    db.write("managers.stack.retry", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ready=tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions p JOIN jobs j ON json_extract(j.payload,'$.id')=p.id WHERE p.id=?1 AND p.state='blocked' AND p.hold='setup' AND j.kind='stack.install' AND j.state='failed')",[&key],|r|r.get::<_,bool>(0))?;
        if !ready {return Ok(false);}
        tx.execute("UPDATE stack_provisions SET state=CASE WHEN container_id IS NULL THEN 'queued' ELSE 'connecting' END,error=NULL,updated_at=?1 WHERE id=?2",params![now(),key])?;
        tx.execute("UPDATE jobs SET state='queued',error=NULL,available_at=?1 WHERE kind='stack.install' AND json_extract(payload,'$.id')=?2",params![now(),key])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn kind(db: &Database, key: String) -> anyhow::Result<Option<String>> {
    db.read("managers.stack.kind", move |db| {
        Ok(db
            .query_row(
                "SELECT kind FROM stack_provisions WHERE id=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}

pub(super) struct Owned {
    pub(super) kind: String,
    pub(super) hold: Option<String>,
}
pub(super) async fn owned(db: &Database, key: String) -> anyhow::Result<Option<Owned>> {
    db.read("managers.stack.owned", move |db| {
        Ok(db
            .query_row(
                "SELECT kind,hold FROM stack_provisions WHERE id=?1",
                [key],
                |r| {
                    Ok(Owned {
                        kind: r.get(0)?,
                        hold: r.get(1)?,
                    })
                },
            )
            .optional()?)
    })
    .await
}
