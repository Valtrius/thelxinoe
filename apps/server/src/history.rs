#[path = "storage/history.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security, statistics,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Capability, now};

pub(crate) use storage::delete_provider;
pub(crate) use storage::finish_stale;
pub(crate) use storage::record;
#[derive(Deserialize, Default)]
pub struct Filter {
    before: Option<String>,
    #[serde(flatten)]
    scope: statistics::Filter,
}
#[derive(Deserialize, Default)]
pub struct AuditFilter {
    before: Option<i64>,
}
pub async fn mine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    history(&state, Some(p.user.id), &p.user.timezone, filter)
        .await
        .map(Json)
}
pub async fn admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::InspectHistory).await?;
    history(&state, filter.scope.user.clone(), &p.user.timezone, filter)
        .await
        .map(Json)
}
async fn history(
    state: &AppState,
    user: Option<String>,
    zone: &str,
    filter: Filter,
) -> Result<Value> {
    let range = filter.scope.range.unwrap_or_else(|| "30d".into());
    let platform = filter.scope.platform.unwrap_or_else(|| "all".into());
    statistics::validate_scope(&range, &platform)?;
    let zone = zone
        .parse()
        .map_err(|_| ApiError::bad("Unknown display timezone"))?;
    let now = chrono::Utc::now();
    let until = now.timestamp();
    let cutoff = statistics::range_cutoff(zone, &range, now);
    let before = filter
        .before
        .as_deref()
        .map(serde_json::from_str::<(i64, i64)>)
        .transpose()
        .map_err(|_| ApiError::bad("Invalid history cursor"))?;
    Ok(storage::history(&state.db, user, platform, cutoff, until, before).await?)
}
pub async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<AuditFilter>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let items = storage::audit(&state.db, filter).await?;
    Ok(Json(
        json!({"next_before":items.last().map(|i|i["id"].clone()).filter(|_|items.len()==100),"items":items}),
    ))
}

#[cfg(test)]
mod tests {
    use crate::online::oauth::tests::{call, fixture};
    use serde_json::Value;
    #[tokio::test]
    async fn history_uses_shared_platform_range_and_private_scope() {
        let (_temp, state, alice) = fixture().await;
        let now = thelxinoe_core::now();
        state.db.write("test.fixture", move |db| {
            for (id,user,platform,media,time) in [
                ("recent","alice","twitch","twitch:recent",now-60),
                ("old","alice","twitch","twitch:old",now-8*86_400),
                ("other-user","bob","twitch","twitch:other",now-60),
                ("kick","alice","kick","kick:channel",now-60),
            ] {
                db.execute("INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES (?1,?2,?3,?4,'Stream','live','live','Browser',?5,?5,?5,20,0,20,'stopped')",rusqlite::params![id,user,platform,media,time])?;
            }
            Ok(())
        }).await.unwrap();
        let response = call(
            &state,
            "/api/v1/me/history?platform=twitch&range=7d&user=bob",
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(response.0, axum::http::StatusCode::OK);
        assert_eq!(response.2["items"].as_array().unwrap().len(), 1);
        assert_eq!(response.2["items"][0]["media_id"], "twitch:recent");
        assert!(response.2.get("stats").is_none());
        assert!(response.2.get("users").is_none());
        assert!(response.2.get("daily").is_none());
        assert!(response.2.get("top").is_none());
    }
    #[tokio::test]
    async fn combined_history_merges_sources_and_uses_a_stable_cursor() {
        let (_temp, state, alice) = fixture().await;
        state
            .db
            .write("test.fixture", |db| {
                for i in 0..99 {
                    db.execute(
                        "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES (?1,'alice','youtube',?2,?3,'upload','public','Browser',?4,?4,?4,30,60,20,'stopped')",
                        rusqlite::params![
                            format!("youtube-{i:03}"),
                            format!("youtube:video-{i:03}"),
                            format!("Video {i:03}"),
                            2_000 + i
                        ],
                    )?;
                }
                db.execute(
                    "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES ('shared','alice','youtube','youtube:shared-video','Shared video','upload','public','Browser',1000,1000,1000,30,60,20,'stopped')",
                    [],
                )?;
                db.execute(
                    "INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,started_at,updated_at,ended_at,position,duration,played_seconds,state) VALUES ('shared','alice','twitch','twitch:shared','Shared stream','live','live','Browser',1000,1000,1000,20,0,20,'stopped')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let first = call(
            &state,
            "/api/v1/me/history?platform=all&range=all",
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(first.0, axum::http::StatusCode::OK);
        assert_eq!(first.2["items"].as_array().unwrap().len(), 100);
        assert_eq!(first.2["items"][0]["kind"], "youtube");
        let before = url::form_urlencoded::byte_serialize(
            first.2["next_before"].as_str().unwrap().as_bytes(),
        )
        .collect::<String>();
        let second = call(
            &state,
            &format!("/api/v1/me/history?platform=all&range=all&before={before}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(second.0, axum::http::StatusCode::OK);
        assert_eq!(second.2["items"].as_array().unwrap().len(), 1);
        assert_eq!(second.2["items"][0]["media_id"], "youtube:shared-video");
        assert!(second.2["next_before"].is_null());
    }
}
