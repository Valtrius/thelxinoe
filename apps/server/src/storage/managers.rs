//! Database operations for managers.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn service(id: String, db: &Database) -> anyhow::Result<Option<Service>> {
    db.read("managers.service", move|db|Ok(db.query_row("SELECT id,name,kind,container_id,port,generation,credential,media_source,defaults,url_base,access_revision,operational FROM manager_services WHERE id=?1 AND enabled=1",[id],|r|Ok(Service{id:r.get(0)?,name:r.get(1)?,kind:r.get(2)?,container:r.get(3)?,port:r.get(4)?,generation:r.get(5)?,credential:r.get(6)?,media_source:r.get(7)?,defaults:serde_json::from_str(&r.get::<_,String>(8)?).unwrap_or(Value::Null),url_base:r.get(9)?,access_revision:r.get(10)?,scope:if r.get::<_,bool>(11)? {ownership::Scope::Operational} else {ownership::Scope::Validation}})).optional()?)).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.list", |db| { let urls = access::launch_urls(db)?; let native = access::native_urls(db)?; Ok(db.prepare("SELECT id,name,kind,container_id,port,version,defaults,checked_at,error,url_base,COALESCE((SELECT value='true' FROM settings WHERE key='manager.retention.'||manager_services.id),EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=manager_services.id AND origin='installed')),operational,verified,engine,container_name FROM manager_services WHERE enabled=1 ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"container_id":r.get::<_,String>(3)?,"port":r.get::<_,u16>(4)?,"version":r.get::<_,String>(5)?,"defaults":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or(Value::Null),"checked_at":r.get::<_,i64>(7)?,"error":r.get::<_,Option<String>>(8)?,"url_base":r.get::<_,String>(9)?,"retention_enabled":r.get::<_,bool>(10)?,"operational":r.get::<_,bool>(11)?,"verified":r.get::<_,bool>(12)?,"engine":r.get::<_,String>(13)?,"container_name":r.get::<_,String>(14)?,"native_url":native.get(&r.get::<_,String>(0)?),"access_url":if r.get::<_,bool>(11)? {urls.get(&r.get::<_,String>(0)?)} else {None}})))?.collect::<rusqlite::Result<Vec<_>>>()?)}).await
}

pub(super) async fn service_for_kind(
    kind: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("managers.service_for_kind", move |db| {
        Ok(db
            .query_row(
                "SELECT id FROM manager_services WHERE operational=1 AND kind=?1",
                [kind],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn register_with_actor_read_manager_services(
    kind: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read(
        "managers.register_with_actor_read_manager_services",
        move |db| {
            Ok(db
                .query_row(
                    "SELECT id FROM manager_services WHERE kind=?1",
                    [kind],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        },
    )
    .await
}

/// Save a Connected manager, or an installation's own container under its setup hold.
pub(super) async fn register_with_actor_write_stack_provisions(
    db: &Database,
    registration: Registration,
    input: Register,
    actor_id: String,
) -> anyhow::Result<bool> {
    db.write("managers.register_with_actor_write_stack_provisions", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let allowed:bool=match &registration.setup {
            Some(provision)=>tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE id=?1 AND kind=?2 AND hold='setup' AND state='connecting' AND container_id=?3 AND (service_id IS NULL OR service_id=?4))",params![provision,input.kind,input.container_id,registration.key],|r|r.get(0))?,
            // A Connected service never coexists with an installation or unfinished transfer of its kind.
            None=>tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1) AND NOT EXISTS(SELECT 1 FROM service_transfers WHERE kind=?1 AND state NOT IN ('review','complete','cancelled','returned'))",[&input.kind],|r|r.get(0))?,
        };
        if !allowed {return Ok(false);}
        // A reused, disabled row is a different service: its auto-delete consent doesn't carry over.
        tx.execute("DELETE FROM settings WHERE key='manager.retention.'||?1 AND EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND enabled=0)",[&registration.key])?;
        tx.execute("INSERT INTO manager_services(id,name,kind,container_id,container_name,port,generation,credential,media_source,version,checked_at,url_base,access_revision,engine,evidence,verified,operational) VALUES (?1,?2,?3,?4,?13,?5,?6,?7,?8,?9,?10,?11,?12,?14,?15,?16,0) ON CONFLICT(id) DO UPDATE SET enabled=1,operational=0,defaults=CASE WHEN manager_services.enabled=0 THEN '{}' ELSE manager_services.defaults END,name=excluded.name,container_id=excluded.container_id,container_name=excluded.container_name,port=excluded.port,url_base=excluded.url_base,access_revision=excluded.access_revision,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,version=excluded.version,checked_at=excluded.checked_at,engine=excluded.engine,evidence=excluded.evidence,verified=excluded.verified,error=NULL",params![registration.key,input.name.trim(),input.kind,input.container_id,input.port,id(),registration.credential,registration.media_source,registration.version,now(),input.url_base,id(),input.container_name,registration.engine,registration.evidence.to_string(),registration.verified])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor_id,if registration.setup.is_some() {"manager.setup.register"} else {"connection.connect"},registration.key,now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn defaults(
    db: &Database,
    id: String,
    input: Defaults,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.defaults", move|db|{let tx=db.transaction()?;tx.execute("UPDATE manager_services SET defaults=?1,generation=?3 WHERE id=?2",params![serde_json::to_string(&input)?,id,thelxinoe_core::id()])?;connections::retry_requests(&tx,&id)?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'manager.defaults',?2,?3)",params![p.user.id,id,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn initialize_defaults(
    db: &Database,
    key: String,
    input: Defaults,
) -> anyhow::Result<()> {
    db.write("managers.initialize_defaults", move |db| {
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE manager_services SET defaults=?1,generation=?2 WHERE id=?3",
            params![serde_json::to_string(&input)?, id(), key],
        )?;
        connections::retry_requests(&tx, &key)?;
        tx.commit()?;
        Ok(())
    })
    .await
}

/// Record a read-only test. Success counts only for the container that was tested.
pub(super) async fn test(
    db: &Database,
    id: String,
    container: String,
    version: Option<String>,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.test", move |db| {
        db.execute(
            "UPDATE manager_services SET checked_at=?1,error=?2,verified=(?2 IS NULL AND container_id=?4),version=COALESCE(?5,version) WHERE id=?3",
            params![now(), error, id, container, version],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn attached(
    key: String,
    db: &Database,
) -> anyhow::Result<Option<recreation::Attached>> {
    db.read("managers.attached", move |db| {
        Ok(db.query_row("SELECT s.kind,s.container_id,s.container_name,s.port,s.url_base,s.media_source,s.credential,EXISTS(SELECT 1 FROM stack_provisions p WHERE p.service_id=s.id OR p.kind=s.kind) FROM (SELECT id,kind,container_id,container_name,port,url_base,media_source,credential FROM manager_services WHERE enabled=1 UNION ALL SELECT id,kind,container_id,container_name,port,url_base,media_source,credential FROM support_services) s WHERE s.id=?1",[key],|r|Ok(recreation::Attached{kind:r.get(0)?,container:r.get(1)?,container_name:r.get(2)?,port:r.get(3)?,url_base:r.get(4)?,media_source:r.get(5)?,credential:r.get(6)?,owned:r.get(7)?})).optional()?)
    })
    .await
}

/// Point an unowned attached service at its verified replacement container.
pub(super) async fn follow(
    db: &Database,
    key: String,
    previous: String,
    container: String,
) -> anyhow::Result<bool> {
    db.write("managers.follow", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = 0;
        for table in ["manager_services", "support_services"] {
            // Generations stay unchanged: the replacement keeps the same configuration
            // and storage, so in-flight work against this service remains valid.
            // The caller revalidated the replacement's API identity with the saved credential.
            changed += tx.execute(&format!("UPDATE {table} SET container_id=?1,verified=1,checked_at=?4,error=NULL,evidence=json_set(evidence,'$.container.id',?1) WHERE id=?2 AND container_id=?3 AND NOT EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=?2 OR kind={table}.kind) AND NOT EXISTS(SELECT 1 FROM service_transfers WHERE service_id=?2 AND state NOT IN ('review','complete','cancelled','returned')) AND NOT EXISTS(SELECT 1 FROM manager_services WHERE container_id=?1 UNION ALL SELECT 1 FROM support_services WHERE container_id=?1)"), params![container, key, previous, now()])?;
        }
        if changed == 1 {
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,'service.container.follow',?1,?2)", params![key, now()])?;
        }
        tx.commit()?;
        Ok(changed == 1)
    })
    .await
}

pub(super) async fn owned_here(key: String, db: &Database) -> anyhow::Result<bool> {
    db.read("managers.owned_here", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=?1)",
            [key],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn installed_here(key: String, db: &Database) -> anyhow::Result<bool> {
    db.read("managers.installed_here", move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=?1 AND origin='installed')",[key],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn support_native_host(
    db: &Database,
    key: String,
) -> anyhow::Result<Option<String>> {
    db.read("managers.support_native_host", move |db| {
        Ok(db
            .query_row(
                "SELECT native_url FROM support_services WHERE id=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}
