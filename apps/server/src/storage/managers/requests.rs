//! Database operations for managers.requests.
use super::*;
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
) -> anyhow::Result<Vec<Value>> {
    db.read("managers.requests.list", move|db|Ok(db.prepare("SELECT r.id,r.title,r.state,r.created_at,r.updated_at,r.manager_id,r.error,u.username,s.name,s.kind,r.user_id FROM acquisition_requests r JOIN users u ON u.id=r.user_id JOIN manager_services s ON s.id=r.service_id WHERE ?1 OR r.user_id=?2 ORDER BY r.created_at DESC LIMIT 200")?.query_map(params![admin,p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"updated_at":r.get::<_,i64>(4)?,"manager_id":r.get::<_,Option<i64>>(5)?,"error":r.get::<_,Option<String>>(6)?,"username":r.get::<_,String>(7)?,"service":r.get::<_,String>(8)?,"kind":r.get::<_,String>(9)?,"user_id":r.get::<_,String>(10)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn request(
    db: &Database,
    input: Request,
    p: thelxinoe_core::Principal,
    s: Service,
    title: String,
) -> anyhow::Result<(String, String)> {
    db.write("managers.requests.request", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(existing)=tx.query_row("SELECT id,state FROM acquisition_requests WHERE user_id=?1 AND service_id=?2 AND external_id=?3 AND state NOT IN ('denied','cancelled','failed')",params![p.user.id,s.id,input.external_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?{return Ok(existing)}
        let auto=p.user.role==Role::Admin||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&p.user.id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let key=id();let status=if auto{"approved"}else{"pending"};
        tx.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![key,p.user.id,s.id,s.generation,input.external_id,title,status,now()])?;
        if auto{enqueue(&tx,&key)?;}tx.commit()?;Ok((key,status.into()))}).await
}

pub(super) async fn decide(
    db: &Database,
    key: String,
    input: Decision,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<bool> {
    db.write("managers.requests.decide", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT r.user_id,r.state,s.generation FROM acquisition_requests r JOIN manager_services s ON s.id=r.service_id WHERE r.id=?1",[&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        let Some((owner,status,generation))=row else{return Ok(false)};
        if owner!=p.user.id&&p.user.role!=Role::Admin{return Ok(false)}
        let reacquire=input.action=="reacquire";
        if if reacquire {!matches!(status.as_str(),"requested"|"available")}else{!matches!(status.as_str(),"pending"|"failed"|"uncertain")} {return Ok(false)}
        let auto=p.user.role==Role::Admin||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&owner],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let next=match input.action.as_str(){"approve"=>"approved","deny"=>"denied","reacquire"=>if auto{"approved"}else{"pending"},_=>"cancelled"};
        tx.execute("UPDATE acquisition_requests SET state=?1,generation=?2,error=NULL,updated_at=?3 WHERE id=?4",params![next,generation,now(),key])?;
        if next=="approved"{enqueue(&tx,&key)?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![p.user.id,format!("request.{}",input.action),key,now()])?;tx.commit()?;Ok(true)}).await
}

pub(super) async fn approval_users(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.requests.approval_users", |db| Ok(db.prepare("SELECT u.id,u.username,COALESCE(a.auto_approve,0) FROM users u LEFT JOIN acquisition_users a ON a.user_id=u.id WHERE u.role <> 'admin' ORDER BY u.username")?.query_map([],|r| Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn auto_approve(
    db: &Database,
    user: String,
    input: Auto,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.requests.auto_approve", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO acquisition_users VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET auto_approve=excluded.auto_approve",params![user,input.enabled])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.auto_approve',?2,?3)",params![p.user.id,user,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn update(
    key: String,
    status: String,
    db: &Database,
    manager: Option<i64>,
) -> anyhow::Result<()> {
    db.write("managers.requests.update", move|db|{db.execute("UPDATE acquisition_requests SET state=?1,manager_id=COALESCE(?2,manager_id),updated_at=?3 WHERE id=?4",params![status,manager,now(),key])?;Ok(())}).await
}

pub(super) async fn acquire(key: String, message: String, db: &Database) -> anyhow::Result<()> {
    db.write("managers.requests.acquire", move|db|{db.execute("UPDATE acquisition_requests SET state=CASE WHEN state IN ('searching','uncertain') THEN 'uncertain' ELSE 'failed' END,error=?1,updated_at=?2 WHERE id=?3",params![message,now(),key])?;Ok(())}).await
}

pub(super) async fn perform(
    request_id: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String, String, String)>> {
    db.read("managers.requests.perform", move|db|Ok(db.query_row("SELECT service_id,generation,external_id,state FROM acquisition_requests WHERE id=?1",[request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?)).await
}

pub(super) fn enqueue(tx: &rusqlite::Transaction<'_>, request: &str) -> anyhow::Result<()> {
    let key = id();
    tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'manager.request',?2,?1,'queued',?3,?3)",params![key,json!({"request_id":request}).to_string(),now()])?;
    Ok(())
}
