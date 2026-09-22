//! Private named lists. Membership determines aggregate retention.
use super::{browse, downloads, feed};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, now};

pub(crate) fn default_list(db: &Connection, user: &str) -> anyhow::Result<i64> {
    db.execute("INSERT INTO youtube_watchlists(user_id,name,is_default,created_at,updated_at) SELECT ?1,'Watch Later',1,?2,?2 WHERE NOT EXISTS(SELECT 1 FROM youtube_watchlists WHERE user_id=?1 AND is_default=1)",params![user,now()])?;
    Ok(db.query_row(
        "SELECT id FROM youtube_watchlists WHERE user_id=?1 AND is_default=1",
        [user],
        |r| r.get(0),
    )?)
}
async fn authorize(state: &AppState, user: &str, id: i64) -> Result<()> {
    let user = user.to_owned();
    if !state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM youtube_watchlists WHERE user_id=?1 AND id=?2)",
                params![user, id],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?
    {
        return Err(ApiError::not_found());
    }
    Ok(())
}
async fn changed(state: &AppState, user: String) -> Result<Json<Value>> {
    state.emit(Some(user), "youtube.changed", json!({})).await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    let lists=state.db.call(move|db|{
        default_list(db,&p.user.id)?;
        let mut lists=db.prepare("SELECT id,name,is_default,auto_download,auto_remove_watched,sort_mode,sort_direction,created_at,updated_at FROM youtube_watchlists WHERE user_id=?1 ORDER BY is_default DESC,id")?.query_map([&p.user.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"isDefault":r.get::<_,bool>(2)?,"autoDownload":r.get::<_,bool>(3)?,"autoRemoveWatched":r.get::<_,bool>(4)?,"sortMode":r.get::<_,String>(5)?,"sortDirection":r.get::<_,String>(6)?,"createdAt":browse::date(r.get(7)?),"updatedAt":browse::date(r.get(8)?)})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for list in &mut lists {
            let entries=db.prepare("SELECT video_id,added_at,manual_position FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2 ORDER BY manual_position,video_id")?.query_map(params![p.user.id,list["id"].as_i64()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            list["items"]=json!(entries.into_iter().map(|(id,added,position)|{let video=browse::one(db,&p.user.id,&id,&grant)?;Ok(json!({"metadataPending":video["metadataPending"],"video":video,"addedAt":browse::date(added),"manualPosition":position}))}).collect::<anyhow::Result<Vec<Value>>>()?);
        }
        Ok(lists)
    }).await?;
    Ok(Json(json!(lists)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub name: String,
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
}
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Create>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    create_for(&state, &p, input).await
}
pub(crate) async fn create_for(
    state: &AppState,
    p: &Principal,
    input: Create,
) -> Result<Json<Value>> {
    if !name_valid(&input.name) {
        return Err(ApiError::bad("Enter a watchlist name of 1–80 characters"));
    }
    let user = p.user.id.clone();
    let id=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        default_list(&tx,&user)?;
        if tx.query_row("SELECT COUNT(*) FROM youtube_watchlists WHERE user_id=?1",[&user],|r|r.get::<_,i64>(0))?>=50{return Ok(None);}
        tx.execute("INSERT INTO youtube_watchlists(user_id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)",params![user,input.name.trim(),now()])?;
        let id=tx.last_insert_rowid();tx.commit()?;Ok(Some(id))
    }).await?.ok_or_else(||ApiError::conflict("You can create up to 50 watchlists"))?;
    let _ = changed(state, p.user.id.clone()).await?;
    Ok(Json(json!(id)))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    name: String,
    auto_download: bool,
    auto_remove_watched: bool,
    sort_mode: String,
    sort_direction: String,
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Update>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    if !name_valid(&input.name)
        || !["manual", "date"].contains(&input.sort_mode.as_str())
        || !["asc", "desc"].contains(&input.sort_direction.as_str())
    {
        return Err(ApiError::bad("Invalid watchlist settings"));
    }
    if input.auto_download && !downloads::enabled(&state).await? {
        return Err(ApiError::conflict(
            "YouTube downloads are disabled by the administrator",
        ));
    }
    let user = p.user.id.clone();
    state.db.call(move|db|{db.execute("UPDATE youtube_watchlists SET name=?1,auto_download=?2,auto_remove_watched=?3,sort_mode=?4,sort_direction=?5,updated_at=?6 WHERE user_id=?7 AND id=?8",params![input.name.trim(),input.auto_download,input.auto_remove_watched,input.sort_mode,input.sort_direction,now(),user,id])?;Ok(())}).await?;
    changed(&state, p.user.id).await
}
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    let user = p.user.id.clone();
    let removed = state
        .db
        .call(move |db| {
            Ok(db.execute(
                "DELETE FROM youtube_watchlists WHERE user_id=?1 AND id=?2 AND is_default=0",
                params![user, id],
            )? > 0)
        })
        .await?;
    if !removed {
        return Err(ApiError::conflict("Watch Later cannot be deleted"));
    }
    changed(&state, p.user.id).await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Add {
    pub video_id: String,
    pub manual_position: f64,
}
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Add>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    add_for(&state, &p, id, input).await
}
pub(crate) async fn add_for(
    state: &AppState,
    p: &Principal,
    id: i64,
    input: Add,
) -> Result<Json<Value>> {
    authorize(state, &p.user.id, id).await?;
    let video = feed::video_id(&input.video_id)
        .ok_or_else(|| ApiError::bad("Enter a YouTube video URL or ID"))?;
    if !input.manual_position.is_finite() || input.manual_position.abs() > 1e15 {
        return Err(ApiError::bad("Invalid watchlist position"));
    }
    let grant = grants::issue(state, p, "youtube-artwork", 300).await?;
    let user = p.user.id.clone();
    let result = video.clone();
    let added=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Check again within the write transaction in case the list was deleted concurrently.
        let exists=tx.query_row("SELECT id FROM youtube_watchlists WHERE user_id=?1 AND id=?2",params![user,id],|r|r.get::<_,i64>(0)).optional()?;
        if exists.is_none(){return Ok(None);}
        let retained=tx.query_row("SELECT COUNT(*) FROM youtube_video_state WHERE user_id=?1 AND video_id<>?2 AND (watchlist=1 OR pinned=1)",params![user,video],|r|r.get::<_,i64>(0))?;
        if retained>=1000{return Ok(None);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,?2,?2) ON CONFLICT DO NOTHING",params![user,video])?;
        tx.execute("INSERT INTO youtube_state(user_id,video_id,updated_at) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",params![user,video,now()])?;
        let added=tx.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![user,id,video,input.manual_position,now()])?>0;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),user])?;
        let output=browse::one(&tx,&user,&video,&grant)?;tx.commit()?;Ok(Some((added,output)))
    }).await?.ok_or_else(||ApiError::conflict("The list no longer exists or your 1,000 saved video limit was reached"))?;
    let auto_download = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT auto_download FROM youtube_watchlists WHERE id=?1",
                    [id],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .await?;
    let download_error = if auto_download {
        downloads::request_for(state,p,result).await.err().map(|_|json!({"category":"api","code":"download_pending","message":"Saved. Automatic download will retry when server tools and video metadata are ready.","technical":""}))
    } else {
        None
    };
    let _ = changed(state, p.user.id.clone()).await?;
    Ok(Json(
        json!({"added":added.0,"video":added.1,"autoDownloadError":download_error}),
    ))
}
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, video)): Path<(i64, String)>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    remove_for(&state, &p, id, video).await
}
pub(crate) async fn remove_for(
    state: &AppState,
    p: &Principal,
    id: i64,
    video: String,
) -> Result<Json<Value>> {
    authorize(state, &p.user.id, id).await?;
    let user = p.user.id.clone();
    state.db.call(move|db|{db.execute("DELETE FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2 AND video_id=?3",params![user,id,video])?;Ok(())}).await?;
    changed(state, p.user.id.clone()).await
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reorder {
    video_ids: Vec<String>,
}
pub async fn reorder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<Reorder>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    authorize(&state, &p.user.id, id).await?;
    if input.video_ids.len() > 1000
        || input
            .video_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.video_ids.len()
    {
        return Err(ApiError::bad("Invalid watchlist order"));
    }
    let user = p.user.id.clone();
    state.db.call(move|db|{let tx=db.transaction()?;for (index,video) in input.video_ids.iter().enumerate(){tx.execute("UPDATE youtube_watchlist_items SET manual_position=?1 WHERE user_id=?2 AND watchlist_id=?3 AND video_id=?4",params![index as i64,user,id,video])?;}tx.commit()?;Ok(())}).await?;
    changed(&state, p.user.id).await
}
