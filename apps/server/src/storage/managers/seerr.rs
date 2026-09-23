use rusqlite::{OptionalExtension, params};
use thelxinoe_database::Database;

pub(crate) async fn service(db: &Database) -> anyhow::Result<Option<String>> {
    db.read("seerr.service", |db| {
        Ok(db
            .query_row(
                "SELECT id FROM support_services WHERE kind='seerr'",
                [],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}
pub(super) async fn begin_bootstrap(
    db: &Database,
    service: String,
    hash: String,
) -> anyhow::Result<()> {
    db.write("seerr.begin_bootstrap", move |db| { db.execute("INSERT INTO seerr_bootstrap_tokens VALUES (?1,?2,?3) ON CONFLICT(service_id) DO UPDATE SET token_hash=excluded.token_hash,expires_at=excluded.expires_at", params![service,hash,thelxinoe_core::now()+120])?; Ok(()) }).await
}
pub(super) async fn end_bootstrap(db: &Database, service: String) -> anyhow::Result<()> {
    db.write("seerr.end_bootstrap", move |db| {
        db.execute(
            "DELETE FROM seerr_bootstrap_tokens WHERE service_id=?1",
            [service],
        )?;
        Ok(())
    })
    .await
}
pub(super) async fn resolve_bootstrap(
    db: &Database,
    hash: String,
) -> anyhow::Result<Option<String>> {
    db.read("seerr.resolve_bootstrap", move |db| Ok(db.query_row("SELECT service_id FROM seerr_bootstrap_tokens WHERE token_hash=?1 AND expires_at>?2", params![hash,thelxinoe_core::now()], |r| r.get(0)).optional()?)).await
}
pub(super) async fn mapped_user(
    db: &Database,
    service: String,
    user: String,
) -> anyhow::Result<Option<i64>> {
    db.read("seerr.mapped_user", move |db| {
        Ok(db
            .query_row(
                "SELECT remote_id FROM seerr_users WHERE service_id=?1 AND user_id=?2",
                params![service, user],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}
pub(super) async fn save_user(
    db: &Database,
    service: String,
    user: String,
    remote: i64,
) -> anyhow::Result<()> {
    db.write("seerr.save_user", move |db| { db.execute("INSERT INTO seerr_users VALUES (?1,?2,?3) ON CONFLICT(service_id,user_id) DO UPDATE SET remote_id=excluded.remote_id", params![service,user,remote])?; Ok(()) }).await
}
pub(super) async fn auto_approve(db: &Database, user: String) -> anyhow::Result<bool> {
    db.read("seerr.auto_approve", move |db| {
        Ok(db
            .query_row(
                "SELECT auto_approve FROM acquisition_users WHERE user_id=?1",
                [user],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    })
    .await
}
pub(super) async fn managers(db: &Database) -> anyhow::Result<Vec<String>> {
    db.read("seerr.managers", |db| Ok(db.prepare("SELECT id FROM manager_services WHERE enabled=1 AND kind IN ('radarr','sonarr') ORDER BY kind")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
