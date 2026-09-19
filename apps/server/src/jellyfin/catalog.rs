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
            if let Some(number) = r.get::<_, Option<i64>>(5)? {
                value["IndexNumber"] = json!(number % 10000);
                value["ParentIndexNumber"] = json!(number / 10000);
            }
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
    if let Some(id) = id {
        if is_playlist(state, id).await? {
            return playlists(state, p, Some(id), query).await;
        }
    } else if let Some(parent) = query.get("parentid")
        && is_playlist(state, parent).await?
    {
        return playlist_items(state, p, parent, query).await;
    }
    browse_media(state, p, query, id).await
}
async fn browse_media(
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
        return playlists(state, p, id.as_deref(), query).await;
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
        .min(500);
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
        if let Some(types)=q.get("excludeitemtypes") {
            let types=types.to_ascii_lowercase().split(',').map(kind).map(str::to_owned).collect::<Vec<_>>();
            conditions.push(format!("m.kind NOT IN (SELECT value FROM json_each({}))",bind(json!(types).to_string())));
        }
        if let Some(folder)=q.get("isfolder") {conditions.push(format!("m.kind {} IN ('movie','episode','track')",if folder.eq_ignore_ascii_case("true"){ "NOT" } else { "" }));}
        if let Some(artists)=q.get("artistids").or(q.get("albumartistids")) {
            let arg=bind(json!(artists.split(',').map(canonical).collect::<Vec<_>>()).to_string());
            conditions.push(format!("(m.parent_id IN (SELECT value FROM json_each({arg})) OR p.parent_id IN (SELECT value FROM json_each({arg})))"));
        }
        if let Some(season)=q.get("season").and_then(|v|v.parse::<i64>().ok()) {conditions.push(format!("p.kind='season' AND p.sort_number={season}"));}
        if let Some(search)=q.get("searchterm") {conditions.push(format!("instr(lower(c.title),lower({}))>0",bind(search.clone())));}
        if let Some(ids)=q.get("ids") {conditions.push(format!("m.id IN (SELECT value FROM json_each({}))",bind(json!(ids.split(',').map(canonical).collect::<Vec<_>>()).to_string())));}
        for (key,column) in [("isfavorite","favorite"),("isplayed","watched")] {
            if let Some(value)=q.get(key) { conditions.push(format!("COALESCE(u.{column},0)={}",i32::from(value.eq_ignore_ascii_case("true")))); }
        }
        if let Some(filters)=q.get("filters") {for filter in filters.to_ascii_lowercase().split(','){match filter{"isfavorite"=>conditions.push("COALESCE(u.favorite,0)=1".into()),"isplayed"=>conditions.push("COALESCE(u.watched,0)=1".into()),"isunplayed"=>conditions.push("COALESCE(u.watched,0)=0".into()),"isresumable"=>conditions.push("ep.position>0 AND ep.position<ep.duration*0.9 AND m.kind IN ('movie','episode')".into()),_=>{}}}}
        let directions=q.get("sortorder").map(String::as_str).unwrap_or("Ascending").split(',').collect::<Vec<_>>();
        let sort=q.get("sortby").map(String::as_str).unwrap_or("SortName").split(',').take(8).enumerate().map(|(index,key)| {
            let column=match key.to_ascii_lowercase().as_str(){"datecreated"=>"m.created_at","dateplayed"=>"ep.updated_at","random"=>"random()","premieredate"|"productionyear"=>"m.year","indexnumber"=>"m.sort_number","parentindexnumber"=>"CASE WHEN m.kind='track' THEN m.sort_number/10000 ELSE p.sort_number END","communityrating"=>"json_extract(m.metadata,'$.vote_average')",_=>"c.title COLLATE NOCASE"};
            let direction=if directions.get(index).or(directions.first()).is_some_and(|s|s.eq_ignore_ascii_case("Descending")){"DESC"}else{"ASC"};
            format!("{column} {direction}")
        }).collect::<Vec<_>>().join(",");
        let filter=if conditions.is_empty(){String::new()}else{format!("WHERE {}",conditions.join(" AND "))};
        let sql=format!("{SELECT} {filter} ORDER BY {sort},m.id LIMIT {limit} OFFSET {start}");
        let rows=db.prepare(&sql)?.query_map(rusqlite::params_from_iter(&args),|r|Ok((row(r,&server)?,r.get::<_,i64>(21)? as usize)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let total=if let Some(row)=rows.first(){row.1}else{db.query_row(&format!("SELECT COUNT(*) FROM ({SELECT} {filter})"),rusqlite::params_from_iter(&args),|r|r.get::<_,i64>(0))? as usize};
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

pub async fn is_playlist(state: &AppState, id: &str) -> Result<bool> {
    let id = canonical(id);
    Ok(state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
                [id],
                |r| r.get(0),
            )?)
        })
        .await?)
}
pub async fn playlist_items(
    state: &AppState,
    p: &Principal,
    id: &str,
    query: &Query,
) -> Result<Value> {
    let id = canonical(id);
    let (revision, tracks) = state
        .db
        .call(move |db| {
            let revision = db
                .query_row("SELECT revision FROM playlists WHERE id=?1", [&id], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?;
            let Some(revision) = revision else {
                return Ok(None);
            };
            let tracks = db
                .prepare(
                    "SELECT media_id FROM playlist_items WHERE playlist_id=?1 ORDER BY position",
                )?
                .query_map([id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(Some((revision, tracks)))
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    let start = query
        .get("startindex")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);
    let page = tracks
        .iter()
        .enumerate()
        .skip(start)
        .take(limit)
        .collect::<Vec<_>>();
    let q = Query::from([
        (
            "ids".into(),
            page.iter()
                .map(|(_, id)| id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
        ("limit".into(), "500".into()),
    ]);
    let catalog = browse_media(state, p, &q, None).await?;
    let items = page
        .into_iter()
        .filter_map(|(position, id)| {
            let mut item = catalog["Items"]
                .as_array()?
                .iter()
                .find(|i| i["Id"] == *id)?
                .clone();
            item["PlaylistItemId"] = json!(format!("{revision}:{position}"));
            Some(item)
        })
        .collect();
    Ok(result(items, tracks.len(), start))
}
pub async fn playlists(
    state: &AppState,
    p: &Principal,
    id: Option<&str>,
    q: &Query,
) -> Result<Value> {
    let user = p.user.id.clone();
    let id = id.map(canonical);
    let single = id.is_some();
    let server = state.server_id.to_string();
    let favorite = q
        .get("isfavorite")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || q.get("filters")
            .is_some_and(|v| v.split(',').any(|f| f.eq_ignore_ascii_case("IsFavorite")));
    let start = if single {
        0
    } else {
        q.get("startindex")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let limit = if single {
        1
    } else {
        q.get("limit")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(100)
            .min(500)
    };
    let (items,total)=state.db.call(move|db|{
        let filter="(?2 IS NULL OR p.id=?2) AND (?3=0 OR EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.playlist_id=p.id AND f.user_id=?1))";
        let total=db.query_row(&format!("SELECT COUNT(*) FROM playlists p WHERE {filter}"),params![user,id,favorite],|r|r.get::<_,i64>(0).map(|v|v as usize))?;
        let items=db.prepare(&format!("SELECT p.id,p.name,p.description,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.playlist_id=p.id AND f.user_id=?1),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p WHERE {filter} ORDER BY p.name,p.id LIMIT ?4 OFFSET ?5"))?.query_map(params![user,id,favorite,limit,start],|r|Ok(json!({"Id":r.get::<_,String>(0)?,"Name":r.get::<_,String>(1)?,"Overview":r.get::<_,String>(2)?,"Type":"Playlist","MediaType":"Audio","IsFolder":true,"ServerId":server,"ChildCount":r.get::<_,i64>(4)?,"ImageTags":{},"UserData":{"Key":r.get::<_,String>(0)?,"ItemId":r.get::<_,String>(0)?,"IsFavorite":r.get::<_,bool>(3)?,"PlayCount":0,"Played":false,"PlaybackPositionTicks":0}})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok((items,total))
    }).await?;
    if single {
        items.into_iter().next().ok_or_else(ApiError::not_found)
    } else {
        Ok(result(items, total, start as usize))
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
    state.db.call(move|db|{
        if let Some(value)=body {
            let inserted=db.execute("INSERT INTO compat_preferences SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM compat_preferences WHERE user_id=?1 AND client=?2 AND preference_id=?3) OR (SELECT COUNT(*) FROM compat_preferences WHERE user_id=?1)<100 ON CONFLICT(user_id,client,preference_id) DO UPDATE SET value=excluded.value",params![uid,client,id,value.to_string()])?;
            if inserted==0 {return Ok(None);}
        }
        Ok(Some(db.query_row("SELECT value FROM compat_preferences WHERE user_id=?1 AND client=?2 AND preference_id=?3",params![uid,client,id],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str(&v).ok()).unwrap_or_else(||json!({"Id":id,"Client":client,"SortBy":"SortName","SortOrder":"Ascending","RememberIndexing":false,"PrimaryImageHeight":250,"PrimaryImageWidth":250,"CustomPrefs":{},"ScrollDirection":"Horizontal","ShowBackdrop":true,"RememberSorting":false,"ShowSidebar":false}))))
    }).await?.ok_or_else(||ApiError::conflict("Too many display preferences"))
}
