use super::*;
use thelxinoe_database::Database;

pub(super) async fn load(db: &Database, key: String) -> anyhow::Result<Option<Record>> {
    db.read("managers.connection_settings.load",move |db| {
        Ok(db.query_row("SELECT kind,container_id,credential FROM manager_services WHERE id=?1 AND enabled=1 UNION ALL SELECT kind,container_id,credential FROM support_services WHERE id=?1",[key],|r|Ok(Record{kind:r.get(0)?,container:r.get(1)?,credential:r.get(2)?})).optional()?)
    }).await
}
pub(super) struct Saved {
    pub(super) container: String,
    pub(super) container_name: String,
    pub(super) port: u16,
    pub(super) url_base: String,
    pub(super) media_source: String,
    pub(super) version: String,
    pub(super) verified: bool,
    pub(super) engine: String,
    pub(super) evidence: Value,
    pub(super) credential: Vec<u8>,
}
pub(super) async fn save(
    db: &Database,
    key: String,
    record: Record,
    saved: Saved,
    actor: String,
) -> anyhow::Result<bool> {
    db.write("managers.connection_settings.save",move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Connected services may move to another container. Owned services may only correct
        // credentials of their own released container. Never onto a container another service uses.
        let ready:bool=tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1 AND (state!='complete' OR hold IS NOT NULL OR container_id IS NOT ?2 OR ?2 IS NOT ?3)) AND NOT EXISTS(SELECT 1 FROM service_transfers WHERE kind=?1 AND state NOT IN ('review','complete','cancelled','returned')) AND NOT EXISTS(SELECT 1 FROM service_updates WHERE service_id IN (SELECT id FROM stack_provisions WHERE kind=?1) AND state NOT IN ('committed','rolled-back','blocked')) AND NOT EXISTS(SELECT 1 FROM manager_services WHERE container_id=?3 AND id!=?4 UNION ALL SELECT 1 FROM support_services WHERE container_id=?3 AND id!=?4)",params![record.kind,record.container,saved.container,key],|r|r.get(0))?;
        if !ready {return Ok(false);}
        let table=if matches!(record.kind.as_str(),"radarr"|"sonarr"|"lidarr") {"manager_services"} else {"support_services"};
        let updated=tx.execute(&format!("UPDATE {table} SET port=?1,url_base=?2,credential=?3,media_source=?4,version=?5,checked_at=?6,error=NULL,generation=?7,access_revision=?8,container_id=?11,container_name=?12,verified=?13,engine=?14,evidence=?15 WHERE id=?9 AND container_id=?10"),params![saved.port,saved.url_base,saved.credential,saved.media_source,saved.version,now(),id(),id(),key,record.container,saved.container,saved.container_name,saved.verified,saved.engine,saved.evidence.to_string()])?;
        if updated!=1 {return Ok(false);}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'connection.edit',?2,?3)",params![actor,key,now()])?;
        tx.commit()?;Ok(true)
    }).await
}
