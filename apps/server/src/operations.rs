//! Administrative observations never include credentials or arbitrary upstream payloads.
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, put},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/me/notifications", get(notifications))
        .route("/api/v1/me/notifications/{id}", put(read))
        .route("/api/v1/admin/operations", get(dashboard))
        .route("/api/v1/admin/diagnostics", get(diagnostics))
        .route("/api/v1/admin/devices", get(devices))
        .route("/api/v1/admin/backups", get(backups).post(backup))
        .route(
            "/api/v1/admin/backups/{id}/restore",
            axum::routing::post(restore),
        )
        .route("/api/v1/users/{id}", put(change_user).delete(delete_user))
}
async fn backups(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(
        crate::managers::controller_request(&state, "/backups", None).await?,
    ))
}
#[derive(serde::Deserialize)]
struct BackupInput {
    passphrase: String,
    #[serde(default)]
    confirm: bool,
}
async fn backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BackupInput>,
) -> Result<Json<Value>> {
    backup_command(state, headers, input, None).await
}
async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<BackupInput>,
) -> Result<Json<Value>> {
    if uuid::Uuid::parse_str(&key).is_err() {
        return Err(ApiError::bad("Invalid backup ID"));
    }
    backup_command(state, headers, input, Some(key)).await
}
async fn backup_command(
    state: AppState,
    headers: HeaderMap,
    input: BackupInput,
    restore: Option<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !input.confirm {
        return Err(ApiError::bad("Confirm the temporary service interruption"));
    }
    let busy=state.db.call(|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?1-120) OR EXISTS(SELECT 1 FROM jobs WHERE state='running')",[now()],|r|r.get::<_,bool>(0))?)).await?;
    if busy {
        return Err(ApiError::conflict(
            "Wait for active playback and background work to finish",
        ));
    }
    let path = restore
        .as_ref()
        .map_or("/backups".into(), |key| format!("/backups/{key}/restore"));
    state
        .db
        .call(move |db| {
            db.execute(
                "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,'server',?3)",
                params![
                    p.user.id,
                    if restore.is_some() {
                        "backup.restore"
                    } else {
                        "backup.create"
                    },
                    now()
                ],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(
        crate::managers::controller_request(
            &state,
            &path,
            Some(json!({"passphrase":input.passphrase})),
        )
        .await?,
    ))
}
#[derive(serde::Deserialize)]
struct UserChange {
    role: thelxinoe_core::Role,
    password: Option<String>,
}
async fn change_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<UserChange>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    let _slot = state
        .password_slots
        .acquire()
        .await
        .map_err(anyhow::Error::from)?;
    let hash = if let Some(password) = input.password {
        thelxinoe_auth::validate_credentials("valid-user", &password)
            .map_err(|e| ApiError::bad(e.to_string()))?;
        Some(thelxinoe_auth::password_hash(password).await?)
    } else {
        None
    };
    let status=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT role FROM users WHERE id=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        let Some(current)=current else{return Ok(404);};
        if current=="admin" && input.role==thelxinoe_core::Role::User && tx.query_row("SELECT COUNT(*) FROM users WHERE role='admin'",[],|r|r.get::<_,i64>(0))?<=1{return Ok(409);}
        tx.execute("UPDATE users SET role=?1,password_hash=COALESCE(?2,password_hash) WHERE id=?3",params![input.role.as_str(),hash,key])?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.update',?2,?3)",params![p.user.id,key,now()])?;
        tx.commit()?;Ok(200)
    }).await?;
    match status {
        404 => Err(ApiError::not_found()),
        409 => Err(ApiError::conflict("Keep at least one administrator")),
        _ => Ok(Json(json!({"saved":true}))),
    }
}
async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    if key == p.user.id {
        return Err(ApiError::conflict(
            "Use another administrator account to delete this account",
        ));
    }
    let _lease = state.media_operations.write().await;
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current=tx.query_row("SELECT role FROM users WHERE id=?1",[&key],|r|r.get::<_,String>(0)).optional()?;
        if current.is_none(){return Ok(None);}
        let playbacks=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1")?.query_map([&key],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        // Provisioned services survive removal of the administrator who installed them.
        tx.execute("UPDATE stack_provisions SET actor_id=?1 WHERE actor_id=?2",params![p.user.id,key])?;
        tx.execute("DELETE FROM events WHERE user_id=?1",[&key])?;
        tx.execute("UPDATE retention_policies SET trigger_users=(SELECT COALESCE(json_group_array(value),'[]') FROM json_each(trigger_users) WHERE value<>?1),updated_at=?2 WHERE EXISTS(SELECT 1 FROM json_each(trigger_users) WHERE value=?1)",params![key,now()])?;
        tx.execute("UPDATE audit SET actor_id=NULL WHERE actor_id=?1",[&key])?;
        tx.execute("DELETE FROM users WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'user.delete',?2,?3)",params![p.user.id,key,now()])?;
        tx.commit()?;Ok(Some(playbacks))
    }).await?.ok_or_else(ApiError::not_found)?;
    for session in result {
        state.playback.stop(&session).await;
    }
    state.emit(None, "users.changed", json!({})).await?;
    Ok(Json(json!({"deleted":true})))
}
async fn notifications(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items=state.db.call(move|db| Ok(db.prepare("SELECT id,severity,message,created_at,read_at FROM notifications WHERE user_id=?1 ORDER BY created_at DESC,id LIMIT 200")?.query_map([p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"severity":r.get::<_,String>(1)?,"message":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"read_at":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":items})))
}
async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE notifications SET read_at=?1 WHERE user_id=?2 AND (id=?3 OR ?3='all')",
                params![now(), user, key],
            )?;
            Ok(())
        })
        .await?;
    state
        .emit(Some(p.user.id), "notifications.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        crate::managers::operational_health(&state).await?;
        observe(&state).await?;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
pub(crate) async fn observe(state: &AppState) -> anyhow::Result<()> {
    let changed=state.db.call(|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Fixed messages prevent private media titles, URLs and provider errors entering shared notifications.
        let observations=tx.prepare("SELECT 'job:'||id,'error','A background job failed. Open administration for details.' FROM jobs WHERE state='failed' AND completed_at>?1
          UNION ALL SELECT 'service:'||id,'error','An integration is unavailable. Check service health.' FROM manager_services WHERE error IS NOT NULL
          UNION ALL SELECT 'support:'||id,'error','A support service needs attention. Check download and indexer health.' FROM support_services WHERE error IS NOT NULL
          UNION ALL SELECT 'update:'||id||':'||state,'warning','A service update needs attention. Open service updates.' FROM service_updates WHERE state IN ('blocked','failed','incompatible','unable-to-verify','runtime-failure')
          UNION ALL SELECT 'health:'||json_extract(j.value,'$.id'),'warning','An indexer or download service needs attention. Open Support services.' FROM settings s,json_each(s.value,'$.items') j WHERE s.key='operations.support' AND json_extract(j.value,'$.problem')=1")?.query_map([now()-7*86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let admins=tx.prepare("SELECT id FROM users WHERE role='admin'")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed=vec![];
        let previous=tx.prepare("SELECT source,occurrence,active FROM notification_conditions")?.query_map([],|r|Ok((r.get::<_,String>(0)?,(r.get::<_,i64>(1)?,r.get::<_,bool>(2)?))))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?;
        tx.execute("UPDATE notification_conditions SET active=0",[])?;
        for (source,severity,message) in &observations {
            let epoch=previous.get(source).map_or(1,|(n,active)|if *active{*n}else{n+1});
            tx.execute("INSERT INTO notification_conditions VALUES (?1,?2,1) ON CONFLICT(source) DO UPDATE SET occurrence=excluded.occurrence,active=1",params![source,epoch])?;
            let key=format!("{source}:{epoch}");
            for user in &admins {if tx.execute("INSERT OR IGNORE INTO notifications VALUES (?1,?2,?3,?4,?5,?6,NULL)",params![id(),user,key,severity,message,now()])?>0 && !changed.contains(user){changed.push(user.clone());}}
        }
        let personal=tx.prepare("SELECT user_id,'request:'||id||':'||state||':'||updated_at,'Your media request has changed. Open Requests for details.' FROM acquisition_requests WHERE updated_at>?1 AND state IN ('available','denied','failed','requested') UNION ALL SELECT user_id,'account:'||provider||':'||generation,'A linked account needs you to sign in again.' FROM online_accounts WHERE status='reconnect_required'")?.query_map([now()-7*86400],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (user,source,message) in personal {if tx.execute("INSERT OR IGNORE INTO notifications VALUES (?1,?2,?3,'info',?4,?5,NULL)",params![id(),user,source,message,now()])?>0 && !changed.contains(&user){changed.push(user);}}
        tx.execute("DELETE FROM notifications WHERE created_at<?1",[now()-90*86400])?;
        tx.commit()?;Ok(changed)
    }).await?;
    for user in changed {
        state
            .emit(Some(user), "notifications.changed", json!({}))
            .await?;
    }
    Ok(())
}
fn usage(path: &std::path::Path) -> anyhow::Result<Value> {
    let mut pending = vec![path.to_path_buf()];
    let mut bytes = 0u64;
    let mut entries = 0;
    let mut limited = false;
    while let Some(dir) = pending.pop() {
        for item in std::fs::read_dir(dir)? {
            entries += 1;
            if entries > 100000 {
                limited = true;
                break;
            }
            let item = item?;
            let meta = std::fs::symlink_metadata(item.path())?;
            if meta.is_file() {
                bytes += meta.len();
            } else if meta.is_dir() {
                pending.push(item.path());
            }
        }
        if limited {
            break;
        }
    }
    Ok(json!({"bytes":bytes,"partial":limited,"free_bytes":fs2::available_space(path)?}))
}
async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut result=state.db.call(|db|{
        let playback=db.prepare("SELECT p.id,u.username,COALESCE(m.title,'Online playback'),p.mode,p.state,p.position,p.duration FROM playback_sessions p JOIN users u ON u.id=p.user_id LEFT JOIN media m ON m.id=p.media_id WHERE p.state IN ('ready','playing','paused') AND p.updated_at>?1 ORDER BY p.updated_at DESC LIMIT 100")?.query_map([now()-120],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"mode":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"position":r.get::<_,f64>(5)?,"duration":r.get::<_,f64>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let errors=db.prepare("SELECT id,kind,completed_at FROM jobs WHERE state='failed' ORDER BY completed_at DESC LIMIT 50")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"at":r.get::<_,Option<i64>>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let services=db.prepare("SELECT kind,version,checked_at,error IS NULL FROM manager_services UNION ALL SELECT kind,version,checked_at,error IS NULL FROM support_services")?.query_map([],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"version":r.get::<_,String>(1)?,"checked_at":r.get::<_,i64>(2)?,"healthy":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let support=db.query_row("SELECT value FROM settings WHERE key='operations.support'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str::<Value>(&v).ok()).unwrap_or_else(||json!({"items":[]}));
        Ok(json!({"playback":playback,"errors":errors,"services":services,"support":support}))
    }).await?;
    let config = state.config.clone();
    let storage = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        Ok(json!({"state":usage(&config.state)?,"cache":usage(&config.cache)?}))
    })
    .await
    .map_err(anyhow::Error::from)??;
    result["storage"] = storage;
    Ok(Json(result))
}
async fn diagnostics(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let summary=state.db.call(move|db|{
        let schema=db.query_row("SELECT MAX(version) FROM schema_migrations",[],|r|r.get::<_,i64>(0))?;
        let jobs=db.prepare("SELECT state,COUNT(*) FROM jobs GROUP BY state")?.query_map([],|r|Ok(json!({"state":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let check=db.query_row("PRAGMA quick_check",[],|r|r.get::<_,String>(0))?=="ok";
        db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'diagnostics.export','server',?2)",params![p.user.id,now()])?;
        Ok(json!({"format":1,"version":thelxinoe_core::VERSION,"schema":schema,"database_ok":check,"jobs":jobs,"created_at":now()}))
    }).await?;
    // Deliberate allowlist: no settings, identifiers, paths, error strings, user history or Docker specs.
    Ok(Json(summary))
}
async fn devices(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let items=state.db.call(|db|Ok(db.prepare("SELECT s.id,u.username,s.name,s.transport,s.last_seen FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.expires_at>?1 ORDER BY s.last_seen DESC LIMIT 500")?.query_map([now()],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"transport":r.get::<_,String>(3)?,"last_seen":r.get::<_,i64>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":items})))
}
