#[path = "storage/grants.rs"]
mod storage;

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
    storage::issue(hash, p, resource, &state.db, ttl).await?;
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
    storage::resolve(hash, resource, &state.db, consume).await
}
pub async fn event_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let cursor = storage::event_ticket(&state.db).await?;
    Ok(Json(
        json!({"ticket":issue(&state,&p,"events",30).await?,"cursor":cursor}),
    ))
}
