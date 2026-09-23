//! Database operations for managers.support.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn load(
    key: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String, String, u16, String, Vec<u8>)>> {
    db.read("managers.support.load", move|db|Ok(db.query_row("SELECT id,kind,container_id,port,media_source,credential FROM support_services WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u16>(3)?,r.get::<_,String>(4)?,r.get::<_,Vec<u8>>(5)?))).optional()?)).await
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

pub(super) async fn register_with_actor_write_stack_provisions(
    media_source: String,
    version: String,
    key: String,
    secret: Vec<u8>,
    db: &Database,
    input: Register,
    actor_id: String,
) -> anyhow::Result<bool> {
    db.write("managers.support.register_with_actor_write_stack_provisions", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let provision:Option<(String,Option<String>)>=tx.query_row("SELECT state,container_id FROM stack_provisions WHERE kind=?1",[&input.kind],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if provision.is_some_and(|(state,container)| state!="connecting" || container.as_deref()!=Some(input.container_id.as_str())) {return Ok(false);}
        tx.execute("INSERT INTO support_services(id,name,kind,container_id,port,generation,credential,media_source,native_url,version,checked_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET name=excluded.name,container_id=excluded.container_id,port=excluded.port,generation=excluded.generation,credential=excluded.credential,media_source=excluded.media_source,native_url=excluded.native_url,version=excluded.version,checked_at=excluded.checked_at,error=NULL",params![key,input.name.trim(),input.kind,input.container_id,input.port,id(),secret,media_source,input.native_url,version,now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'support.register',?2,?3)",params![actor_id,key,now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.support.list", |db|Ok(db.prepare("SELECT id,name,kind,version,native_url,checked_at,error FROM support_services ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"version":r.get::<_,String>(3)?,"native_url":r.get::<_,String>(4)?,"checked_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn operational_health_read_support_services(
    db: &Database,
) -> anyhow::Result<Vec<(String, String)>> {
    db.read(
        "managers.support.operational_health_read_support_services",
        |db| {
            Ok(db
                .prepare(
                    "SELECT id,kind FROM support_services WHERE kind IN ('prowlarr','nzbget')",
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
