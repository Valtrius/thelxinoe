//! Shared file-generation-specific segments for first-party and Jellyfin clients.
mod analysis;
mod fingerprint;
#[cfg(test)]
mod tests;
use crate::{
    AppState,
    error::{ApiError, Result},
    playback, security,
};
pub use analysis::run;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Capability, Principal, id, now};
const KINDS: [&str; 4] = ["Intro", "Recap", "Credits", "Preview"];

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/catalog/{id}/segments", get(list).put(save))
        .route("/api/v1/catalog/{id}/segments/analyze", post(reanalyze))
        .route(
            "/api/v1/me/segments",
            get(preferences).put(save_preferences),
        )
        .route("/api/v1/admin/segments", get(status).put(configure))
}
#[derive(Serialize, Deserialize, Clone)]
pub(super) struct Segment {
    #[serde(default)]
    pub id: String,
    pub kind: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub confidence: f64,
}
#[derive(Deserialize, Default)]
struct Selection {
    file_id: Option<String>,
}
pub(crate) async fn resolved(
    state: &AppState,
    media: &str,
    file: Option<&str>,
) -> Result<(String, String, Vec<Segment>)> {
    let src = playback::source(state, media, file).await?;
    let media = media.to_owned();
    let file = src.id.clone();
    let generation = src.generation.clone();
    let items=state.db.call(move|db|{
        let manual=db.query_row("SELECT EXISTS(SELECT 1 FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3)",params![media,file,generation],|r|r.get::<_,bool>(0))?;
        let rows=db.prepare("SELECT id,kind,start,end,source,confidence FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 ORDER BY CASE source WHEN 'manual' THEN 0 WHEN 'theintrodb' THEN 1 ELSE 2 END,start")?.query_map(params![media,file,generation],|r|Ok(Segment{id:r.get(0)?,kind:r.get(1)?,start:r.get(2)?,end:r.get(3)?,source:r.get(4)?,confidence:r.get(5)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut selected=Vec::<Segment>::new();
        for s in rows {
            if manual && s.source!="manual" {continue;}
            if selected.iter().any(|other|other.source!=s.source && other.kind==s.kind){continue;}
            selected.push(s);
        }
        selected.sort_by(|a,b|a.start.total_cmp(&b.start));
        Ok(selected)
    }).await?;
    Ok((src.id, src.generation, items))
}
async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Query(selection): Query<Selection>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let (file, generation, items) = resolved(&state, &media, selection.file_id.as_deref()).await?;
    Ok(Json(
        json!({"file_id":file,"generation":generation,"items":items,"preferences":preferences_for(&state,&p).await?}),
    ))
}
pub(crate) async fn for_jellyfin(
    state: &AppState,
    p: &Principal,
    media: &str,
) -> Result<Vec<Segment>> {
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let mid = media.to_owned();
    let (file,count)=state.db.call(move|db|{
        let file=db.query_row("SELECT s.file_id FROM playback_sessions s JOIN media_files f ON f.id=s.file_id AND f.generation=s.generation AND f.present=1 WHERE s.media_id=?1 AND s.user_id=?2 AND s.auth_session_id=?3 AND s.state IN ('ready','playing','paused') ORDER BY s.created_at DESC,s.rowid DESC LIMIT 1",params![mid,user,auth],|r|r.get::<_,String>(0)).optional()?;
        let count=db.query_row("SELECT COUNT(*) FROM media_sources s JOIN media_files f ON f.id=s.file_id AND f.present=1 WHERE s.media_id=?1",[mid],|r|r.get::<_,i64>(0))?;
        Ok((file,count))
    }).await?;
    // The standard endpoint has no edition parameter. Never guess between editions.
    if file.is_none() && count > 1 {
        return Ok(vec![]);
    }
    Ok(resolved(state, media, file.as_deref()).await?.2)
}
pub(crate) async fn preferences_for(state: &AppState, p: &Principal) -> Result<Value> {
    let user = p.user.id.clone();
    Ok(state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT value FROM segment_preferences WHERE user_id=?1",
                    [user],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({"Intro":"Ask","Recap":"Ask","Credits":"Ask","Preview":"Ask"})))
}
async fn preferences(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(preferences_for(&state, &p).await?))
}
async fn save_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if input.as_object().is_none_or(|o| o.len() != 4)
        || KINDS
            .iter()
            .any(|k| !matches!(input[*k].as_str(), Some("Auto" | "Ask" | "Ignore")))
    {
        return Err(ApiError::bad(
            "Choose Auto, Ask or Ignore for every segment type",
        ));
    }
    state.db.call(move|db|{db.execute("INSERT INTO segment_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![p.user.id,input.to_string()])?;Ok(())}).await?;
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Edit {
    file_id: String,
    generation: String,
    items: Vec<Segment>,
    #[serde(default)]
    reset: bool,
}
async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let _lease = state.media_operations.write().await;
    let src = playback::source(&state, &media, Some(&input.file_id)).await?;
    if src.generation != input.generation {
        return Err(ApiError::conflict(
            "Media was replaced; reload its segments",
        ));
    }
    if input.items.len() > 40 || input.items.iter().any(|s| !valid(s, src.duration())) {
        return Err(ApiError::bad(
            "Segments need a supported type and a valid start/end within this file",
        ));
    }
    let event_media = media.clone();
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 AND source='manual'",params![media,src.id,src.generation])?;
        tx.execute("DELETE FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3",params![media,src.id,src.generation])?;
        if !input.reset {
            tx.execute("INSERT INTO segment_overrides VALUES (?1,?2,?3)",params![media,src.id,src.generation])?;
            for s in input.items {insert(&tx,&src,&s,"manual")?;}
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'segments.edit',?2,?3)",params![p.user.id,media,now()])?;
        tx.commit()?;Ok(())
    }).await?;
    state
        .emit(None, "segments.changed", json!({"media_id":event_media}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
fn valid(s: &Segment, duration: f64) -> bool {
    KINDS.contains(&s.kind.as_str())
        && s.start.is_finite()
        && s.end.is_finite()
        && s.start >= 0.0
        && s.end > s.start
        && s.end <= duration
}
fn insert(
    db: &rusqlite::Connection,
    src: &thelxinoe_playback::Source,
    s: &Segment,
    origin: &str,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO media_segments VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id(),
            src.media_id,
            src.id,
            src.generation,
            s.kind,
            s.start,
            s.end,
            origin,
            if origin == "manual" {
                1.0
            } else {
                s.confidence
            },
            now()
        ],
    )?;
    Ok(())
}
async fn reanalyze(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Query(selection): Query<Selection>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    let src = playback::source(&state, &media, selection.file_id.as_deref()).await?;
    let mid = media.clone();
    let episode = state
        .db
        .call(move |db| {
            Ok(
                db.query_row("SELECT kind='episode' FROM media WHERE id=?1", [mid], |r| {
                    r.get::<_, bool>(0)
                })?,
            )
        })
        .await?;
    if !episode {
        return Err(ApiError::bad(
            "Automatic analysis is available for episodes",
        ));
    }
    state.db.call(move|db|{db.execute("INSERT INTO segment_analysis(media_id,file_id,generation,state,requested_at) VALUES (?1,?2,?3,'queued',?4) ON CONFLICT(media_id,file_id,generation) DO UPDATE SET state=CASE WHEN state='running' THEN state ELSE 'queued' END,requested_at=excluded.requested_at,error=NULL",params![media,src.id,src.generation,now()])?;Ok(())}).await?;
    Ok(Json(json!({"queued":true})))
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
    local: bool,
    external: bool,
}
async fn config(state: &AppState) -> Result<Config> {
    Ok(state
        .db
        .call(|db| {
            Ok(serde_json::from_str(&db.query_row(
                "SELECT value FROM settings WHERE key='segments.config'",
                [],
                |r| r.get::<_, String>(0),
            )?)?)
        })
        .await?)
}
async fn configure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Config>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE settings SET value=?1 WHERE key='segments.config'",
                [serde_json::to_string(&input)?],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"saved":true})))
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let items=state.db.call(|db|Ok(db.prepare("SELECT a.media_id,m.title,a.state,a.error,a.completed_at FROM segment_analysis a JOIN media m ON m.id=a.media_id ORDER BY a.requested_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"media_id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"error":r.get::<_,Option<String>>(3)?,"completed_at":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"config":config(&state).await?,"items":items})))
}
