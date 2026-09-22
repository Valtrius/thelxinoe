//! Database operations for online.tools.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn status(db: &Database) -> anyhow::Result<Option<Value>> {
    db.read("online.tools.status", |db|Ok(db.query_row("SELECT id,state,error FROM jobs WHERE kind='online.tools.install' ORDER BY created_at DESC,rowid DESC LIMIT 1",[],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"error":r.get::<_,Option<String>>(2)?}))).optional()?)).await
}

pub(super) async fn enqueue_install(
    db: &Database,
    actor: Option<String>,
) -> anyhow::Result<String> {
    db.write("online.tools.enqueue_install", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id)=tx.query_row("SELECT id FROM jobs WHERE kind='online.tools.install' AND state IN ('queued','running') LIMIT 1",[],|r|r.get::<_,String>(0)).optional()?{return Ok(id);}
        let id=thelxinoe_core::id();
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'online.tools.install','{}',?1,'queued',?2,?2)",params![id,now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.tools.install',?2,?3)",params![actor,id,now()])?;
        tx.commit()?;Ok(id)
    }).await
}

pub(super) async fn selection(db: &Database) -> anyhow::Result<Option<String>> {
    db.read("online.tools.selection", |db| {
        Ok(db
            .query_row(
                "SELECT value FROM settings WHERE key='online.tools'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn install_write_jobs(
    payload: String,
    id: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.tools.install_write_jobs", move |db| {
        db.execute(
            "UPDATE jobs SET payload=?1 WHERE id=?2",
            params![payload, id],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn install_write_settings(value: String, db: &Database) -> anyhow::Result<()> {
    db.write("online.tools.install_write_settings", move|db|{db.execute("INSERT INTO settings(key,value) VALUES ('online.tools',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;Ok(())}).await
}
