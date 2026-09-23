//! Database operations for managers.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn service(id: String, db: &Database) -> anyhow::Result<Option<Service>> {
    db.read("managers.service", move|db|Ok(db.query_row("SELECT id,name,kind,container_id,port,generation,credential,media_source,defaults FROM manager_services WHERE id=?1 AND enabled=1",[id],|r|Ok(Service{id:r.get(0)?,name:r.get(1)?,kind:r.get(2)?,container:r.get(3)?,port:r.get(4)?,generation:r.get(5)?,credential:r.get(6)?,media_source:r.get(7)?,defaults:serde_json::from_str(&r.get::<_,String>(8)?).unwrap_or(Value::Null)})).optional()?)).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.list", |db|Ok(db.prepare("SELECT id,name,kind,container_id,port,version,defaults,checked_at,error FROM manager_services WHERE enabled=1 ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"container_id":r.get::<_,String>(3)?,"port":r.get::<_,u16>(4)?,"version":r.get::<_,String>(5)?,"defaults":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or(Value::Null),"checked_at":r.get::<_,i64>(7)?,"error":r.get::<_,Option<String>>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
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
        let provision:Option<(String,Option<String>)>=tx.query_row("SELECT state,container_id FROM stack_provisions WHERE kind=?1",[&input.kind],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if provision.is_some_and(|(state,container)| state!="connecting" || container.as_deref()!=Some(input.container_id.as_str())) {return Ok(false);}
        tx.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET enabled=1,defaults=CASE WHEN manager_services.enabled=0 THEN '{}' ELSE manager_services.defaults END,name=excluded.name,container_id=excluded.container_id,port=excluded.port,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,version=excluded.version,checked_at=excluded.checked_at,error=NULL",params![key,input.name.trim(),input.kind,input.container_id,input.port,id(),credential,media_source,version,now()])?;
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
