#[path = "../storage/jellyfin/quick_connect.rs"]
mod storage;

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
    let (secret, code, date) = storage::initiate(scope, device2, &state.db)
        .await?
        .ok_or_else(|| ApiError::bad("Too many Quick Connect requests; wait five minutes"))?;
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
    storage::status(hash, code, secret, &state.db)
        .await?
        .ok_or_else(ApiError::unauthorized)
}
pub async fn exchange(state: &AppState, secret: &str) -> Result<Value> {
    if secret.len() != 64 {
        return Err(ApiError::unauthorized());
    }
    let hash = thelxinoe_auth::digest(secret);
    let (user, device) = storage::exchange(hash, &state.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
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
    let mut device = storage::inspect(&state.db, code, uid)
        .await?
        .ok_or_else(|| {
            ApiError::bad("Code is unavailable or expired; check the TV or wait before retrying")
        })?;
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
    let updated = storage::approve(&state.db, p, hash).await?;
    if updated != 1 {
        return Err(ApiError::bad(
            "This code has expired or was already approved",
        ));
    }
    Ok(Json(json!({"approved":true})))
}
