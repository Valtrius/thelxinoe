//! Database operations for jellyfin.quick_connect.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn initiate(
    scope: String,
    device2: Device,
    db: &Database,
) -> anyhow::Result<Option<(String, String, String)>> {
    db.write("jellyfin.quick_connect.initiate", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM quick_connect WHERE expires_at<=?1",[now()])?;
        tx.execute("INSERT INTO login_attempts(address,count,window_start) VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=CASE WHEN window_start<?2-300 THEN 1 ELSE count+1 END,window_start=CASE WHEN window_start<?2-300 THEN ?2 ELSE window_start END",params![scope,now()])?;
        let count:i64=tx.query_row("SELECT count FROM login_attempts WHERE address=?1",[scope],|r|r.get(0))?;
        let total:i64=tx.query_row("SELECT COUNT(*) FROM quick_connect",[],|r|r.get(0))?;
        if count>10 || total>=1000 {tx.commit()?;return Ok(None);}
        for _ in 0..16 {
            let secret=thelxinoe_auth::token();
            let hash=thelxinoe_auth::digest(&secret);
            let code=format!("{:06}",u32::from_str_radix(&secret[..6],16).unwrap()%1_000_000);
            let code_hash=thelxinoe_auth::digest(&code);
            if tx.execute("INSERT INTO quick_connect(secret_hash,code_hash,device_id,device_name,client,version,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(code_hash) DO NOTHING",params![hash,code_hash,device2.id,device2.name,device2.client,device2.version,now()+300])?==1 {
                let date:String=tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')",[],|r|r.get(0))?;
                tx.commit()?;return Ok(Some((secret,code,date)));
            }
        }
        tx.commit()?;Ok(None)
    }).await
}

pub(super) async fn status(
    hash: String,
    code: String,
    secret: String,
    db: &Database,
) -> anyhow::Result<Option<Value>> {
    db.read("jellyfin.quick_connect.status", move|db|Ok(db.query_row("SELECT user_id IS NOT NULL,device_id,device_name,client,version,strftime('%Y-%m-%dT%H:%M:%SZ',expires_at-300,'unixepoch') FROM quick_connect WHERE secret_hash=?1 AND expires_at>?2",params![hash,now()],|r|Ok(json!({"Authenticated":r.get::<_,bool>(0)?,"Secret":secret,"Code":code,"DeviceId":r.get::<_,String>(1)?,"DeviceName":r.get::<_,String>(2)?,"AppName":r.get::<_,String>(3)?,"AppVersion":r.get::<_,String>(4)?,"DateAdded":r.get::<_,String>(5)?}))).optional()?)).await
}

pub(super) async fn exchange(
    hash: String,
    db: &Database,
) -> anyhow::Result<Option<(String, Device)>> {
    db.write("jellyfin.quick_connect.exchange", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT q.user_id,q.device_id,q.device_name,q.client,q.version FROM quick_connect q JOIN sessions s ON s.id=q.authorizer_session_id AND s.user_id=q.user_id WHERE q.secret_hash=?1 AND q.expires_at>?2 AND s.expires_at>?2",params![hash,now()],|r|Ok((r.get::<_,String>(0)?,Device{id:r.get(1)?,name:r.get(2)?,client:r.get(3)?,version:r.get(4)?}))).optional()?;
        if row.is_some(){tx.execute("DELETE FROM quick_connect WHERE secret_hash=?1",[hash])?;}
        tx.commit()?;Ok(row)
    }).await
}

pub(super) async fn inspect(
    db: &Database,
    code: String,
    uid: String,
) -> anyhow::Result<Option<Value>> {
    db.write("jellyfin.quick_connect.inspect", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let scope=format!("quick-connect-lookup:{uid}");
        tx.execute("INSERT INTO login_attempts(address,count,window_start) VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=CASE WHEN window_start<?2-300 THEN 1 ELSE count+1 END,window_start=CASE WHEN window_start<?2-300 THEN ?2 ELSE window_start END",params![scope,now()])?;
        let count:i64=tx.query_row("SELECT count FROM login_attempts WHERE address=?1",[scope],|r|r.get(0))?;
        if count>20{tx.commit()?;return Ok(None);}
        let row=tx.query_row("SELECT device_name,client,version FROM quick_connect WHERE code_hash=?1 AND expires_at>?2 AND user_id IS NULL",params![code,now()],|r|Ok(json!({"device":r.get::<_,String>(0)?,"client":r.get::<_,String>(1)?,"version":r.get::<_,String>(2)?}))).optional()?;
        tx.commit()?;Ok(row)
    }).await
}

pub(super) async fn approve(
    db: &Database,
    p: thelxinoe_core::Principal,
    hash: String,
) -> anyhow::Result<usize> {
    db.write("jellyfin.quick_connect.approve", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n=tx.execute("UPDATE quick_connect SET user_id=?1,authorizer_session_id=?2 WHERE code_hash=?3 AND expires_at>?4 AND user_id IS NULL",params![p.user.id,p.session_id,hash,now()])?;
        if n>0 {tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'device.quick-connect',?2,?3)",params![p.user.id,"Compatibility device",now()])?;}
        tx.commit()?;Ok(n)
    }).await
}
