use super::auth::{self, Device};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

pub async fn initiate(
    state: &AppState,
    device: Device,
    address: std::net::IpAddr,
) -> Result<Value> {
    let scope = format!("quick-connect:{address}");
    let device2 = device.clone();
    let (secret,code,date)=state.db.call(move|db|{
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
    }).await?.ok_or_else(||ApiError::bad("Too many Quick Connect requests; wait five minutes"))?;
    Ok(
        json!({"Authenticated":false,"Secret":secret,"Code":code,"DeviceId":device.id,"DeviceName":device.name,"AppName":device.client,"AppVersion":device.version,"DateAdded":date}),
    )
}
pub async fn status(state: &AppState, secret: &str) -> Result<Value> {
    if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::unauthorized());
    }
    let hash = thelxinoe_auth::digest(secret);
    let code = format!(
        "{:06}",
        u32::from_str_radix(&secret[..6], 16).map_err(|_| ApiError::unauthorized())? % 1_000_000
    );
    let secret = secret.to_owned();
    state.db.call(move|db|Ok(db.query_row("SELECT user_id IS NOT NULL,device_id,device_name,client,version,strftime('%Y-%m-%dT%H:%M:%SZ',expires_at-300,'unixepoch') FROM quick_connect WHERE secret_hash=?1 AND expires_at>?2",params![hash,now()],|r|Ok(json!({"Authenticated":r.get::<_,bool>(0)?,"Secret":secret,"Code":code,"DeviceId":r.get::<_,String>(1)?,"DeviceName":r.get::<_,String>(2)?,"AppName":r.get::<_,String>(3)?,"AppVersion":r.get::<_,String>(4)?,"DateAdded":r.get::<_,String>(5)?}))).optional()?)).await?.ok_or_else(ApiError::unauthorized)
}
pub async fn exchange(state: &AppState, secret: &str) -> Result<Value> {
    if secret.len() != 64 {
        return Err(ApiError::unauthorized());
    }
    let hash = thelxinoe_auth::digest(secret);
    let (user,device)=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT q.user_id,q.device_id,q.device_name,q.client,q.version FROM quick_connect q JOIN sessions s ON s.id=q.authorizer_session_id AND s.user_id=q.user_id WHERE q.secret_hash=?1 AND q.expires_at>?2 AND s.expires_at>?2",params![hash,now()],|r|Ok((r.get::<_,String>(0)?,Device{id:r.get(1)?,name:r.get(2)?,client:r.get(3)?,version:r.get(4)?}))).optional()?;
        if row.is_some(){tx.execute("DELETE FROM quick_connect WHERE secret_hash=?1",[hash])?;}
        tx.commit()?;Ok(row)
    }).await?.ok_or_else(ApiError::unauthorized)?;
    auth::issue(state, user, device).await
}
#[derive(Deserialize)]
pub struct Approval {
    code: String,
    confirmation: Option<String>,
}
pub async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Approval>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if input.code.len() != 6 || !input.code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ApiError::bad("Enter the six-digit code shown on your TV"));
    }
    let hash = thelxinoe_auth::digest(&input.code);
    let code = hash.clone();
    let uid = p.session_id.clone();
    let mut device=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let scope=format!("quick-connect-lookup:{uid}");
        tx.execute("INSERT INTO login_attempts(address,count,window_start) VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=CASE WHEN window_start<?2-300 THEN 1 ELSE count+1 END,window_start=CASE WHEN window_start<?2-300 THEN ?2 ELSE window_start END",params![scope,now()])?;
        let count:i64=tx.query_row("SELECT count FROM login_attempts WHERE address=?1",[scope],|r|r.get(0))?;
        if count>20{tx.commit()?;return Ok(None);}
        let row=tx.query_row("SELECT device_name,client,version FROM quick_connect WHERE code_hash=?1 AND expires_at>?2 AND user_id IS NULL",params![code,now()],|r|Ok(json!({"device":r.get::<_,String>(0)?,"client":r.get::<_,String>(1)?,"version":r.get::<_,String>(2)?}))).optional()?;
        tx.commit()?;Ok(row)
    }).await?.ok_or_else(||ApiError::bad("Code is unavailable or expired; check the TV or wait before retrying"))?;
    device["confirmation"] =
        json!(crate::grants::issue(&state, &p, &format!("quick-connect:{hash}"), 120).await?);
    Ok(Json(device))
}
pub async fn approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Approval>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let hash = thelxinoe_auth::digest(&input.code);
    let grant = crate::grants::resolve(
        &state,
        input.confirmation.as_deref().unwrap_or_default(),
        &format!("quick-connect:{hash}"),
        true,
    )
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    if grant.session_id != p.session_id {
        return Err(ApiError::forbidden());
    }
    let updated=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n=tx.execute("UPDATE quick_connect SET user_id=?1,authorizer_session_id=?2 WHERE code_hash=?3 AND expires_at>?4 AND user_id IS NULL",params![p.user.id,p.session_id,hash,now()])?;
        if n>0 {tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'device.quick-connect',?2,?3)",params![p.user.id,"Compatibility device",now()])?;}
        tx.commit()?;Ok(n)
    }).await?;
    if updated != 1 {
        return Err(ApiError::bad(
            "This code has expired or was already approved",
        ));
    }
    Ok(Json(json!({"approved":true})))
}
