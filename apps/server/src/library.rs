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
    let added=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing:Vec<String>=tx.prepare("SELECT path FROM library_roots")?.query_map([],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        if existing.iter().any(|other|std::path::Path::new(&path).starts_with(other)||std::path::Path::new(other).starts_with(&path)){return Ok(false);}
        tx.execute("INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",params![rid,input.name,input.kind,path])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'library.add',?2,?3)",params![p.user.id,rid,now()])?;tx.commit()?;Ok(true)
    }).await?;
    if !added {
        return Err(ApiError::conflict("Library roots cannot overlap"));
    }
    enqueue_scan(&state, &root_id).await?;
    Ok(Json(json!({"id":root_id})))
}
pub async fn enqueue_scan(state: &AppState, root_id: &str) -> anyhow::Result<String> {
    let root_id = root_id.to_string();
    state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(queued)=tx.query_row("SELECT id FROM jobs WHERE kind='library.scan' AND state='queued' AND json_extract(payload,'$.root_id')=?1",[&root_id],|r|r.get::<_,String>(0)).optional()?{return Ok(queued);}
        let job_id=id();tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'library.scan',?2,?1,'queued',?3,?3)",params![job_id,json!({"root_id":root_id}).to_string(),now()])?;tx.commit()?;Ok(job_id)
    }).await
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
    let mut items=state.db.call(move|db|{
        let mut query=db.prepare("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE (?1 IS NULL OR m.kind=?1) AND (?2 IS NULL OR m.parent_id=?2) AND (?3 IS NULL OR COALESCE(json_extract(m.overrides,'$.title'),json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) LIKE '%'||?3||'%') AND (?4 IS NULL OR json_extract(m.metadata,'$.belongs_to_collection.id')=?4) ORDER BY m.sort_number,m.title LIMIT 500")?;
        Ok(query.query_map(params![input.kind,input.parent,input.q,input.collection],media_row)?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await?;
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
    let metadata = state.db.call(move |db| {
        let mut query = db.prepare("SELECT m.id,m.year,CASE WHEN json_extract(m.metadata,'$.artwork_cached')=1 THEN m.id WHEN json_extract(p.metadata,'$.artwork_cached')=1 THEN p.id WHEN json_extract(g.metadata,'$.artwork_cached')=1 THEN g.id END FROM media m LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media g ON g.id=p.parent_id WHERE m.id IN (SELECT value FROM json_each(?1))")?;
        Ok(query.query_map([ids], |r| Ok((r.get::<_,String>(0)?, (r.get::<_,Option<i64>>(1)?,r.get::<_,Option<String>>(2)?))))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?)
    }).await?;
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
fn media_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let metadata =
        serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or_else(|_| json!({}));
    let overrides =
        serde_json::from_str::<Value>(&r.get::<_, String>(7)?).unwrap_or_else(|_| json!({}));
    let title = overrides
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .get("title")
                .or_else(|| metadata.get("name"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .unwrap_or(r.get::<_, String>(2)?);
    let year = overrides
        .get("year")
        .and_then(Value::as_i64)
        .or_else(|| {
            metadata
                .get("release_date")
                .or_else(|| metadata.get("first_air_date"))
                .and_then(Value::as_str)
                .and_then(|s| s.get(..4))
                .and_then(|s| s.parse().ok())
        })
        .or(r.get::<_, Option<i64>>(4)?);
    let overview = overrides
        .get("overview")
        .or_else(|| metadata.get("overview"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(
        json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"title":title,"overview":overview,"parent_id":r.get::<_,Option<String>>(3)?,"year":year,"sort_number":r.get::<_,Option<i64>>(5)?,"metadata":metadata,"overrides":overrides,"available":r.get::<_,bool>(8)?}),
    )
}
pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::Browse).await?;
    let result=state.db.call(move|db|{
        let item=db.query_row("SELECT m.id,m.kind,m.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) FROM media m WHERE m.id=?1",[&media_id],media_row).optional()?;
        let files=db.prepare("SELECT f.id,f.edition,f.probe,f.present FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1")?.query_map([&media_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"edition":r.get::<_,String>(1)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or_default(),"present":r.get::<_,bool>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let trailers=db.prepare("SELECT f.id,f.probe FROM local_trailers t JOIN media_files f ON f.id=t.file_id WHERE t.media_id=?1 AND f.present=1")?.query_map([&media_id],|r|Ok(json!({"file_id":r.get::<_,String>(0)?,"probe":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default()})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(item.map(|mut item|{item["files"]=json!(files);item["local_trailers"]=json!(trailers);item}))
    }).await?;
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
        "title" | "overview" => {
            !value.is_null() && !value.as_str().is_some_and(|s| s.len() <= 10_000)
        }
        "year" => !value.is_null() && !value.as_i64().is_some_and(|n| (1..=9999).contains(&n)),
        _ => true,
    }) {
        return Err(ApiError::bad("Unsupported metadata override"));
    }
    let updated=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let n=tx.execute("UPDATE media SET overrides=?1 WHERE id=?2",params![value.to_string(),media_id])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.override',?2,?3)",params![p.user.id,media_id,now()])?;tx.commit()?;Ok(n)}).await?;
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
        state.db.call(|db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Shows','shows','/data/shows')",[])?;
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
