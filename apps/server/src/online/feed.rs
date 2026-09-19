use super::{bounded_response, sync};
use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

#[derive(Default, Deserialize)]
pub struct Filter {
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    watchlist: bool,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    hide_shorts: bool,
    #[serde(default)]
    unwatched: bool,
    #[serde(default)]
    search: String,
    #[serde(default)]
    channel: String,
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if filter.offset > 100000
        || filter.search.len() > 200
        || (!filter.channel.is_empty() && !sync::identifier(&filter.channel, 24))
    {
        return Err(ApiError::bad("Invalid feed filter"));
    }
    let owner = p.user.id.clone();
    let (mut items,total,sync)=state.db.call(move|db|{
        let from="FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id) WHERE v.user_id=?1 AND (?2=0 OR s.watchlist=1) AND (?3=0 OR s.pinned=1) AND (?4=0 OR COALESCE(v.is_short,0)=0) AND (?5=0 OR COALESCE(s.watched,0)=0) AND (?6='' OR instr(lower(v.title),lower(?6))>0 OR instr(lower(v.channel_title),lower(?6))>0) AND (?7='' OR v.channel_id=?7) AND (?2=1 OR ?3=1 OR EXISTS(SELECT 1 FROM youtube_subscriptions c WHERE c.user_id=v.user_id AND c.channel_id=v.channel_id AND c.active=1))";
        let query=params![owner,filter.watchlist,filter.pinned,filter.hide_shorts,filter.unwatched,filter.search,filter.channel];
        let total=db.query_row(&format!("SELECT COUNT(*) {from}"),query,|r|r.get::<_,u32>(0))?;
        let sql=format!("SELECT v.video_id,v.title,v.channel_title,v.published_at,v.duration,v.broadcast,v.available,v.is_short,v.metadata_at,COALESCE(s.watchlist,0),COALESCE(s.pinned,0),COALESCE(s.watched,0),COALESCE(s.position,0),v.privacy {from} ORDER BY CASE WHEN ?2=1 THEN s.added_at ELSE v.published_at END DESC,v.video_id LIMIT 50 OFFSET ?8");
        let rows=db.prepare(&sql)?.query_map(params![owner,filter.watchlist,filter.pinned,filter.hide_shorts,filter.unwatched,filter.search,filter.channel,filter.offset],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"channel":r.get::<_,String>(2)?,"published_at":r.get::<_,i64>(3)?,"duration":r.get::<_,Option<i64>>(4)?,"broadcast":r.get::<_,String>(5)?,"available":r.get::<_,bool>(6)?,"is_short":r.get::<_,Option<bool>>(7)?,"pending":r.get::<_,i64>(8)?==0,"watchlist":r.get::<_,bool>(9)?,"pinned":r.get::<_,bool>(10)?,"watched":r.get::<_,bool>(11)?,"position":r.get::<_,f64>(12)?,"privacy":r.get::<_,String>(13)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let sync=db.query_row("SELECT next_run,last_complete,error,cursor FROM youtube_sync WHERE user_id=?1",[owner],|r|Ok(json!({"next_run":r.get::<_,i64>(0)?,"last_complete":r.get::<_,Option<i64>>(1)?,"error":r.get::<_,Option<String>>(2)?,"in_progress":r.get::<_,String>(3)?!="{}"}))).optional()?;
        Ok((rows,total,sync))
    }).await?;
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    for item in &mut items {
        if item["privacy"] == "public" {
            item["artwork_url"] = json!(format!(
                "/api/v1/online/youtube/videos/{}/artwork?grant={grant}",
                item["id"].as_str().unwrap()
            ));
        }
    }
    Ok(Json(json!({"items":items,"total":total,"sync":sync})))
}
#[derive(Deserialize)]
pub struct Add {
    url: String,
}
pub(super) fn video_id(value: &str) -> Option<String> {
    let value = value.trim();
    if sync::identifier(value, 11) {
        return Some(value.into());
    }
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let id = match url.host_str()? {
        "youtu.be" => url.path().trim_start_matches('/').to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            if url.path() == "/watch" {
                url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
            } else {
                let mut pieces = url.path().trim_start_matches('/').split('/');
                if !matches!(pieces.next()?, "shorts" | "live" | "embed") {
                    return None;
                }
                let id = pieces.next()?.to_owned();
                if pieces.next().is_some() {
                    return None;
                }
                id
            }
        }
        _ => return None,
    };
    sync::identifier(&id, 11).then_some(id)
}
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Add>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let video = video_id(&input.url)
        .ok_or_else(|| ApiError::bad("Enter a YouTube video URL or video ID"))?;
    let result = video.clone();
    let user = p.user.id.clone();
    let added=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_state WHERE user_id=?1 AND video_id=?2 AND watchlist=1)",params![user,video],|r|r.get::<_,bool>(0))?;
        let count=tx.query_row("SELECT COUNT(*) FROM youtube_state WHERE user_id=?1 AND (watchlist=1 OR pinned=1)",[&user],|r|r.get::<_,u32>(0))?;
        if !exists&&count>=1000{return Ok(false);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES (?1,?2,?2) ON CONFLICT DO NOTHING",params![user,video])?;
        tx.execute("INSERT INTO youtube_state(user_id,video_id,watchlist,added_at,updated_at) VALUES (?1,?2,1,?3,?3) ON CONFLICT(user_id,video_id) DO UPDATE SET watchlist=1,added_at=CASE WHEN watchlist=0 THEN excluded.added_at ELSE added_at END,updated_at=excluded.updated_at",params![user,video,now()])?;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),user])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !added {
        return Err(ApiError::conflict(
            "Your YouTube watchlist and pins can retain at most 1,000 videos",
        ));
    }
    state
        .emit(Some(p.user.id), "youtube.changed", json!({}))
        .await?;
    Ok(Json(json!({"id":result,"watchlist":true})))
}
#[derive(Deserialize)]
pub struct Edit {
    watchlist: Option<bool>,
    pinned: Option<bool>,
    watched: Option<bool>,
}
pub async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video): Path<String>,
    Json(input): Json<Edit>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    let updated=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",params![user,video],|r|r.get::<_,bool>(0))? {return Ok(0);}
        if input.watchlist==Some(true)||input.pinned==Some(true) {
            let count=tx.query_row("SELECT COUNT(*) FROM youtube_state WHERE user_id=?1 AND video_id<>?2 AND (watchlist=1 OR pinned=1)",params![user,video],|r|r.get::<_,u32>(0))?;
            if count>=1000 {return Ok(2);}
        }
        tx.execute("INSERT INTO youtube_state(user_id,video_id,added_at,updated_at) VALUES (?1,?2,?3,?3) ON CONFLICT DO NOTHING",params![user,video,now()])?;
        tx.execute("UPDATE youtube_state SET added_at=CASE WHEN ?1=1 AND watchlist=0 THEN ?4 ELSE added_at END,watchlist=COALESCE(?1,watchlist),pinned=COALESCE(?2,pinned),watched=COALESCE(?3,watched),updated_at=?4 WHERE user_id=?5 AND video_id=?6",params![input.watchlist,input.pinned,input.watched,now(),user,video])?;
        tx.commit()?;Ok(1)
    }).await?;
    if updated == 2 {
        return Err(ApiError::conflict(
            "Your YouTube watchlist and pins can retain at most 1,000 videos",
        ));
    }
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(p.user.id), "youtube.changed", json!({}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
pub async fn delete_data(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let user = p.user.id.clone();
    state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for table in ["oauth_attempts","online_accounts"]{tx.execute(&format!("DELETE FROM {table} WHERE user_id=?1 AND provider='youtube'"),[&user])?;}
        for table in ["youtube_sync","youtube_subscriptions","youtube_videos"]{tx.execute(&format!("DELETE FROM {table} WHERE user_id=?1"),[&user])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.delete-data','youtube',?2)",params![user,now()])?;
        tx.commit()?;Ok(())
    }).await?;
    state
        .emit(
            Some(p.user.id),
            "online.account.changed",
            json!({"provider":"youtube"}),
        )
        .await?;
    Ok(Json(json!({"deleted":true})))
}
#[derive(Deserialize)]
pub struct Artwork {
    grant: String,
}
pub async fn artwork(
    State(state): State<AppState>,
    Path(video): Path<String>,
    Query(query): Query<Artwork>,
) -> Result<Response> {
    let p = grants::resolve(&state, &query.grant, "youtube-artwork", false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if !sync::identifier(&video, 11) {
        return Err(ApiError::not_found());
    }
    let id = video.clone();
    let allowed=state.db.call(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2 AND privacy='public')",params![p.user.id,id],|r|r.get::<_,bool>(0))?)).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    let _slot = state
        .online
        .slots
        .acquire()
        .await
        .map_err(|_| ApiError::not_found())?;
    let response = state
        .online
        .http
        .get(format!("https://i.ytimg.com/vi/{video}/hqdefault.jpg"))
        .send()
        .await
        .map_err(|_| ApiError::not_found())?;
    let (status, _, bytes) = bounded_response(response)
        .await
        .map_err(|_| ApiError::not_found())?;
    if !status.is_success() || !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(ApiError::not_found());
    }
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=300"),
        ],
        bytes,
    )
        .into_response())
}
