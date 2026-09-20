//! The subscription feed contract shared by the YouTwitch presentation components.
use crate::{
    AppState,
    error::{ApiError, Result},
    grants, security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::{Connection, params, params_from_iter, types::Value as SqlValue};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::now;

pub(super) fn date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

pub(super) const VIDEO_SELECT: &str = "SELECT v.video_id,v.channel_id,v.channel_title,v.title,v.published_at,v.duration,v.broadcast,v.available,v.metadata_at,v.privacy,COALESCE(s.watched,0),COALESCE(s.position,0),COALESCE(s.pinned,0),c.thumbnail_url,v.scheduled_start,v.actual_start,v.actual_end,d.state,d.size,d.requested_at,d.error FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id) LEFT JOIN youtube_subscriptions c ON c.user_id=v.user_id AND c.channel_id=v.channel_id LEFT JOIN youtube_downloads d ON d.video_id=v.video_id";

pub(super) fn video(row: &rusqlite::Row<'_>, grant: &str) -> rusqlite::Result<Value> {
    let id: String = row.get(0)?;
    let duration: Option<i64> = row.get(5)?;
    let position: f64 = row.get(11)?;
    let broadcast: String = row.get(6)?;
    let status: Option<String> = row.get(17)?;
    let download = status.map(|status| -> rusqlite::Result<Value> { Ok(json!({
        "videoId":id,"status":match status.as_str(){"ready"=>"ready","queued"=>"queued","downloading"=>"downloading","processing"=>"processing",_=>"failed"},
        "fileSizeBytes":row.get::<_,Option<i64>>(18)?.unwrap_or(0),"pinned":row.get::<_,bool>(12)?,"quality":"1080p",
        "requestedAt":date(row.get::<_,Option<i64>>(19)?.unwrap_or(0)),"errorMessage":row.get::<_,Option<String>>(20)?
    })) }).transpose()?;
    Ok(json!({
        "videoId":id,"channelId":row.get::<_,String>(1)?,"channelName":row.get::<_,String>(2)?,
        "title":row.get::<_,String>(3)?,"publishedAt":date(row.get(4)?),"durationSeconds":duration,
        "isLive":broadcast=="live","isUpcoming":broadcast=="upcoming","isLiveReplay":broadcast=="replay","broadcastState":broadcast,
        "availabilityStatus":if row.get::<_,bool>(7)? {"available"}else{"unavailable"},"metadataPending":row.get::<_,i64>(8)?==0,
        "thumbnailUrl":if row.get::<_,String>(9)?=="public" {Some(format!("/api/v1/online/youtube/videos/{id}/artwork?grant={grant}"))}else{None},
        "channelThumbnailUrl":row.get::<_,Option<String>>(13)?,"scheduledStartAt":row.get::<_,Option<String>>(14)?,"actualStartAt":row.get::<_,Option<String>>(15)?,"actualEndAt":row.get::<_,Option<String>>(16)?,
        "positionSeconds":position,"watchedPercentage":duration.filter(|v|*v>0).map(|d|(position/d as f64*100.0).clamp(0.0,100.0)).unwrap_or(0.0),
        "isWatched":row.get::<_,bool>(10)?,"download":download
    }))
}

pub(super) fn one(db: &Connection, user: &str, id: &str, grant: &str) -> anyhow::Result<Value> {
    Ok(db.query_row(
        &format!("{VIDEO_SELECT} WHERE v.user_id=?1 AND v.video_id=?2"),
        params![user, id],
        |r| video(r, grant),
    )?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    search: String,
    watch_states: Vec<String>,
    include_shorts: bool,
    include_live: bool,
    include_live_replays: bool,
    include_upcoming: bool,
    channel_id: Option<String>,
    duration_filter: String,
    published_filter: String,
    sort_field: String,
    sort_direction: String,
    grouping: String,
    download_filter: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    query: Query,
    page: u32,
    page_size: u32,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let q = &input.query;
    if input.page > 2000
        || !(1..=100).contains(&input.page_size)
        || q.search.len() > 200
        || q.watch_states.is_empty()
        || q.watch_states.len() > 3
        || q.watch_states
            .iter()
            .any(|s| !["unwatched", "in_progress", "watched"].contains(&s.as_str()))
        || !["any", "under_10", "10_plus", "30_plus"].contains(&q.duration_filter.as_str())
        || !["any", "7_days", "30_days"].contains(&q.published_filter.as_str())
        || !["date", "channel", "duration"].contains(&q.sort_field.as_str())
        || !["asc", "desc"].contains(&q.sort_direction.as_str())
        || !["smart", "day", "week", "month", "none"].contains(&q.grouping.as_str())
        || !["all", "downloaded"].contains(&q.download_filter.as_str())
        || q.channel_id.as_ref().is_some_and(|s| s.len() > 24)
    {
        return Err(ApiError::bad("Invalid YouTube feed filters"));
    }
    let grant = grants::issue(&state, &p, "youtube-artwork", 300).await?;
    let output=state.db.call(move |db| {
        let q=input.query;
        let user=p.user.id;
        let base="v.user_id=? AND EXISTS(SELECT 1 FROM youtube_subscriptions sub WHERE sub.user_id=v.user_id AND sub.channel_id=v.channel_id AND sub.active=1)";
        let mut where_sql=format!("{base} AND (? OR COALESCE(v.is_short,0)=0) AND (? OR v.broadcast<>'live') AND (? OR v.broadcast<>'replay') AND (? OR v.broadcast<>'upcoming')");
        let mut binds:Vec<SqlValue>=vec![user.clone().into(),(q.include_shorts as i64).into(),(q.include_live as i64).into(),(q.include_live_replays as i64).into(),(q.include_upcoming as i64).into()];
        if !q.search.is_empty(){where_sql.push_str(" AND (instr(lower(v.title),lower(?))>0 OR instr(lower(v.channel_title),lower(?))>0)");binds.push(q.search.clone().into());binds.push(q.search.into());}
        if let Some(channel)=q.channel_id.filter(|s|!s.is_empty()){where_sql.push_str(" AND v.channel_id=?");binds.push(channel.into());}
        where_sql.push_str(match q.duration_filter.as_str(){"under_10"=>" AND v.duration<600","10_plus"=>" AND v.duration>=600","30_plus"=>" AND v.duration>=1800",_=>""});
        if q.published_filter!="any" {where_sql.push_str(" AND v.published_at>=?");binds.push((now()-if q.published_filter=="7_days"{7}else{30}*86400).into());}
        if q.download_filter=="downloaded"{where_sql.push_str(" AND d.state='ready'");}
        let from="FROM youtube_videos v LEFT JOIN youtube_state s USING(user_id,video_id) LEFT JOIN youtube_downloads d ON d.video_id=v.video_id";
        let mut counts=db.query_row(&format!("SELECT COUNT(*),COALESCE(SUM(COALESCE(s.watched,0)=0 AND COALESCE(s.position,0)<=1),0),COALESCE(SUM(COALESCE(s.watched,0)=0 AND s.position>1),0),COALESCE(SUM(s.watched=1),0) {from} WHERE {where_sql}"),params_from_iter(&binds),|r|Ok(json!({"all":r.get::<_,i64>(0)?,"unwatched":r.get::<_,i64>(1)?,"inProgress":r.get::<_,i64>(2)?,"watched":r.get::<_,i64>(3)?})))?;
        for (key,condition) in [("shorts","v.is_short=1"),("live","v.broadcast='live'"),("liveReplays","v.broadcast='replay'"),("upcoming","v.broadcast='upcoming'")] {
            counts[key]=json!(db.query_row(&format!("SELECT COUNT(*) FROM youtube_videos v WHERE {base} AND {condition}"),[&user],|r|r.get::<_,i64>(0))?);
        }
        let channels=db.prepare("SELECT channel_id,title FROM youtube_subscriptions WHERE user_id=?1 AND active=1 ORDER BY title COLLATE NOCASE")?.query_map([&user],|r|Ok(json!({"channelId":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        counts["subscribedChannelCount"]=json!(channels.len());
        let states=q.watch_states.iter().map(|s|match s.as_str(){"watched"=>"s.watched=1","in_progress"=>"(COALESCE(s.watched,0)=0 AND s.position>1)",_=>"(COALESCE(s.watched,0)=0 AND COALESCE(s.position,0)<=1)"}).collect::<Vec<_>>().join(" OR ");
        where_sql.push_str(&format!(" AND ({states})"));
        let sort=match q.sort_field.as_str(){"channel"=>"v.channel_title COLLATE NOCASE","duration"=>"v.duration",_=>"v.published_at"};
        let sql=format!("{VIDEO_SELECT} WHERE {where_sql} ORDER BY {sort} {},v.video_id LIMIT ? OFFSET ?",q.sort_direction);
        binds.push((input.page_size as i64+1).into());binds.push((input.page as i64*input.page_size as i64).into());
        let mut items=db.prepare(&sql)?.query_map(params_from_iter(&binds),|r|video(r,&grant))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let more=items.len()>input.page_size as usize;items.truncate(input.page_size as usize);
        Ok(json!({"items":items,"page":input.page,"pageSize":input.page_size,"hasMore":more,"counts":counts,"channels":channels}))
    }).await?;
    Ok(Json(output))
}
