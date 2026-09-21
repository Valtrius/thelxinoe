use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use thelxinoe_auth::{
    issue_session, password_hash, user_row, validate_credentials, verify_password,
};
use thelxinoe_core::{Capability, Role, id, now};

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
}

pub async fn setup_status(State(state): State<AppState>) -> Result<Json<Value>> {
    let count = state
        .db
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))?))
        .await?;
    Ok(Json(
        json!({"setup_required":count==0,"version":thelxinoe_core::VERSION}),
    ))
}
async fn respond_session(
    state: &AppState,
    user_id: String,
    c: &Credentials,
    secure: bool,
) -> Result<Response> {
    let transport = c.transport.as_deref().unwrap_or("web");
    if !matches!(transport, "web" | "device") {
        return Err(ApiError::bad("Unsupported credential transport"));
    }
    let raw = issue_session(
        &state.db,
        user_id.clone(),
        transport.into(),
        c.device_name
            .as_deref()
            .unwrap_or(if transport == "web" {
                "Browser"
            } else {
                "Desktop"
            })
            .chars()
            .take(100)
            .collect(),
    )
    .await?;
    let user = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT id,username,role,timezone FROM user_profiles WHERE id=?1",
                [user_id],
                user_row,
            )?)
        })
        .await?;
    let user = crate::avatars::profile(state, user).await?;
    let mut response =
        Json(json!({"user":user,"token":if transport=="device"{Some(&raw)}else{None}}))
            .into_response();
    if transport == "web" {
        response.headers_mut().insert(
            header::SET_COOKIE,
            security::cookie(&raw, secure).parse().unwrap(),
        );
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}
pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> Result<Response> {
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let context = security::request_context(&state.config, &headers, peer)?;
    if !matches!(c.transport.as_deref().unwrap_or("web"), "web" | "device") {
        return Err(ApiError::bad("Unsupported transport"));
    }
    validate_credentials(&c.username, &c.password).map_err(|e| ApiError::bad(e.to_string()))?;
    let hash = password_hash(c.password.clone()).await?;
    let user_id = id();
    let uid = user_id.clone();
    let username = c.username.clone();
    let created = state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? != 0 {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO users(id,username,password_hash,role,timezone,created_at) VALUES (?1,?2,?3,'admin','UTC',?4)",
                params![uid, username, hash, now()],
            )?;
            tx.execute(
                "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'setup',?1,?2)",
                params![uid, now()],
            )?;
            tx.commit()?;
            Ok(true)
        })
        .await?;
    if !created {
        return Err(ApiError::conflict("Setup is already complete"));
    }
    respond_session(&state, user_id, &c, context.secure).await
}
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> Result<Response> {
    let context = security::request_context(&state.config, &headers, peer)?;
    let user = check_credentials(&state, context.address, &c.username, &c.password).await?;
    respond_session(&state, user, &c, context.secure).await
}
pub(crate) async fn check_credentials(
    state: &AppState,
    address: std::net::IpAddr,
    username: &str,
    password: &str,
) -> Result<String> {
    allow_password_attempt(state, address.to_string()).await?;
    if password.len() > 256 || username.len() > 64 {
        return Err(ApiError::unauthorized());
    }
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let username = username.to_owned();
    let record = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT id,password_hash FROM users WHERE username=?1",
                    [username],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await?;
    let hash = record
        .as_ref()
        .map(|r| r.1.clone())
        .unwrap_or_else(|| state.dummy_hash.as_ref().clone());
    let valid = verify_password(password.to_owned(), hash).await?;
    if !valid || record.is_none() {
        return Err(ApiError::unauthorized());
    }
    Ok(record.unwrap().0)
}
async fn allow_password_attempt(state: &AppState, address: String) -> Result<()> {
    let allowed=state.db.call(move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM login_attempts WHERE window_start<?1",[now()-900])?;
        let count:i64=tx.query_row("INSERT INTO login_attempts VALUES (?1,1,?2) ON CONFLICT(address) DO UPDATE SET count=count+1 RETURNING count",params![address,now()],|r|r.get(0))?;
        tx.commit()?;Ok(count<=20)
    }).await?;
    if !allowed {
        return Err(ApiError(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts. Try again in 15 minutes.".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordChange>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    validate_credentials(&principal.user.username, &input.new_password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    if input.current_password.len() > 256 {
        return Err(ApiError::bad("Current password is incorrect"));
    }
    allow_password_attempt(&state, format!("password:{}", principal.user.id)).await?;
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let user_id = principal.user.id.clone();
    let previous_hash = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT password_hash FROM users WHERE id=?1",
                [user_id],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await?;
    if !verify_password(input.current_password, previous_hash.clone()).await? {
        return Err(ApiError::bad("Current password is incorrect"));
    }
    let hash = password_hash(input.new_password).await?;
    let user_id = principal.user.id.clone();
    let changed = state.db.call(move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A concurrent reset or session revocation must invalidate this change.
        let changed = tx.execute(
            "UPDATE users SET password_hash=?1 WHERE id=?2 AND password_hash=?3
             AND EXISTS(SELECT 1 FROM sessions WHERE id=?4 AND user_id=?2 AND expires_at>?5)",
            params![hash, user_id, previous_hash, principal.session_id, now()],
        )? == 1;
        if changed {
            tx.execute("DELETE FROM sessions WHERE user_id=?1 AND id<>?2", params![user_id, principal.session_id])?;
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.password',?1,?2)", params![user_id, now()])?;
        }
        tx.commit()?;
        Ok(changed)
    }).await?;
    if !changed {
        return Err(ApiError::conflict(
            "Your account changed. Sign in again before changing your password",
        ));
    }
    state
        .emit(
            Some(principal.user.id),
            "account.password.changed",
            json!({}),
        )
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = security::principal(&state, &headers).await?.user;
    Ok(Json(
        json!({"user":crate::avatars::profile(&state, user).await?}),
    ))
}
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let p = security::principal(&state, &headers).await?;
    state
        .db
        .call(move |db| {
            db.execute("DELETE FROM sessions WHERE id=?1", [p.session_id])?;
            Ok(())
        })
        .await?;
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        "thelxinoe_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(response)
}
pub async fn sessions(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let current = p.session_id;
    let rows=state.db.call(move |db|{
        let mut query=db.prepare("SELECT id,name,transport,created_at,expires_at,last_seen FROM sessions WHERE user_id=?1 AND expires_at>?2 ORDER BY last_seen DESC")?;
        Ok(query.query_map(params![p.user.id,now()],|r|Ok(thelxinoe_auth::Session{id:r.get(0)?,name:r.get(1)?,transport:r.get(2)?,created_at:r.get(3)?,expires_at:r.get(4)?,last_seen:r.get(5)?}))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await?;
    Ok(Json(json!({"items":rows,"current":current})))
}
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    state.db.call(move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM sessions WHERE id=?1 AND (user_id=?2 OR ?3)",params![session,p.user.id,p.user.role.allows(Capability::ManageUsers)])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'session.revoke',?2,?3)",params![p.user.id,session,now()])?;tx.commit()?;Ok(())
    }).await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn users(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let rows = state
        .db
        .call(|db| {
            Ok(db
                .prepare("SELECT id,username,role,timezone FROM user_profiles ORDER BY username")?
                .query_map([], user_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    password: String,
    role: Role,
}
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(user): Json<NewUser>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    validate_credentials(&user.username, &user.password)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = password_hash(user.password).await?;
    let result=state.db.call(move |db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let uid=id();
        let n=tx.execute("INSERT OR IGNORE INTO users(id,username,password_hash,role,timezone,created_at) VALUES (?1,?2,?3,?4,'UTC',?5)",params![uid,user.username,hash,user.role.as_str(),now()])?;
        if n==0{return Ok(None);}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.create',?2,?3)",params![p.user.id,uid,now()])?;tx.commit()?;Ok(Some(uid))
    }).await?;
    result
        .map(|id| Json(json!({"id":id})))
        .ok_or_else(|| ApiError::conflict("Username already exists"))
}
pub async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let timezone = state
        .db
        .call(|db| {
            Ok(db
                .query_row("SELECT value FROM settings WHERE key='timezone'", [], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .unwrap_or("UTC".into()))
        })
        .await?;
    Ok(Json(
        json!({"timezone":timezone,"public_url":state.config.public_url.as_ref().map(ToString::to_string),"trusted_proxies":state.config.trusted_proxies.iter().map(ToString::to_string).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
pub struct Settings {
    timezone: String,
}
pub async fn save_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<Settings>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    settings
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::bad("Unknown timezone"))?;
    let timezone = settings.timezone.clone();
    state.db.call(move |db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;tx.execute("INSERT INTO settings VALUES ('timezone',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[settings.timezone])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'settings.update','server',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(())}).await?;
    state
        .emit(
            None,
            "server.settings.changed",
            json!({"timezone":timezone}),
        )
        .await?;
    Ok(Json(json!({"ok":true})))
}
