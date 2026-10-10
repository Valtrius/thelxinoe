//! Database operations for managers.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn service(id: String, db: &Database) -> anyhow::Result<Option<Service>> {
    db.read("managers.service", move|db|Ok(db.query_row("SELECT id,name,kind,container_id,port,generation,credential,media_source,defaults,url_base,access_revision FROM manager_services WHERE id=?1 AND enabled=1",[id],|r|Ok(Service{id:r.get(0)?,name:r.get(1)?,kind:r.get(2)?,container:r.get(3)?,port:r.get(4)?,generation:r.get(5)?,credential:r.get(6)?,media_source:r.get(7)?,defaults:serde_json::from_str(&r.get::<_,String>(8)?).unwrap_or(Value::Null),url_base:r.get(9)?,access_revision:r.get(10)?})).optional()?)).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.list", |db| { let urls = access::launch_urls(db)?; let native = access::native_urls(db)?; Ok(db.prepare("SELECT id,name,kind,container_id,port,version,defaults,checked_at,error,url_base,COALESCE((SELECT value='true' FROM settings WHERE key='manager.retention.'||manager_services.id),EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=manager_services.id AND origin='installed')) FROM manager_services WHERE enabled=1 ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"container_id":r.get::<_,String>(3)?,"port":r.get::<_,u16>(4)?,"version":r.get::<_,String>(5)?,"defaults":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or(Value::Null),"checked_at":r.get::<_,i64>(7)?,"error":r.get::<_,Option<String>>(8)?,"url_base":r.get::<_,String>(9)?,"retention_enabled":r.get::<_,bool>(10)?,"native_url":native.get(&r.get::<_,String>(0)?),"access_url":urls.get(&r.get::<_,String>(0)?)})))?.collect::<rusqlite::Result<Vec<_>>>()?)}).await
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

pub(super) async fn register_with_actor_write_stack_provisions(
    media_source: String,
    version: String,
    key: String,
    credential: Vec<u8>,
    db: &Database,
    input: Register,
    actor_id: String,
) -> anyhow::Result<bool> {
    db.write("managers.register_with_actor_write_stack_provisions", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let provision:Option<(String,Option<String>,String)>=tx.query_row("SELECT state,container_id,origin FROM stack_provisions WHERE kind=?1",[&input.kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if provision.is_some_and(|(state,container,origin)| container.as_deref()!=Some(input.container_id.as_str()) || if origin=="adopted" {state!="complete"} else {state!="connecting" || input.url_base != thelxinoe_core::service_url_base(&input.kind)}) {return Ok(false);}
        tx.execute("INSERT INTO manager_services(id,name,kind,container_id,container_name,port,generation,credential,media_source,version,checked_at,url_base,access_revision) VALUES (?1,?2,?3,?4,?13,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO UPDATE SET enabled=1,defaults=CASE WHEN manager_services.enabled=0 THEN '{}' ELSE manager_services.defaults END,name=excluded.name,container_id=excluded.container_id,container_name=excluded.container_name,port=excluded.port,url_base=excluded.url_base,access_revision=excluded.access_revision,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,version=excluded.version,checked_at=excluded.checked_at,error=NULL",params![key,input.name.trim(),input.kind,input.container_id,input.port,id(),credential,media_source,version,now(),input.url_base,id(),input.container_name])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'manager.register',?2,?3)",params![actor_id,key,now()])?;
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
    db.write("managers.defaults", move|db|{let tx=db.transaction()?;tx.execute("UPDATE manager_services SET defaults=?1,generation=?3 WHERE id=?2",params![serde_json::to_string(&input)?,id,thelxinoe_core::id()])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'manager.defaults',?2,?3)",params![p.user.id,id,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn initialize_defaults(
    db: &Database,
    key: String,
    input: Defaults,
) -> anyhow::Result<()> {
    db.write("managers.initialize_defaults", move |db| {
        db.execute(
            "UPDATE manager_services SET defaults=?1,generation=?2 WHERE id=?3",
            params![serde_json::to_string(&input)?, id(), key],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn test(db: &Database, id: String, error: Option<String>) -> anyhow::Result<()> {
    db.write("managers.test", move |db| {
        db.execute(
            "UPDATE manager_services SET checked_at=?1,error=?2 WHERE id=?3",
            params![now(), error, id],
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
            changed += tx.execute(&format!("UPDATE {table} SET container_id=?1 WHERE id=?2 AND container_id=?3 AND NOT EXISTS(SELECT 1 FROM stack_provisions WHERE service_id=?2 OR kind={table}.kind) AND NOT EXISTS(SELECT 1 FROM manager_services WHERE container_id=?1 UNION ALL SELECT 1 FROM support_services WHERE container_id=?1)"), params![container, key, previous])?;
        }
        if changed == 1 {
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,'service.container.follow',?1,?2)", params![key, now()])?;
        }
        tx.commit()?;
        Ok(changed == 1)
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
