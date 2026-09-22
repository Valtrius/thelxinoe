#[path = "storage/realtime.rs"]
mod storage;

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
            let records=storage::events( &state.db, since, user, session).await;
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
    let rows = storage::jobs(&state.db).await?;
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
