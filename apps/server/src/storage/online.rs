//! Database operations for online.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn configuration(db: &Database) -> anyhow::Result<(bool, u32)> {
    db.read("online.configuration", |db| {
        let downloads = db
            .query_row(
                "SELECT value FROM settings WHERE key='youtube_downloads'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .is_some_and(|v| v == "true");
        let budget = db
            .query_row(
                "SELECT value FROM settings WHERE key='youtube_daily_quota'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(10000);
        Ok((downloads, budget))
    })
    .await
}

pub(super) async fn configure(
    db: &Database,
    input: Settings,
    p: thelxinoe_core::Principal,
    changed: bool,
    encrypted: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    db.write("online.configure", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(encrypted)=encrypted{tx.execute("INSERT INTO secrets VALUES ('provider.google',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;}
        if changed {
            tx.execute("UPDATE online_accounts SET status=CASE WHEN status='disconnected' THEN status ELSE 'reconnect_required' END,credential=NULL,expires_at=0,generation=?1 WHERE provider='youtube'",[thelxinoe_core::id()])?;
            tx.execute("DELETE FROM oauth_attempts WHERE provider='youtube'",[])?;
            tx.execute("DELETE FROM youtube_sync",[])?;
        }
        for (key,value) in [("youtube_downloads",input.youtube_downloads.to_string()),("youtube_daily_quota",input.youtube_daily_quota.to_string())]{tx.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','youtube',?2)",params![p.user.id,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn account_read_youtube_sync(
    db: &Database,
    user: String,
) -> anyhow::Result<Option<Value>> {
    db.read("online.account_read_youtube_sync", move|db|Ok(db.query_row("SELECT next_run,last_complete,error,cursor,failures FROM youtube_sync WHERE user_id=?1",[user],|r|Ok(json!({"next_run":r.get::<_,i64>(0)?,"last_complete":r.get::<_,Option<i64>>(1)?,"error":r.get::<_,Option<String>>(2)?,"in_progress":r.get::<_,String>(3)?!="{}" && r.get::<_,u32>(4)?==0,"phase":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).ok().and_then(|v|v["phase"].as_str().map(str::to_owned))}))).optional()?)).await
}

pub(super) async fn account_read_online_accounts(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Option<Value>> {
    db.read("online.account_read_online_accounts", move|db|Ok(db.query_row("SELECT status,display_name,external_id,updated_at,avatar_url FROM online_accounts WHERE user_id=?1 AND provider='youtube'",[p.user.id],|r|Ok(json!({"status":r.get::<_,String>(0)?,"display_name":r.get::<_,String>(1)?,"external_id":r.get::<_,String>(2)?,"updated_at":r.get::<_,i64>(3)?,"avatar_url":r.get::<_,Option<String>>(4)?}))).optional()?)).await
}

pub(super) async fn disconnect(db: &Database, user: String) -> anyhow::Result<()> {
    db.write("online.disconnect", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE online_accounts SET status='disconnected',credential=NULL,expires_at=0,generation=?1,updated_at=?2,avatar_url=NULL,profile_checked_at=0 WHERE user_id=?3 AND provider='youtube'",params![thelxinoe_core::id(),now(),user])?;
        tx.execute("DELETE FROM oauth_attempts WHERE user_id=?1 AND provider='youtube'",[&user])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.disconnect','youtube',?2)",params![user,now()])?;
        tx.commit()?;Ok(())
    }).await
}
