//! Database operations for realtime.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn events(
    db: &Database,
    since: i64,
    user: String,
    session: String,
) -> anyhow::Result<Option<Vec<Value>>> {
    db.read("realtime.events", move |db|{
        let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND expires_at>?2)",rusqlite::params![session,now()],|r|r.get(0))?;
        if !active{return Ok(None);}
        let mut query=db.prepare("SELECT id,kind,payload FROM events WHERE id>?1 AND (user_id IS NULL OR user_id=?2) ORDER BY id LIMIT 100")?;
        Ok(Some(query.query_map(rusqlite::params![since,user],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"payload":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(Value::Null)})))?.collect::<std::result::Result<Vec<_>,_>>()?))
    }).await
}

pub(super) async fn jobs(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("realtime.jobs", |db|Ok(db.prepare("SELECT id,kind,state,attempts,error,created_at FROM jobs ORDER BY created_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"attempts":r.get::<_,i64>(3)?,"error":r.get::<_,Option<String>>(4)?,"created_at":r.get::<_,i64>(5)?})))?.collect::<std::result::Result<Vec<_>,_>>()?)).await
}
