//! Database operations for backups.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn policy(db: &Database) -> anyhow::Result<Option<Policy>> {
    db.read("backups.policy", |db| {
        Ok(db
            .query_row(
                "SELECT value FROM settings WHERE key='backups.policy'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|v| serde_json::from_str(&v))
            .transpose()?)
    })
    .await
}

pub(super) async fn save_policy(
    db: &Database,
    policy: Policy,
    actor: String,
) -> anyhow::Result<()> {
    db.write("backups.save_policy", move |db| {
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO settings VALUES ('backups.policy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&policy)?],
        )?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'backup.policy','server',?2)",
            params![actor, now()],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn run(db: &Database) -> anyhow::Result<Option<Run>> {
    db.read("backups.run", |db| {
        Ok(db
            .query_row(
                "SELECT value FROM settings WHERE key='backups.automatic'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|v| serde_json::from_str(&v))
            .transpose()?)
    })
    .await
}

pub(super) async fn save_run(db: &Database, run: Run) -> anyhow::Result<()> {
    db.write("backups.save_run", move |db| {
        db.execute(
            "INSERT INTO settings VALUES ('backups.automatic',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&run)?],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn passphrase_saved(db: &Database) -> anyhow::Result<bool> {
    db.read("backups.passphrase_saved", |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM secrets WHERE scope=?1)",
            [PASSPHRASE],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn busy(db: &Database) -> anyhow::Result<bool> {
    db.read("backups.busy", |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running') OR EXISTS(SELECT 1 FROM media_operations WHERE state='executing')",[now()],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn backup_command_read_playback_sessions(db: &Database) -> anyhow::Result<bool> {
    db.read("backups.backup_command_read_playback_sessions", |db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running')",[now()],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn audit(
    db: &Database,
    actor: Option<String>,
    action: &'static str,
) -> anyhow::Result<()> {
    db.write("backups.audit", move |db| {
        db.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,'server',?3)",
            params![actor, action, now()],
        )?;
        Ok(())
    })
    .await
}
