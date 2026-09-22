#[path = "storage/library.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};

pub async fn roots(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    Ok(Json(
        json!({"items":thelxinoe_catalog::roots(&state.db).await?,"media_mount":state.config.media}),
    ))
}
#[derive(Deserialize)]
pub struct NewRoot {
    name: String,
    kind: String,
    path: String,
}
pub async fn add_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewRoot>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || !matches!(input.kind.as_str(), "movies" | "shows" | "music")
    {
        return Err(ApiError::bad("Enter a library name and valid media type"));
    }
    let path =
        thelxinoe_catalog::approved_path(std::path::Path::new(&input.path), &state.config.media)
            .map_err(|e| ApiError::bad(e.to_string()))?;
    let root_id = id();
    let rid = root_id.clone();
    let path = path.to_string_lossy().to_string();
    let added = storage::add_root(&state.db, input, p, rid, path).await?;
    if !added {
        return Err(ApiError::conflict("Library roots cannot overlap"));
    }
    enqueue_scan(&state, &root_id).await?;
    Ok(Json(json!({"id":root_id})))
}
pub async fn enqueue_scan(state: &AppState, root_id: &str) -> anyhow::Result<String> {
    let root_id = root_id.to_string();
    storage::enqueue_scan(root_id, &state.db).await
}
pub async fn scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    if !thelxinoe_catalog::roots(&state.db)
        .await?
        .iter()
        .any(|r| r.id == root_id)
    {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"job_id":enqueue_scan(&state,&root_id).await?})))
}
#[derive(Deserialize, Default)]
pub struct Browse {
    kind: Option<String>,
    parent: Option<String>,
    q: Option<String>,
    collection: Option<i64>,
}
pub async fn browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Browse>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::Browse).await?;
    let mut items = storage::browse(&state.db, input).await?;
    decorate_cards(&state, &p, &mut items.iter_mut().collect::<Vec<_>>()).await?;
    Ok(Json(json!({"items":items})))
}
/// Resolve cached art from the item, album or show in one bounded query.
pub(crate) async fn decorate_cards(
    state: &AppState,
    p: &thelxinoe_core::Principal,
    items: &mut [&mut Value],
) -> Result<()> {
    let ids = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let ids = json!(ids).to_string();
    let metadata = storage::decorate_cards(ids, &state.db).await?;
    let grant = crate::grants::issue(state, p, "artwork", 300).await?;
    for item in items {
        if let Some((year, art)) = item["id"].as_str().and_then(|id| metadata.get(id)) {
            if item.get("year").is_none() {
                item["year"] = json!(year);
            }
            if let Some(id) = art {
                item["artwork_url"] = json!(format!("/api/v1/catalog/{id}/artwork?grant={grant}"));
            }
        }
    }
    Ok(())
}

pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::Browse).await?;
    let result = storage::detail(&state.db, media_id).await?;
    Ok(Json(result.ok_or_else(ApiError::not_found)?))
}
pub async fn overrides(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let Some(fields) = value.as_object() else {
        return Err(ApiError::bad("Expected metadata fields"));
    };
    if fields.iter().any(|(key, value)| match key.as_str() {
        "title" | "overview" => !value.is_null() && value.as_str().is_none_or(|s| s.len() > 10_000),
        "year" => !value.is_null() && !value.as_i64().is_some_and(|n| (1..=9999).contains(&n)),
        _ => true,
    }) {
        return Err(ApiError::bad("Unsupported metadata override"));
    }
    let updated = storage::overrides(&state.db, media_id, value, p).await?;
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"ok":true})))
}
pub async fn reconcile(state: AppState) -> anyhow::Result<()> {
    use notify::Watcher;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| {
            matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
                    | notify::EventKind::Any
            )
        }) {
            let _ = tx.try_send(());
        }
    })?;
    // Watching the mounted ancestor also catches new library roots without restarting.
    if state.config.media.exists()
        && let Err(error) = watcher.watch(&state.config.media, notify::RecursiveMode::Recursive)
    {
        tracing::warn!(%error,"Filesystem notifications unavailable; periodic reconciliation remains active");
    }
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
    loop {
        tokio::select! {_=interval.tick()=>{},_=rx.recv()=>{tokio::time::sleep(std::time::Duration::from_secs(2)).await;while rx.try_recv().is_ok(){}}}
        for root in thelxinoe_catalog::roots(&state.db).await? {
            enqueue_scan(&state, &root.id).await?;
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use crate::online::oauth::tests::{call, fixture};
    use serde_json::Value;
    #[tokio::test]
    async fn home_inherits_show_artwork_without_exposing_other_users_shelves() {
        let (_temp, state, alice) = fixture().await;
        state.db.write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Shows','shows','/media/shows')",[])?;
            for (id,kind,parent,metadata) in [("show","show",None,"{\"artwork_cached\":true}"),("season","season",Some("show"),"{}"),("episode","episode",Some("season"),"{}")] {
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,metadata,created_at) VALUES (?1,'root',?2,?3,?1,?1,?4,1)",rusqlite::params![id,kind,parent,metadata])?;
            }
            db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite) VALUES ('alice','episode',0,1,1)",[])?;
            Ok(())
        }).await.unwrap();
        let response = call(&state, "/api/v1/me/home", "GET", Value::Null, &alice).await;
        assert_eq!(response.0, axum::http::StatusCode::OK);
        let url = response.2["favorites"][0]["artwork_url"].as_str().unwrap();
        assert!(url.starts_with("/api/v1/catalog/show/artwork?grant="));
        let bob =
            thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        assert_eq!(
            call(
                &state,
                "/api/v1/me/home",
                "GET",
                Value::Null,
                &format!("thelxinoe_session={bob}")
            )
            .await
            .2["favorites"],
            serde_json::json!([])
        );
    }
}
