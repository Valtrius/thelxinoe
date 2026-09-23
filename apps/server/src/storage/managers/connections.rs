//! Small, typed connection configuration records share the existing settings store.
//! Credentials remain in the encrypted integrations; ownership evidence is hashed.
use super::*;
use thelxinoe_database::Database;
const PREFIX: &str = "services.connection.";

pub(super) async fn endpoints(db: &Database) -> anyhow::Result<Vec<Endpoint>> {
    db.read("managers.connections.endpoints",|db| {
        Ok(db.prepare("SELECT id,name,kind,container_id,port FROM manager_services WHERE enabled=1 UNION ALL SELECT id,name,kind,container_id,port FROM support_services ORDER BY kind")?
            .query_map([],|r|Ok(Endpoint{id:r.get(0)?,name:r.get(1)?,kind:r.get(2)?,container:r.get(3)?,port:r.get(4)?}))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}
pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Link>> {
    db.read("managers.connections.list", |db| {
        db.prepare(
            "SELECT value FROM settings WHERE key LIKE 'services.connection.%' ORDER BY key",
        )?
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect()
    })
    .await
}
pub(super) async fn load(db: &Database, key: &str) -> anyhow::Result<Option<Link>> {
    let key = format!("{PREFIX}{key}");
    db.read("managers.connections.load", move |db| {
        db.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .map(|text| Ok(serde_json::from_str(&text)?))
        .transpose()
    })
    .await
}
pub(super) async fn save(db: &Database, link: Link) -> anyhow::Result<()> {
    db.write("managers.connections.save",move|db| {
        db.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("{PREFIX}{}",link.id),serde_json::to_string(&link)?])?;
        Ok(())
    }).await
}
pub(super) async fn configure(
    db: &Database,
    link: Link,
    actor: String,
    action: String,
) -> anyhow::Result<()> {
    db.write("managers.connections.configure",move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Retry and worker state use the same per-link lock in the server.
        tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("{PREFIX}{}",link.id),serde_json::to_string(&link)?])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor,format!("service.connection.{action}"),link.id,now()])?;
        tx.commit()?;Ok(())
    }).await
}
