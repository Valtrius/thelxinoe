//! Database operations for managers.requests.
use crate::managers::domain::{
    AcquisitionRequest, CreateAcquisition, DecideAcquisition, RequestAction, RequestReceipt,
    RequestState,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{id, now};
use thelxinoe_database::Database;

pub(super) async fn services(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.requests.services", |db|Ok(db.prepare("SELECT id,name,kind,defaults<>'{}' FROM manager_services WHERE enabled=1 ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"ready":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn search(
    db: &Database,
    domain: &'static str,
    term: String,
) -> anyhow::Result<Vec<Value>> {
    db.read("managers.requests.search", move|db|Ok(db.prepare("SELECT id,title,year,(WITH RECURSIVE children(id) AS (SELECT card.id UNION ALL SELECT m.id FROM media m JOIN children c ON m.parent_id=c.id) SELECT EXISTS(SELECT 1 FROM children JOIN media_sources s ON s.media_id=children.id JOIN media_files f ON f.id=s.file_id WHERE f.present=1)) FROM media_cards card WHERE kind=?1 AND instr(lower(title),lower(?2))>0 ORDER BY title LIMIT 50")?.query_map(params![domain,term],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"year":r.get::<_,Option<i64>>(2)?,"available":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn list(
    db: &Database,
    p: thelxinoe_core::Principal,
    admin: bool,
    page: u32,
) -> anyhow::Result<Value> {
    db.read("managers.requests.list", move |db| {
        let total: i64 = db.query_row("SELECT COUNT(*) FROM acquisition_requests WHERE ?1 OR user_id=?2", params![admin,p.user.id], |r| r.get(0))?;
        let pages = ((total + 199) / 200).max(1);
        let page = i64::from(page).clamp(1,pages);
        let items = db.prepare("SELECT r.id,r.title,r.state,r.created_at,r.updated_at,r.manager_id,r.error,u.username,s.name,s.kind,r.user_id,r.generation FROM acquisition_requests r JOIN users u ON u.id=r.user_id JOIN manager_services s ON s.id=r.service_id WHERE ?1 OR r.user_id=?2 ORDER BY r.created_at DESC,r.id DESC LIMIT 200 OFFSET ?3")?.query_map(params![admin,p.user.id,(page-1)*200], |r| {
            let id: String = r.get(0)?;
            let state: String = r.get(2)?;
            let updated: i64 = r.get(4)?;
            let owner: String = r.get(10)?;
            let attention = if owner == p.user.id { Some(crate::operations::RequestAttention {
                id: format!("request:{id}"),
                revision: crate::operations::native_request_revision(&r.get::<_,String>(11)?, &state, updated),
            }) } else { None };
            Ok(json!({"id":id,"title":r.get::<_,String>(1)?,"state":state,"created_at":r.get::<_,i64>(3)?,"updated_at":updated,"manager_id":r.get::<_,Option<i64>>(5)?,"error":r.get::<_,Option<String>>(6)?,"username":r.get::<_,String>(7)?,"service":r.get::<_,String>(8)?,"kind":r.get::<_,String>(9)?,"user_id":owner,"attention":attention}))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"items":items,"page":page,"pages":pages}))
    }).await
}

pub(super) async fn request(
    db: &Database,
    input: CreateAcquisition,
) -> anyhow::Result<RequestReceipt> {
    db.write("managers.requests.request", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((id,state))=tx.query_row("SELECT id,state FROM acquisition_requests WHERE user_id=?1 AND service_id=?2 AND external_id=?3 AND state NOT IN ('denied','cancelled','failed')",params![input.user_id,input.service_id,input.external_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?{return Ok(RequestReceipt{id,state:serde_json::from_value(json!(state))?})}
        let auto=input.administrator||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&input.user_id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let key=id();let state=if auto{RequestState::Approved}else{RequestState::Pending};
        tx.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![key,input.user_id,input.service_id,input.service_generation,input.external_id,input.title,state.as_str(),now()])?;
        if auto{enqueue(&tx,&key)?;}tx.commit()?;Ok(RequestReceipt{id:key,state})}).await
}

pub(super) async fn decide(db: &Database, input: DecideAcquisition) -> anyhow::Result<bool> {
    db.write("managers.requests.decide", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let key=&input.id;
        let row=tx.query_row("SELECT r.user_id,r.state,s.generation FROM acquisition_requests r JOIN manager_services s ON s.id=r.service_id WHERE r.id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        let Some((owner,status,generation))=row else{return Ok(false)};
        if owner!=input.actor&&!input.administrator{return Ok(false)}
        if matches!(input.action,RequestAction::Approve|RequestAction::Deny)&&!input.administrator{return Ok(false)}
        let state:RequestState=serde_json::from_value(json!(status))?;
        let reacquire=input.action==RequestAction::Reacquire;
        if if reacquire {!matches!(state,RequestState::Requested|RequestState::Available)}else{!matches!(state,RequestState::Pending|RequestState::Failed|RequestState::Uncertain)} {return Ok(false)}
        let auto=input.administrator||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&owner],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let next=match input.action{RequestAction::Approve=>RequestState::Approved,RequestAction::Deny=>RequestState::Denied,RequestAction::Reacquire=>if auto{RequestState::Approved}else{RequestState::Pending},RequestAction::Cancel=>RequestState::Cancelled};
        tx.execute("UPDATE acquisition_requests SET state=?1,generation=?2,error=NULL,updated_at=?3 WHERE id=?4",params![next.as_str(),generation,now(),key])?;
        if next==RequestState::Approved{enqueue(&tx,key)?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![input.actor,format!("request.{}",input.action.as_str()),key,now()])?;tx.commit()?;Ok(true)}).await
}

pub(super) async fn approval_users(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.requests.approval_users", |db| Ok(db.prepare("SELECT u.id,u.username,COALESCE(a.auto_approve,0) FROM users u LEFT JOIN acquisition_users a ON a.user_id=u.id WHERE u.role <> 'admin' ORDER BY u.username")?.query_map([],|r| Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn auto_approve(
    db: &Database,
    user: String,
    enabled: bool,
    actor: String,
) -> anyhow::Result<()> {
    db.write("managers.requests.auto_approve", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO acquisition_users VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET auto_approve=excluded.auto_approve",params![user,enabled])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.auto_approve',?2,?3)",params![actor,user,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn update(
    key: String,
    status: RequestState,
    db: &Database,
    manager: Option<i64>,
) -> anyhow::Result<()> {
    db.write("managers.requests.update", move|db|{db.execute("UPDATE acquisition_requests SET state=?1,manager_id=COALESCE(?2,manager_id),updated_at=?3 WHERE id=?4",params![status.as_str(),manager,now(),key])?;Ok(())}).await
}

pub(super) async fn acquire(key: String, message: String, db: &Database) -> anyhow::Result<()> {
    db.write("managers.requests.acquire", move|db|{db.execute("UPDATE acquisition_requests SET state=CASE WHEN state IN ('searching','uncertain') THEN 'uncertain' ELSE 'failed' END,error=?1,updated_at=?2 WHERE id=?3",params![message,now(),key])?;Ok(())}).await
}

pub(super) async fn perform(
    request_id: String,
    db: &Database,
) -> anyhow::Result<Option<AcquisitionRequest>> {
    db.read("managers.requests.perform", move|db|{
        let row=db.query_row("SELECT service_id,generation,external_id,state FROM acquisition_requests WHERE id=?1",[request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
        row.map(|(service_id,service_generation,external_id,state)|Ok(AcquisitionRequest{service_id,service_generation,external_id,state:serde_json::from_value(json!(state))?})).transpose()
    }).await
}

pub(super) fn enqueue(tx: &rusqlite::Transaction<'_>, request: &str) -> anyhow::Result<()> {
    let key = id();
    tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'manager.request',?2,?1,'queued',?3,?3)",params![key,json!({"request_id":request}).to_string(),now()])?;
    Ok(())
}
