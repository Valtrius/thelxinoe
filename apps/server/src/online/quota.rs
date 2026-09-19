use crate::{
    AppState,
    error::{ApiError, Result},
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
fn day() -> String {
    Utc::now()
        .with_timezone(&chrono_tz::America::Los_Angeles)
        .date_naive()
        .to_string()
}
pub async fn block(state: &AppState) -> Result<()> {
    let today = day();
    state.db.call(move|db|{db.execute("INSERT INTO youtube_quota(day,blocked) VALUES (?1,1) ON CONFLICT(day) DO UPDATE SET blocked=1",[today])?;Ok(())}).await?;
    Ok(())
}
pub async fn status(state: &AppState) -> Result<Value> {
    let today = day();
    Ok(state.db.call(move|db|{
        let (used,blocked)=db.query_row("SELECT used,blocked FROM youtube_quota WHERE day=?1",[&today],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional()?.unwrap_or((0,false));
        Ok(json!({"day":today,"used":used,"blocked":blocked,"reset_timezone":"America/Los_Angeles"}))
    }).await?)
}
// Reserve before sending. Failed and retried requests also consume provider
// quota. The scheduler will take one bounded page per user on each turn.
pub async fn reserve(state: &AppState, user: &str) -> Result<()> {
    let today = day();
    let user = user.to_owned();
    let allowed=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let budget=tx.query_row("SELECT value FROM settings WHERE key='youtube_daily_quota'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|v|v.parse::<i64>().ok()).unwrap_or(10000);
        tx.execute("INSERT INTO youtube_quota(day) VALUES (?1) ON CONFLICT DO NOTHING",[&today])?;
        let changed=tx.execute("UPDATE youtube_quota SET used=used+1 WHERE day=?1 AND used<?2 AND blocked=0",params![today,budget])?;
        if changed==1 {tx.execute("INSERT INTO youtube_quota_users VALUES (?1,?2,1) ON CONFLICT(day,user_id) DO UPDATE SET used=used+1",params![today,user])?;}
        tx.commit()?;Ok(changed==1)
    }).await?;
    if !allowed {
        return Err(ApiError::conflict(
            "YouTube's shared daily API budget is exhausted; synchronization resumes after midnight Pacific time",
        ));
    }
    Ok(())
}
