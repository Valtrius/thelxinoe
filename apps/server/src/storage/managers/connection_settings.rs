use super::*;
use thelxinoe_database::Database;

pub(super) async fn load(db: &Database, key: String) -> anyhow::Result<Option<Record>> {
    db.read("managers.connection_settings.load",move |db| {
        Ok(db.query_row("SELECT kind,container_id,credential FROM manager_services WHERE id=?1 AND enabled=1 UNION ALL SELECT kind,container_id,credential FROM support_services WHERE id=?1",[key],|r|Ok(Record{kind:r.get(0)?,container:r.get(1)?,credential:r.get(2)?})).optional()?)
    }).await
}
#[expect(clippy::too_many_arguments)]
pub(super) async fn save(
    db: &Database,
    key: String,
    record: Record,
    (container, container_name): (String, String),
    port: u16,
    url_base: String,
    media_source: String,
    version: String,
    credential: Vec<u8>,
    actor: String,
) -> anyhow::Result<bool> {
    db.write("managers.connection_settings.save",move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Only unowned connections may move to another container, and never onto one
        // that another service already uses.
        let ready:bool=tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1 AND (state!='complete' OR container_id IS NOT ?2 OR ?2 IS NOT ?3)) AND NOT EXISTS(SELECT 1 FROM service_updates WHERE service_id IN (SELECT id FROM stack_provisions WHERE kind=?1) AND state NOT IN ('committed','rolled-back','blocked')) AND NOT EXISTS(SELECT 1 FROM manager_services WHERE container_id=?3 AND id!=?4 UNION ALL SELECT 1 FROM support_services WHERE container_id=?3 AND id!=?4)",params![record.kind,record.container,container,key],|r|r.get(0))?;
        if !ready {return Ok(false);}
        let table=if matches!(record.kind.as_str(),"radarr"|"sonarr"|"lidarr") {"manager_services"} else {"support_services"};
        let updated=tx.execute(&format!("UPDATE {table} SET port=?1,url_base=?2,credential=?3,media_source=?4,version=?5,checked_at=?6,error=NULL,generation=?7,access_revision=?8,container_id=?11,container_name=?12 WHERE id=?9 AND container_id=?10"),params![port,url_base,credential,media_source,version,now(),id(),id(),key,record.container,container,container_name])?;
        if updated!=1 {return Ok(false);}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.connection.edit',?2,?3)",params![actor,key,now()])?;
        tx.commit()?;Ok(true)
    }).await
}
