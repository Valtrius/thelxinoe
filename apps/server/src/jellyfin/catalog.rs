use super::{Query, canonical};
use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::{
    extract::Request,
    response::{IntoResponse, Response},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::Principal;

pub const PLAYLIST_VIEW: &str = "275504fb-fdb1-49b3-bf8d-730b56915677";
pub fn item_type(kind: &str) -> &'static str {
    match kind {
        "movie" => "Movie",
        "show" => "Series",
        "season" => "Season",
        "episode" => "Episode",
        "artist" => "MusicArtist",
        "album" => "MusicAlbum",
        "track" => "Audio",
        _ => "Folder",
    }
}
fn kind(name: &str) -> &str {
    match name {
        "movie" => "movie",
        "series" => "show",
        "season" => "season",
        "episode" => "episode",
        "musicartist" => "artist",
        "musicalbum" => "album",
        "audio" => "track",
        _ => "unsupported",
    }
}
pub fn ticks(seconds: f64) -> i64 {
    (seconds.max(0.0) * 10_000_000.0).round() as i64
}
pub fn result(items: Vec<Value>, total: usize, start: usize) -> Value {
    json!({"Items":items,"TotalRecordCount":total,"StartIndex":start})
}
pub async fn views(state: &AppState) -> Result<Value> {
    let server = state.server_id.to_string();
    let items = state.db.call(move |db| {
        let mut items = db.prepare("SELECT id,name,kind FROM library_roots ORDER BY name")?.query_map([], |r| {
            let domain: String = r.get(2)?;
            Ok(json!({"Id":r.get::<_,String>(0)?,"Name":r.get::<_,String>(1)?,"Type":"CollectionFolder","IsFolder":true,"CollectionType":if domain=="shows" {"tvshows"}else{&domain},"ServerId":server,"ImageTags":{}}))
        })?.collect::<std::result::Result<Vec<_>,_>>()?;
        items.push(json!({"Id":PLAYLIST_VIEW,"Name":"Playlists","Type":"CollectionFolder","IsFolder":true,"CollectionType":"playlists","ServerId":server,"ImageTags":{}}));
        Ok(items)
    }).await?;
    let total = items.len();
    Ok(result(items, total, 0))
}

const SELECT: &str = "SELECT m.id,m.kind,c.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,m.created_at,m.root_id,COALESCE(u.favorite,0),COALESCE(u.watched,0),COALESCE(ep.position,0),COALESCE(ep.duration,0),p.kind,p.sort_number,pc.title,p.parent_id,gc.title,(SELECT f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1 ORDER BY f.edition,f.id LIMIT 1),(SELECT COUNT(*) FROM media ch WHERE ch.parent_id=m.id),COUNT(*) OVER() FROM media m JOIN media_cards c ON c.id=m.id LEFT JOIN media_state u ON u.media_id=m.id AND u.user_id=?1 LEFT JOIN edition_progress ep ON ep.rowid=(SELECT rowid FROM edition_progress WHERE media_id=m.id AND user_id=?1 ORDER BY updated_at DESC,edition LIMIT 1) LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media_cards pc ON pc.id=p.id LEFT JOIN media_cards gc ON gc.id=p.parent_id";

fn row(r: &rusqlite::Row<'_>, server: &str) -> rusqlite::Result<Value> {
    let id: String = r.get(0)?;
    let kind: String = r.get(1)?;
    let title: String = r.get(2)?;
    let parent: Option<String> = r.get(3)?;
    let metadata: Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
    let overrides: Value = serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default();
    let probe: Value = r
        .get::<_, Option<String>>(19)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(r.get(13)?);
    let position: f64 = r.get(12)?;
    let played: bool = r.get(11)?;
    let year = overrides["year"]
        .as_i64()
        .or_else(|| {
            metadata["release_date"]
                .as_str()
                .or(metadata["first_air_date"].as_str())
                .and_then(|s| s.get(..4))
                .and_then(|s| s.parse().ok())
        })
        .or(r.get(4)?);
    let mut value = json!({"Id":id,"ServerId":server,"Name":title,"SortName":title,"Type":item_type(&kind),"IsFolder":!["movie","episode","track"].contains(&kind.as_str()),"MediaType":if kind=="track"{"Audio"}else{"Video"},"LocationType":"FileSystem","ParentId":parent.clone().unwrap_or(r.get::<_,String>(9)?),"ProductionYear":year,"IndexNumber":r.get::<_,Option<i64>>(5)?,"RunTimeTicks":ticks(duration),"Overview":overrides.get("overview").or(metadata.get("overview")),"ChildCount":r.get::<_,i64>(20)?,"Genres":[],"People":[],"Studios":[],"ProviderIds":{},"ImageTags":{},"BackdropImageTags":[],"MediaSources":[],"UserData":{"Key":id,"ItemId":id,"IsFavorite":r.get::<_,bool>(10)?,"Played":played,"PlayCount":if played {1}else{0},"PlaybackPositionTicks":if played {0}else{ticks(position)},"PlayedPercentage":if duration>0.0{position/duration*100.0}else{0.0}}});
    if metadata["artwork_cached"] == true {
        value["ImageTags"] = json!({"Primary":format!("{}",r.get::<_,i64>(8)?)});
    }
    if let Some(genres) = metadata["genres"].as_array() {
        value["Genres"] = json!(
            genres
                .iter()
                .filter_map(|g| g["name"].as_str())
                .collect::<Vec<_>>()
        );
    }
    match kind.as_str() {
        "episode" => {
            value["SeasonId"] = json!(parent);
            value["SeriesId"] = json!(r.get::<_, Option<String>>(17)?);
            value["SeriesName"] = json!(r.get::<_, Option<String>>(18)?);
            value["SeasonName"] = json!(r.get::<_, Option<String>>(16)?);
            value["ParentIndexNumber"] = json!(r.get::<_, Option<i64>>(15)?);
        }
        "season" => {
            value["SeriesId"] = json!(parent);
            value["SeriesName"] = json!(r.get::<_, Option<String>>(16)?);
        }
        "album" => {
            value["AlbumArtist"] = json!(r.get::<_, Option<String>>(16)?);
            value["ArtistItems"] = json!([{"Id":parent,"Name":r.get::<_,Option<String>>(16)?}]);
        }
        "track" => {
            value["AlbumId"] = json!(parent);
            value["Album"] = json!(r.get::<_, Option<String>>(16)?);
            value["Artists"] = json!([r.get::<_, Option<String>>(18)?]);
            value["ArtistItems"] = json!([{"Id":r.get::<_,Option<String>>(17)?,"Name":r.get::<_,Option<String>>(18)?}]);
        }
        _ => {}
    }
    Ok(value)
}

pub async fn browse(
    state: &AppState,
    p: &Principal,
    query: &Query,
    id: Option<&str>,
) -> Result<Value> {
    let q = query.clone();
    let uid = p.user.id.clone();
    let server = state.server_id.to_string();
    let id = id.map(canonical);
    let parent = q.get("parentid").map(|s| canonical(s));
    if parent.as_deref() == Some(PLAYLIST_VIEW)
        || q.get("includeitemtypes")
            .is_some_and(|s| s.eq_ignore_ascii_case("Playlist"))
    {
        return playlists(state, p, id.as_deref()).await;
    }
    if let Some(id) = id.as_deref() {
        let roots = views(state).await?;
        if let Some(root) = roots["Items"]
            .as_array()
            .and_then(|a| a.iter().find(|i| i["Id"] == id))
        {
            return Ok(root.clone());
        }
    }
    let start = q
        .get("startindex")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1_000_000);
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let single = id.is_some();
    let (mut items,total)=state.db.call(move|db| {
        let mut args:Vec<rusqlite::types::Value>=vec![uid.into()];
        let mut conditions=Vec::new();
        let mut bind=|value:String|{args.push(value.into());format!("?{}",args.len())};
        if let Some(id)=id {conditions.push(format!("m.id={}",bind(id)));}
        if let Some(parent)=parent {
            let arg=bind(parent);
            if q.get("recursive").is_some_and(|v|v.eq_ignore_ascii_case("true")) {
                conditions.push(format!("(m.root_id={arg} OR m.id IN (WITH RECURSIVE descendants(id) AS (SELECT id FROM media WHERE parent_id={arg} UNION ALL SELECT m.id FROM media m JOIN descendants d ON m.parent_id=d.id) SELECT id FROM descendants))"));
            }else{conditions.push(format!("(m.parent_id={arg} OR (m.root_id={arg} AND m.parent_id IS NULL))"));}
        }
        if let Some(types)=q.get("includeitemtypes") {
            let types=types.to_ascii_lowercase().split(',').map(kind).map(str::to_owned).collect::<Vec<_>>();
            conditions.push(format!("m.kind IN (SELECT value FROM json_each({}))",bind(json!(types).to_string())));
        }
        if let Some(search)=q.get("searchterm") {conditions.push(format!("instr(lower(c.title),lower({}))>0",bind(search.clone())));}
        if let Some(ids)=q.get("ids") {conditions.push(format!("m.id IN (SELECT value FROM json_each({}))",bind(json!(ids.split(',').map(canonical).collect::<Vec<_>>()).to_string())));}
        for (key,column) in [("isfavorite","favorite"),("isplayed","watched")] {
            if let Some(value)=q.get(key) { conditions.push(format!("COALESCE(u.{column},0)={}",i32::from(value.eq_ignore_ascii_case("true")))); }
        }
        if let Some(filters)=q.get("filters") {for filter in filters.to_ascii_lowercase().split(','){match filter{"isfavorite"=>conditions.push("COALESCE(u.favorite,0)=1".into()),"isplayed"=>conditions.push("COALESCE(u.watched,0)=1".into()),"isunplayed"=>conditions.push("COALESCE(u.watched,0)=0".into()),"isresumable"=>conditions.push("ep.position>0 AND ep.position<ep.duration*0.9 AND m.kind IN ('movie','episode')".into()),_=>{}}}}
        let sort=match q.get("sortby").map(|s|s.to_ascii_lowercase()).as_deref(){Some("datecreated")=>"m.created_at",Some("dateplayed")=>"ep.updated_at",Some("random")=>"random()",Some("premieredate")|Some("productionyear")=>"m.year",Some("indexnumber")|Some("parentindexnumber,indexnumber")=>"m.sort_number",_=>"c.title COLLATE NOCASE"};
        let direction=if q.get("sortorder").is_some_and(|s|s.eq_ignore_ascii_case("Descending")){"DESC"}else{"ASC"};
        let sql=format!("{SELECT} {} ORDER BY {sort} {direction},m.id LIMIT {limit} OFFSET {start}",if conditions.is_empty(){String::new()}else{format!("WHERE {}",conditions.join(" AND "))});
        let rows=db.prepare(&sql)?.query_map(rusqlite::params_from_iter(args),|r|Ok((row(r,&server)?,r.get::<_,i64>(21)? as usize)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let total=rows.first().map_or(0,|v|v.1);
        Ok((rows.into_iter().map(|v|v.0).collect::<Vec<_>>(),total))
    }).await?;
    if items
        .iter()
        .any(|item| item["ImageTags"]["Primary"].is_string())
    {
        let grant = crate::grants::issue(state, p, "artwork", 300).await?;
        for item in &mut items {
            if item["ImageTags"]["Primary"].is_string() {
                item["ImageTags"]["Primary"] = json!(grant);
            }
        }
    }
    if single {
        items.into_iter().next().ok_or_else(ApiError::not_found)
    } else {
        Ok(result(items, total, start))
    }
}

pub async fn playlists(state: &AppState, p: &Principal, id: Option<&str>) -> Result<Value> {
    let user = p.user.id.clone();
    let id = id.map(canonical);
    let single = id.is_some();
    let server = state.server_id.to_string();
    let items=state.db.call(move|db|Ok(db.prepare("SELECT p.id,p.name,p.description,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.playlist_id=p.id AND f.user_id=?1),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p WHERE ?2 IS NULL OR p.id=?2 ORDER BY p.name LIMIT 500")?.query_map(params![user,id],|r|Ok(json!({"Id":r.get::<_,String>(0)?,"Name":r.get::<_,String>(1)?,"Overview":r.get::<_,String>(2)?,"Type":"Playlist","MediaType":"Audio","IsFolder":true,"ServerId":server,"ChildCount":r.get::<_,i64>(4)?,"ImageTags":{},"UserData":{"IsFavorite":r.get::<_,bool>(3)?,"Played":false,"PlaybackPositionTicks":0}})))?.collect::<std::result::Result<Vec<_>,_>>()?)).await?;
    if single {
        items.into_iter().next().ok_or_else(ApiError::not_found)
    } else {
        let total = items.len();
        Ok(result(items, total, 0))
    }
}

pub async fn image(state: &AppState, id: &str, kind: &str, request: Request) -> Result<Response> {
    if kind != "Primary" {
        return Err(ApiError::not_found());
    }
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| ApiError::not_found())?
        .to_string();
    let path = state.config.cache.join("artwork").join(id);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::not_found())?;
    let content_type = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let mut response = tower_http::services::ServeFile::new(path)
        .try_call(request)
        .await
        .map_err(anyhow::Error::from)?
        .into_response();
    response
        .headers_mut()
        .insert("content-type", content_type.parse().unwrap());
    Ok(response)
}

pub async fn display_preferences(
    state: &AppState,
    p: &Principal,
    id: &str,
    q: &Query,
    body: Option<Value>,
) -> Result<Value> {
    let uid = p.user.id.clone();
    let id = id.to_owned();
    let client = q.get("client").cloned().unwrap_or_default();
    if id.len() > 128 || client.len() > 128 {
        return Err(ApiError::bad("Invalid display preference identifier"));
    }
    Ok(state.db.call(move|db|{
        if let Some(value)=body {let count:i64=db.query_row("SELECT COUNT(*) FROM compat_preferences WHERE user_id=?1",[&uid],|r|r.get(0))?;if count>=100 {anyhow::bail!("Too many display preferences");}db.execute("INSERT INTO compat_preferences VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,client,preference_id) DO UPDATE SET value=excluded.value",params![uid,client,id,value.to_string()])?;}
        Ok(db.query_row("SELECT value FROM compat_preferences WHERE user_id=?1 AND client=?2 AND preference_id=?3",params![uid,client,id],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str(&v).ok()).unwrap_or_else(||json!({"Id":id,"Client":client,"SortBy":"SortName","SortOrder":"Ascending","RememberIndexing":false,"PrimaryImageHeight":250,"PrimaryImageWidth":250,"CustomPrefs":{},"ScrollDirection":"Horizontal","ShowBackdrop":true,"RememberSorting":false,"ShowSidebar":false})))
    }).await?)
}
