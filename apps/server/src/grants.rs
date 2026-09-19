use crate::{AppState, error::Result, security};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub async fn issue(
    state: &AppState,
    p: &Principal,
    resource: &str,
    ttl: i64,
) -> anyhow::Result<String> {
    let raw = thelxinoe_auth::token();
    let hash = thelxinoe_auth::digest(&raw);
    let p = p.clone();
    let resource = resource.to_string();
    state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("DELETE FROM playback_grants WHERE expires_at<=?1", [now()])?;
            tx.execute(
                "INSERT INTO playback_grants VALUES (?1,?2,?3,?4,?5)",
                params![hash, p.user.id, p.session_id, resource, now() + ttl],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(raw)
}
pub async fn resolve(
    state: &AppState,
    token: &str,
    resource: &str,
    consume: bool,
) -> anyhow::Result<Option<Principal>> {
    let hash = thelxinoe_auth::digest(token);
    let resource = resource.to_string();
    state.db.call(move|db|{
        let query=|db:&rusqlite::Connection|db.query_row("SELECT u.id,u.username,u.role,u.timezone,s.id,s.transport FROM playback_grants g JOIN sessions s ON s.id=g.session_id JOIN users u ON u.id=g.user_id WHERE g.token_hash=?1 AND g.resource=?2 AND g.expires_at>?3 AND s.expires_at>?3",params![hash,resource,now()],|r|Ok(Principal{user:thelxinoe_auth::user_row(r)?,session_id:r.get(4)?,transport:r.get(5)?})).optional();
        if !consume{return Ok(query(db)?);}
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let principal=query(&tx)?;
        tx.execute("DELETE FROM playback_grants WHERE token_hash=?1",[hash])?;tx.commit()?;Ok(principal)
    }).await
}
pub async fn event_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let cursor = state
        .db
        .call(|db| {
            Ok(
                db.query_row("SELECT COALESCE(MAX(id),0) FROM events", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await?;
    Ok(Json(
        json!({"ticket":issue(&state,&p,"events",30).await?,"cursor":cursor}),
    ))
}
