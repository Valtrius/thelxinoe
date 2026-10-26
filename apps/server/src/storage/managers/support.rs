//! Database operations for managers.support.
use super::*;
use thelxinoe_database::Database;

pub(super) struct Row {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) container: String,
    pub(super) port: u16,
    pub(super) media_source: String,
    pub(super) credential: Vec<u8>,
    pub(super) url_base: String,
    pub(super) access_revision: String,
    pub(super) generation: String,
    pub(super) operational: bool,
}

pub(super) async fn load(key: String, db: &Database) -> anyhow::Result<Option<Row>> {
    db.read("managers.support.load", move|db|Ok(db.query_row("SELECT id,kind,container_id,port,media_source,credential,url_base,access_revision,generation,operational FROM support_services WHERE id=?1",[key],|r|Ok(Row{id:r.get(0)?,kind:r.get(1)?,container:r.get(2)?,port:r.get(3)?,media_source:r.get(4)?,credential:r.get(5)?,url_base:r.get(6)?,access_revision:r.get(7)?,generation:r.get(8)?,operational:r.get(9)?})).optional()?)).await
}

pub(super) async fn register_with_actor_read_support_services(
    kind: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read(
        "managers.support.register_with_actor_read_support_services",
        move |db| {
            Ok(db
                .query_row(
                    "SELECT id FROM support_services WHERE kind=?1",
                    [kind],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        },
    )
    .await
}

/// Save a Connected support service, or an installation's own container under its setup hold.
pub(super) async fn register_with_actor_write_stack_provisions(
    db: &Database,
    registration: Registration,
    input: Register,
    actor_id: String,
) -> anyhow::Result<bool> {
    db.write("managers.support.register_with_actor_write_stack_provisions", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let allowed:bool=match &registration.setup {
            Some(provision)=>tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND kind=?2 AND hold='setup' AND state='connecting' AND container_id=?3 AND (service_id IS NULL OR service_id=?4))",params![provision,input.kind,input.container_id,registration.key],|r|r.get(0))?,
            // A Connected service never coexists with an installation or unfinished transfer of its kind.
            None=>tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1) AND NOT EXISTS(SELECT 1 FROM service_transfers WHERE kind=?1 AND state NOT IN ('review','complete','cancelled','returned'))",[&input.kind],|r|r.get(0))?,
        };
        if !allowed {return Ok(false);}
        tx.execute("INSERT INTO support_services(id,name,kind,container_id,container_name,port,generation,credential,media_source,native_url,version,checked_at,url_base,access_revision,engine,evidence,verified,operational) VALUES (?1,?2,?3,?4,?14,?5,?6,?7,?8,?9,?10,?11,?12,?13,?15,?16,?17,0) ON CONFLICT(id) DO UPDATE SET name=excluded.name,container_id=excluded.container_id,container_name=excluded.container_name,port=excluded.port,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,native_url=excluded.native_url,url_base=excluded.url_base,access_revision=excluded.access_revision,version=excluded.version,checked_at=excluded.checked_at,engine=excluded.engine,evidence=excluded.evidence,verified=excluded.verified,operational=0,error=NULL",params![registration.key,input.name.trim(),input.kind,input.container_id,input.port,id(),registration.credential,registration.media_source,input.native_url,registration.version,now(),input.url_base,id(),input.container_name,registration.engine,registration.evidence.to_string(),registration.verified])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor_id,if registration.setup.is_some() {"support.setup.register"} else {"connection.connect"},registration.key,now()])?;
        tx.execute("DELETE FROM seerr_request_states WHERE service_id=?1", [&registration.key])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.support.list", |db| { let urls = access::launch_urls(db)?; let native = access::native_urls(db)?; Ok(db.prepare("SELECT id,name,kind,version,native_url,checked_at,error,container_id,port,url_base,operational,verified,engine,container_name FROM support_services ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"version":r.get::<_,String>(3)?,"native_url":native.get(&r.get::<_,String>(0)?).cloned().unwrap_or(r.get::<_,String>(4)?),"checked_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?,"container_id":r.get::<_,String>(7)?,"port":r.get::<_,u16>(8)?,"url_base":r.get::<_,String>(9)?,"operational":r.get::<_,bool>(10)?,"verified":r.get::<_,bool>(11)?,"engine":r.get::<_,String>(12)?,"container_name":r.get::<_,String>(13)?,"access_url":if r.get::<_,bool>(10)? {urls.get(&r.get::<_,String>(0)?)} else {None}})))?.collect::<rusqlite::Result<Vec<_>>>()?)}).await
}

/// Record a read-only test. Success counts only for the container that was tested.
pub(super) async fn test(
    db: &Database,
    key: String,
    container: String,
    version: Option<String>,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.support.test", move |db| {
        db.execute(
            "UPDATE support_services SET checked_at=?1,error=?2,verified=(?2 IS NULL AND container_id=?4),version=COALESCE(?5,version) WHERE id=?3",
            params![now(), error, key, container, version],
        )?;
        Ok(())
    })
    .await
}

/// Replace the credential of the exact connection `revision`; an edit in between wins.
pub(super) async fn replace_credential(
    db: &Database,
    key: String,
    container: String,
    revision: String,
    credential: Vec<u8>,
) -> anyhow::Result<bool> {
    db.write("managers.support.replace_credential", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE support_services SET credential=?1,access_revision=?2 WHERE id=?3 AND container_id=?4 AND access_revision=?5",
            params![credential, thelxinoe_core::id(), key, container, revision],
        )?;
        if updated != 1 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,'connection.credential_generated',?1,?2)",
            params![key, now()],
        )?;
        tx.commit()?;
        Ok(true)
    })
    .await
}

pub(super) async fn operational_health_read_support_services(
    db: &Database,
) -> anyhow::Result<Vec<(String, String)>> {
    db.read(
        "managers.support.operational_health_read_support_services",
        |db| {
            Ok(db
                .prepare(
                    "SELECT id,kind FROM support_services WHERE kind IN ('prowlarr','nzbget') AND operational=1",
                )?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        },
    )
    .await
}

pub(super) async fn operational_health_write_settings(
    items: Vec<Value>,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.support.operational_health_write_settings", move|db|{db.execute("INSERT INTO settings VALUES ('operations.support',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!({"items":items,"checked_at":now()}).to_string()])?;Ok(())}).await
}

pub(super) async fn inspect(
    db: &Database,
    key: String,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.support.inspect", move |db| {
        db.execute(
            "UPDATE support_services SET checked_at=?1,error=?2 WHERE id=?3",
            params![now(), error, key],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn action(
    db: &Database,
    action: String,
    p: thelxinoe_core::Principal,
    s: Support,
) -> anyhow::Result<()> {
    db.write("managers.support.action", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![p.user.id, format!("support.{}", action), s.id, now()],
        )?;
        Ok(())
    })
    .await
}
