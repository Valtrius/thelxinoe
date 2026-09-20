use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub(crate) fn card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"title":row.get::<_,String>(2)?,"available":row.get::<_,bool>(3)?}),
    )
}
pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    get_for(&state, &p, &media).await.map(Json)
}
pub(crate) async fn get_for(state: &AppState, p: &Principal, media: &str) -> Result<Value> {
    let p = p.clone();
    let media = media.to_owned();
    let result = state.db.call(move |db| {
        Ok(db.query_row("SELECT COALESCE(s.favorite,0),COALESCE(s.watch_later,0),COALESCE(s.watched,0) FROM media m LEFT JOIN media_state s ON s.media_id=m.id AND s.user_id=?1 WHERE m.id=?2",params![p.user.id,media],|r|Ok(json!({"favorite":r.get::<_,bool>(0)?,"watch_later":r.get::<_,bool>(1)?,"watched":r.get::<_,bool>(2)?}))).optional()?)
    }).await?;
    result.ok_or_else(ApiError::not_found)
}
#[derive(Deserialize)]
pub struct Change {
    pub(crate) favorite: Option<bool>,
    pub(crate) watch_later: Option<bool>,
    pub(crate) watched: Option<bool>,
}
pub async fn set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media): Path<String>,
    Json(input): Json<Change>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    set_for(&state, &p, &media, input).await.map(Json)
}
pub(crate) async fn set_for(
    state: &AppState,
    p: &Principal,
    media: &str,
    input: Change,
) -> Result<Value> {
    let _lease = state.media_operations.read().await;
    let user = p.user.id.clone();
    let mid = media.to_owned();
    let result = state.db.call(move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind:Option<String> = tx.query_row("SELECT kind FROM media WHERE id=?1",[&mid],|r|r.get(0)).optional()?;
        let Some(kind) = kind else { return Ok(404); };
        if input.watch_later.is_some() && !["movie","show","episode"].contains(&kind.as_str()) { return Ok(400); }
        if input.watched.is_some() && !["movie","episode","track"].contains(&kind.as_str()) { return Ok(400); }
        tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite,watch_later) VALUES (?1,?2,COALESCE(?3,0),?4,COALESCE(?5,0),COALESCE(?6,0)) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=COALESCE(?3,watched),favorite=COALESCE(?5,favorite),watch_later=COALESCE(?6,watch_later),updated_at=?4",params![user,mid,input.watched,now(),input.favorite,input.watch_later])?;
        if let Some(watched) = input.watched {
            tx.execute("UPDATE edition_progress SET position=CASE WHEN ?1 THEN duration ELSE 0 END,updated_at=?2 WHERE user_id=?3 AND media_id=?4",params![watched,now(),user,mid])?;
        }
        tx.commit()?;
        Ok(200)
    }).await?;
    match result {
        404 => return Err(ApiError::not_found()),
        400 => {
            return Err(ApiError::bad(
                "This state is not supported for this media type",
            ));
        }
        _ => {}
    }
    state
        .emit(
            Some(p.user.id.clone()),
            "media-state.changed",
            json!({"media_id":media}),
        )
        .await?;
    get_for(state, p, media).await
}
pub async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let mut home = home_for(&state, &p).await?;
    let mut items = home
        .as_object_mut()
        .into_iter()
        .flat_map(|shelves| shelves.values_mut())
        .filter_map(Value::as_array_mut)
        .flat_map(|items| items.iter_mut())
        .collect::<Vec<_>>();
    crate::library::decorate_cards(&state, &p, &mut items).await?;
    Ok(Json(home))
}
pub(crate) async fn home_for(state: &AppState, principal: &Principal) -> Result<Value> {
    let p = principal.clone();
    let result = state.db.call(move |db| {
        let favorites = db.prepare("SELECT c.id,c.kind,c.title,c.available FROM media_cards c JOIN media_state s ON s.media_id=c.id WHERE s.user_id=?1 AND s.favorite=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let later = db.prepare("SELECT c.id,c.kind,c.title,c.available FROM media_cards c JOIN media_state s ON s.media_id=c.id WHERE s.user_id=?1 AND s.watch_later=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let resume = db.prepare("WITH latest AS (SELECT e.*,ROW_NUMBER() OVER(PARTITION BY media_id ORDER BY updated_at DESC,edition) AS rn FROM edition_progress e WHERE user_id=?1) SELECT c.id,c.kind,c.title,c.available,e.position,e.duration,e.edition,(SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1 ORDER BY f.id LIMIT 1) FROM latest e JOIN media_cards c ON c.id=e.media_id WHERE e.rn=1 AND EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1) AND c.kind IN ('movie','episode') AND c.available=1 AND e.position>0 AND e.position<e.duration*0.9 ORDER BY e.updated_at DESC LIMIT 50")?.query_map([&p.user.id],|r|{let mut c=card(r)?;c["position"]=json!(r.get::<_,f64>(4)?);c["duration"]=json!(r.get::<_,f64>(5)?);c["edition"]=json!(r.get::<_,String>(6)?);c["fileId"]=json!(r.get::<_,String>(7)?);Ok(c)})?.collect::<std::result::Result<Vec<_>,_>>()?;
        let next = db.prepare("WITH ordered AS (SELECT e.id,s.parent_id AS show_id,ROW_NUMBER() OVER(PARTITION BY s.parent_id ORDER BY s.sort_number,e.sort_number,e.id) AS ord FROM media e JOIN media s ON s.id=e.parent_id WHERE e.kind='episode' AND s.kind='season' AND s.sort_number>0), anchor AS (SELECT o.show_id,MAX(o.ord) AS ord FROM ordered o JOIN media_state u ON u.media_id=o.id WHERE u.user_id=?1 AND u.watched=1 GROUP BY o.show_id), candidates AS (SELECT o.id,o.show_id,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY o.ord) AS rn FROM ordered o LEFT JOIN anchor a ON a.show_id=o.show_id LEFT JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 LEFT JOIN media_state show_state ON show_state.media_id=o.show_id AND show_state.user_id=?1 JOIN media_cards c ON c.id=o.id WHERE c.available=1 AND COALESCE(u.watched,0)=0 AND o.ord>COALESCE(a.ord,0) AND (a.ord IS NOT NULL OR COALESCE(show_state.favorite,0)=1 OR COALESCE(show_state.watch_later,0)=1)) SELECT c.id,c.kind,c.title,c.available,sh.title FROM candidates n JOIN media_cards c ON c.id=n.id JOIN media_cards sh ON sh.id=n.show_id WHERE n.rn=1 ORDER BY sh.title LIMIT 50")?.query_map([&p.user.id],|r|{let mut c=card(r)?;c["show_title"]=json!(r.get::<_,String>(4)?);Ok(c)})?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"favorites":favorites,"watch_later":later,"continue_watching":resume,"next_up":next}))
    }).await?;
    Ok(result)
}
#[derive(Clone, Deserialize, Serialize)]
pub struct QueueContext {
    pub client_id: String,
    pub revision: i64,
    pub index: usize,
}
pub(crate) fn valid_client(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()
}
pub(crate) fn validate_tracks(db: &rusqlite::Connection, items: &[String]) -> anyhow::Result<bool> {
    if items.len() > 500 {
        return Ok(false);
    }
    let mut query =
        db.prepare("SELECT EXISTS(SELECT 1 FROM media WHERE id=?1 AND kind='track')")?;
    for item in items {
        if !query.query_row([item], |r| r.get::<_, bool>(0))? {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) fn queue_value(
    db: &rusqlite::Connection,
    p: &Principal,
    client: &str,
) -> anyhow::Result<Value> {
    Ok(db.query_row("SELECT revision,items,current_index,position,completed FROM music_queues WHERE user_id=?1 AND client_id=?2",params![p.user.id,client],|r|Ok(json!({"revision":r.get::<_,i64>(0)?,"items":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default(),"current_index":r.get::<_,i64>(2)?,"position":r.get::<_,f64>(3)?,"completed":r.get::<_,bool>(4)?}))).optional()?.unwrap_or_else(||json!({"revision":0,"items":[],"current_index":0,"position":0})))
}
pub async fn queue_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if !valid_client(&client) {
        return Err(ApiError::bad("Invalid client identifier"));
    }
    Ok(Json(state.db.call(move |db| {
        let mut value=queue_value(db,&p,&client)?;
        let tracks=db.prepare("SELECT c.id,c.kind,c.title,c.available FROM json_each(?1) j JOIN media_cards c ON c.id=j.value ORDER BY CAST(j.key AS INTEGER)")?.query_map([value["items"].to_string()],card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        value["tracks"]=json!(tracks);Ok(value)
    }).await?))
}
#[derive(Deserialize)]
pub struct SaveQueue {
    revision: i64,
    items: Vec<String>,
    current_index: usize,
}
pub async fn queue_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client): Path<String>,
    Json(input): Json<SaveQueue>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if !valid_client(&client)
        || input.items.len() > 500
        || input.current_index >= input.items.len().max(1)
    {
        return Err(ApiError::bad("Invalid music queue"));
    }
    let principal = p.clone();
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous=queue_value(&tx,&principal,&client)?;
        if previous["revision"].as_i64()!=Some(input.revision) {return Ok((409,Value::Null));}
        if !validate_tracks(&tx,&input.items)? {return Ok((400,Value::Null));}
        let count:i64=tx.query_row("SELECT COUNT(*) FROM music_queues WHERE user_id=?1",[&principal.user.id],|r|r.get(0))?;
        if input.revision==0 && count>=100 {return Ok((400,Value::Null));}
        tx.execute("INSERT INTO music_queues(user_id,client_id,revision,items,current_index,position,updated_at) VALUES (?1,?2,?3,?4,?5,0,?6) ON CONFLICT(user_id,client_id) DO UPDATE SET revision=excluded.revision,items=excluded.items,current_index=excluded.current_index,position=0,completed=0,updated_at=excluded.updated_at",params![principal.user.id,client,input.revision+1,serde_json::to_string(&input.items)?,input.current_index as i64,now()])?;
        let value=queue_value(&tx,&principal,&client)?;
        tx.commit()?;Ok((200,value))
    }).await?;
    match result.0 {
        409 => Err(ApiError::conflict(
            "This queue changed in another window; reload it before editing",
        )),
        400 => Err(ApiError::bad(
            "Choose up to 500 catalog tracks and at most 100 client queues",
        )),
        _ => Ok(Json(result.1)),
    }
}
#[derive(Deserialize)]
pub struct Preferences {
    timezone: String,
}
pub async fn preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Preferences>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    input
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::bad("Unknown timezone"))?;
    let zone = input.timezone.clone();
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE users SET timezone=?1 WHERE id=?2",
                params![input.timezone, p.user.id],
            )?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({"timezone":zone})))
}
