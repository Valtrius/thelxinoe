//! Database operations for online.quota.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn block(today: String, db: &Database) -> anyhow::Result<()> {
    db.write("online.quota.block", move|db|{db.execute("INSERT INTO youtube_quota(day,blocked) VALUES (?1,1) ON CONFLICT(day) DO UPDATE SET blocked=1",[today])?;Ok(())}).await
}

pub(super) async fn status(today: String, db: &Database) -> anyhow::Result<Value> {
    db.read("online.quota.status", move|db|{
        let (used,blocked)=db.query_row("SELECT used,blocked FROM youtube_quota WHERE day=?1",[&today],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional()?.unwrap_or((0,false));
        Ok(json!({"day":today,"used":used,"blocked":blocked,"reset_timezone":"America/Los_Angeles"}))
    }).await
}

pub(super) async fn reserve(today: String, user: String, db: &Database) -> anyhow::Result<bool> {
    db.write("online.quota.reserve", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let budget=tx.query_row("SELECT value FROM settings WHERE key='youtube_daily_quota'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|v|v.parse::<i64>().ok()).unwrap_or(10000);
        tx.execute("INSERT INTO youtube_quota(day) VALUES (?1) ON CONFLICT DO NOTHING",[&today])?;
        let changed=tx.execute("UPDATE youtube_quota SET used=used+1 WHERE day=?1 AND used<?2 AND blocked=0",params![today,budget])?;
        if changed==1 {tx.execute("INSERT INTO youtube_quota_users VALUES (?1,?2,1) ON CONFLICT(day,user_id) DO UPDATE SET used=used+1",params![today,user])?;}
        tx.commit()?;Ok(changed==1)
    }).await
}
