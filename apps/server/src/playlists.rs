use crate::{
    AppState,
    error::{ApiError, Result},
    security,
    user_media::{card, validate_tracks},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, id, now};

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let items=state.db.call(move|db|{
        Ok(db.prepare("SELECT p.id,p.name,p.description,p.owner_id,u.username,p.revision,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.user_id=?1 AND f.playlist_id=p.id),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p JOIN users u ON u.id=p.owner_id ORDER BY p.updated_at DESC,p.id LIMIT 500")?.query_map([p.user.id],playlist_row)?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await?;
    Ok(Json(json!({"items":items})))
}
fn playlist_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,String>(2)?,"owner_id":r.get::<_,String>(3)?,"owner":r.get::<_,String>(4)?,"revision":r.get::<_,i64>(5)?,"favorite":r.get::<_,bool>(6)?,"count":r.get::<_,i64>(7)?}),
    )
}
pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let result=state.db.call(move|db|{
        let playlist=db.query_row("SELECT p.id,p.name,p.description,p.owner_id,u.username,p.revision,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.user_id=?1 AND f.playlist_id=p.id),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p JOIN users u ON u.id=p.owner_id WHERE p.id=?2",params![p.user.id,id],playlist_row).optional()?;
        let Some(mut playlist)=playlist else{return Ok(None);};
        let items=db.prepare("SELECT c.id,c.kind,c.title,c.available FROM playlist_items i JOIN media_cards c ON c.id=i.media_id WHERE i.playlist_id=?1 ORDER BY i.position")?.query_map([id],card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        playlist["items"]=json!(items);Ok(Some(playlist))
    }).await?;
    Ok(Json(result.ok_or_else(ApiError::not_found)?))
}
#[derive(Deserialize)]
pub struct Save {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    items: Vec<String>,
}
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Save>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    save(&state, p, None, input).await.map(Json)
}
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Save>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    save(&state, p, Some(id), input).await.map(Json)
}
async fn save(state: &AppState, p: Principal, key: Option<String>, input: Save) -> Result<Value> {
    if input.name.trim().is_empty()
        || input.name.len() > 160
        || input.description.len() > 2000
        || input.items.len() > 500
    {
        return Err(ApiError::bad(
            "Enter a name, a description up to 2,000 characters and up to 500 tracks",
        ));
    }
    let creating = key.is_none();
    let key = key.unwrap_or_else(id);
    let playlist = key.clone();
    let result=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if creating {
            let count:i64=tx.query_row("SELECT COUNT(*) FROM playlists WHERE owner_id=?1",[&p.user.id],|r|r.get(0))?;
            if count>=100{return Ok(400);}
            tx.execute("INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",params![key,p.user.id,input.name.trim(),now()])?;
        } else {
            let owner:Option<(String,i64)>=tx.query_row("SELECT owner_id,revision FROM playlists WHERE id=?1",[&key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let Some((owner,revision))=owner else{return Ok(404);};
            if owner!=p.user.id{return Ok(403);}
            if revision!=input.revision{return Ok(409);}
        }
        if !validate_tracks(&tx,&input.items)?{return Ok(400);}
        tx.execute("UPDATE playlists SET name=?1,description=?2,revision=revision+?3,updated_at=?4 WHERE id=?5",params![input.name.trim(),input.description,if creating{0}else{1},now(),key])?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id=?1",[&key])?;
        for (position,media) in input.items.iter().enumerate(){tx.execute("INSERT INTO playlist_items VALUES (?1,?2,?3)",params![key,position as i64,media])?;}
        tx.commit()?;Ok(200)
    }).await?;
    match result {
        400 => {
            return Err(ApiError::bad(
                "Use catalog music tracks and at most 100 owned playlists",
            ));
        }
        403 => return Err(ApiError::forbidden()),
        404 => return Err(ApiError::not_found()),
        409 => {
            return Err(ApiError::conflict(
                "This playlist changed; reload before editing",
            ));
        }
        _ => {}
    }
    state
        .emit(None, "playlists.changed", json!({"id":playlist}))
        .await?;
    Ok(json!({"id":playlist}))
}
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let key = id.clone();
    let code=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let owner:Option<String>=tx.query_row("SELECT owner_id FROM playlists WHERE id=?1",[&key],|r|r.get(0)).optional()?;
        let Some(owner)=owner else{return Ok(404);};
        if owner!=p.user.id{return Ok(403);}
        tx.execute("DELETE FROM playlists WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'playlist.delete',?2,?3)",params![p.user.id,key,now()])?;
        tx.commit()?;Ok(200)
    }).await?;
    if code == 404 {
        return Err(ApiError::not_found());
    }
    if code == 403 {
        return Err(ApiError::forbidden());
    }
    state
        .emit(None, "playlists.changed", json!({"id":id}))
        .await?;
    Ok(Json(json!({"deleted":true})))
}
#[derive(Deserialize)]
pub struct Favorite {
    favorite: bool,
}
pub async fn favorite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Favorite>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    let key = id.clone();
    let found = state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
                [&key],
                |r| r.get::<_, bool>(0),
            )? {
                return Ok(false);
            }
            if input.favorite {
                tx.execute(
                    "INSERT OR IGNORE INTO playlist_favorites VALUES (?1,?2)",
                    params![user, key],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM playlist_favorites WHERE user_id=?1 AND playlist_id=?2",
                    params![user, key],
                )?;
            }
            tx.commit()?;
            Ok(true)
        })
        .await?;
    if !found {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(p.user.id), "playlists.changed", json!({"id":id}))
        .await?;
    Ok(Json(json!({"favorite":input.favorite})))
}
