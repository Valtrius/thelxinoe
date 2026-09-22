//! Database operations for managers.controls.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn target(
    db: &Database,
    key: String,
) -> anyhow::Result<Option<(String, Option<i64>, String, String)>> {
    db.read("managers.controls.target", move|db|Ok(db.query_row("SELECT service_id,manager_id,external_id,user_id FROM acquisition_requests WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?)).await
}

pub(super) async fn status(
    db: &Database,
    check: String,
    uid: String,
    admin: bool,
) -> anyhow::Result<bool> {
    db.read("managers.controls.status", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM acquisition_requests WHERE id=?1 AND (?2 OR user_id=?3))",
            params![check, admin, uid],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn grab(
    db: &Database,
    key: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.controls.grab", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.grab',?2,?3)",
            params![p.user.id, key, now()],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn monitor(
    db: &Database,
    key: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.controls.monitor", move|db|{db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.monitor',?2,?3)",params![p.user.id,key,now()])?;Ok(())}).await
}
