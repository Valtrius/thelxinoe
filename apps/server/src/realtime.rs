use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{
        Query, State,
        ws::{Message, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Capability, now};

#[derive(Deserialize)]
pub struct Cursor {
    #[serde(default)]
    since: i64,
    pub(crate) ticket: Option<String>,
}
pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let p = if let Some(ticket) = &cursor.ticket {
        crate::grants::resolve(&state, ticket, "events", true)
            .await?
            .ok_or_else(ApiError::unauthorized)?
    } else {
        security::principal(&state, &headers).await?
    };
    Ok(ws.on_upgrade(move |mut socket|async move {
        let mut subscription=state.events.subscribe();let mut since=cursor.since.max(0);let mut timer=tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            let user=p.user.id.clone();let session=p.session_id.clone();
            let records=state.db.call(move |db|{
                let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND expires_at>?2)",rusqlite::params![session,now()],|r|r.get(0))?;
                if !active{return Ok(None);}
                let mut query=db.prepare("SELECT id,kind,payload FROM events WHERE id>?1 AND (user_id IS NULL OR user_id=?2) ORDER BY id LIMIT 100")?;
                Ok(Some(query.query_map(rusqlite::params![since,user],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"payload":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(Value::Null)})))?.collect::<std::result::Result<Vec<_>,_>>()?))
            }).await;
            let Ok(Some(records))=records else{let _=socket.send(Message::Close(None)).await;return;};
            let full=records.len()==100;
            for record in records {since=record["id"].as_i64().unwrap_or(since);if socket.send(Message::Text(record.to_string().into())).await.is_err(){return;}}
            if full {continue;}
            tokio::select! {
                _=subscription.recv()=>{},
                _=timer.tick()=>{if socket.send(Message::Ping(Vec::new().into())).await.is_err(){return;}},
                message=socket.recv()=>{if matches!(message,None|Some(Err(_))|Some(Ok(Message::Close(_)))) {return;}}
            }
        }
    }))
}
pub async fn jobs(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows=state.db.call(|db|Ok(db.prepare("SELECT id,kind,state,attempts,error,created_at FROM jobs ORDER BY created_at DESC LIMIT 100")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"attempts":r.get::<_,i64>(3)?,"error":r.get::<_,Option<String>>(4)?,"created_at":r.get::<_,i64>(5)?})))?.collect::<std::result::Result<Vec<_>,_>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
pub struct Enqueue {
    key: String,
}
pub async fn enqueue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Enqueue>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    if input.key.len() > 100 || input.key.is_empty() {
        return Err(ApiError::bad("Invalid job key"));
    }
    let id = thelxinoe_jobs::Queue(state.db.clone())
        .enqueue(
            "checkpoint".into(),
            json!({"message":"Server checkpoint"}),
            format!("checkpoint:{}", input.key),
        )
        .await?;
    Ok(Json(json!({"id":id})))
}
