//! Private online libraries translated to the standard Jellyfin video/playlist API.
use super::{Query, canonical, catalog};
use crate::{
    AppState,
    error::{ApiError, Result},
    online::watchlists,
};
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{Principal, id};

pub const ROOTS: [(&str, &str, &str); 3] = [
    ("d7df0f21-caa6-45a0-9798-267150600001", "youtube", "YouTube"),
    ("d7df0f21-caa6-45a0-9798-267150600002", "twitch", "Twitch"),
    ("d7df0f21-caa6-45a0-9798-267150600003", "kick", "Kick"),
];
pub fn views(server: &str) -> Vec<Value> {
    ROOTS.iter().map(|(id, _, name)| json!({"Id":id,"Name":name,"Type":"CollectionFolder","CollectionType":"homevideos","IsFolder":true,"ServerId":server,"ImageTags":{}})).collect()
}
fn column(kind: &str) -> &'static str {
    match kind {
        "youtube" => "youtube_video_id",
        "twitch" => "twitch_channel_id",
        "kick" => "kick_slug",
        "watchlist" => "watchlist_id",
        _ => unreachable!(),
    }
}
fn mapped(db: &Connection, user: &str, kind: &str, key: &str) -> anyhow::Result<String> {
    let column = column(kind);
    db.execute(&format!("INSERT INTO compat_online_items(id,user_id,{column}) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING"), params![id(), user, key])?;
    Ok(db.query_row(
        &format!("SELECT id FROM compat_online_items WHERE user_id=?1 AND {column}=?2"),
        params![user, key],
        |r| r.get(0),
    )?)
}
#[derive(Clone)]
pub struct Identity {
    pub id: String,
    pub kind: String,
    pub key: String,
}
impl Identity {
    pub fn media(&self) -> String {
        format!("{}:{}", self.kind, self.key)
    }
    pub fn list(&self) -> Result<i64> {
        if self.kind != "watchlist" {
            return Err(ApiError::not_found());
        }
        self.key.parse().map_err(|_| ApiError::not_found())
    }
}
pub async fn resolve(state: &AppState, p: &Principal, item: &str) -> Result<Option<Identity>> {
    let item = canonical(item);
    let user = p.user.id.clone();
    Ok(state.db.call(move |db| Ok(db.query_row("SELECT id,CASE WHEN youtube_video_id IS NOT NULL THEN 'youtube' WHEN twitch_channel_id IS NOT NULL THEN 'twitch' WHEN kick_slug IS NOT NULL THEN 'kick' ELSE 'watchlist' END,CAST(COALESCE(youtube_video_id,twitch_channel_id,kick_slug,watchlist_id) AS TEXT) FROM compat_online_items WHERE id=?1 AND user_id=?2", params![item,user], |r| Ok(Identity{id:r.get(0)?,kind:r.get(1)?,key:r.get(2)?})).optional()?)).await?)
}
pub async fn ensure_lists(state: &AppState, p: &Principal) -> Result<()> {
    let user = p.user.id.clone();
    state
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            watchlists::default_list(&tx, &user)?;
            let lists = tx
                .prepare("SELECT CAST(id AS TEXT) FROM youtube_watchlists WHERE user_id=?1")?
                .query_map([&user], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for list in lists {
                mapped(&tx, &user, "watchlist", &list)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

// Query the provider's own user rows. UUIDs cannot confer access to another user's feed.
const SELECT: &str = "WITH entries AS (
 SELECT 'youtube' kind,v.video_id provider_id,v.title name,v.channel_title overview,COALESCE(v.duration,0) duration,v.broadcast='live' live,
 'https://i.ytimg.com/vi/'||v.video_id||'/mqdefault.jpg' image,COALESCE(s.position,0) position,COALESCE(s.watched,0) played,COALESCE(s.pinned,0) favorite,strftime('%Y-%m-%dT%H:%M:%SZ',v.published_at,'unixepoch') created
 FROM youtube_videos v LEFT JOIN youtube_state s ON s.user_id=v.user_id AND s.video_id=v.video_id
 WHERE v.user_id=?1 AND v.privacy='public' AND v.available=1 AND v.broadcast<>'upcoming'
 AND (EXISTS(SELECT 1 FROM youtube_subscriptions sub WHERE sub.user_id=v.user_id AND sub.channel_id=v.channel_id AND sub.active=1) OR COALESCE(s.watchlist,0)=1 OR COALESCE(s.pinned,0)=1)
 UNION ALL
 SELECT 'twitch',t.channel_id,t.display_name,t.title||' · '||t.category,0,1,'https://static-cdn.jtvnw.net/previews-ttv/live_user_'||t.login||'-640x360.jpg',0,0,COALESCE(c.favorite,0),t.started_at
 FROM twitch_streams t LEFT JOIN compat_online_items c ON c.user_id=t.user_id AND c.twitch_channel_id=t.channel_id WHERE t.user_id=?1 AND t.active=1
 UNION ALL
 SELECT 'kick',k.slug,COALESCE(NULLIF(k.display_name,''),k.slug),k.title||' · '||k.category,0,1,COALESCE(k.thumbnail_url,k.profile_image_url,''),0,0,COALESCE(c.favorite,0),k.started_at
 FROM kick_channels k LEFT JOIN compat_online_items c ON c.user_id=k.user_id AND c.kick_slug=k.slug WHERE k.user_id=?1 AND k.live=1
 )";

pub async fn browse(
    state: &AppState,
    p: &Principal,
    query: &Query,
    item: Option<&str>,
) -> Result<Option<Value>> {
    let single = if let Some(item) = item {
        let key = canonical(item);
        if let Some(root) = views(&state.server_id).into_iter().find(|r| r["Id"] == key) {
            return Ok(Some(root));
        }
        resolve(state, p, &key).await?
    } else {
        None
    };
    if item.is_some() && single.is_none() {
        return Ok(None);
    }
    if let Some(identity) = &single
        && identity.kind == "watchlist"
    {
        return Ok(Some(
            catalog::playlists(state, p, Some(&identity.id), query).await?,
        ));
    }
    let parent = query.get("parentid").map(|s| canonical(s));
    let provider = parent
        .as_ref()
        .and_then(|id| ROOTS.iter().find(|r| r.0 == id).map(|r| r.1));
    let list = if let Some(parent) = &parent {
        resolve(state, p, parent)
            .await?
            .filter(|i| i.kind == "watchlist")
    } else {
        None
    };
    if single.is_none() && parent.is_some() && provider.is_none() && list.is_none() {
        return Ok(None);
    }
    let video_only = query
        .get("includeitemtypes")
        .is_some_and(|types| types.split(',').all(|t| t.eq_ignore_ascii_case("Video")));
    let mapped_ids = if let Some(ids) = query.get("ids") {
        let mut resolved = Vec::new();
        for id in ids.split(',').take(500) {
            if let Some(item) = resolve(state, p, id).await? {
                resolved.push(item);
            }
        }
        resolved
    } else {
        Vec::new()
    };
    if single.is_none()
        && provider.is_none()
        && list.is_none()
        && !video_only
        && mapped_ids.is_empty()
    {
        return Ok(None);
    }
    let list_id = list.as_ref().map(Identity::list).transpose()?;
    let q = query.clone();
    let user = p.user.id.clone();
    let server = state.server_id.to_string();
    let is_single = single.is_some();
    let start = if is_single {
        0
    } else {
        q.get("startindex")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            .min(1_000_000)
    };
    let limit = if is_single {
        1
    } else {
        q.get("limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100)
            .min(500)
    };
    let (mut items,total)=state.db.call(move |db| {
        let mut args:Vec<rusqlite::types::Value>=vec![user.clone().into()];
        let mut filter=Vec::new();
        let mut bind=|value:String|{args.push(value.into());format!("?{}",args.len())};
        if let Some(provider)=provider {filter.push(format!("kind={}",bind(provider.into())));}
        if let Some(single)=single {filter.push(format!("kind={} AND provider_id={}",bind(single.kind),bind(single.key)));}
        if let Some(list)=list_id {filter.push(format!("kind='youtube' AND provider_id IN (SELECT video_id FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id={list})"));}
        if q.contains_key("ids") {let keys=mapped_ids.iter().map(Identity::media).collect::<Vec<_>>();filter.push(format!("kind||':'||provider_id IN (SELECT value FROM json_each({}))",bind(json!(keys).to_string())));}
        if q.get("includeitemtypes").is_some_and(|v| !v.split(',').any(|t|t.eq_ignore_ascii_case("Video"))) || q.get("excludeitemtypes").is_some_and(|v|v.split(',').any(|t|t.eq_ignore_ascii_case("Video"))) || q.get("isfolder").is_some_and(|v|v=="true") {filter.push("0".into());}
        if let Some(search)=q.get("searchterm") {filter.push(format!("instr(lower(name||' '||overview),lower({}))>0",bind(search.clone())));}
        for (key,column) in [("isfavorite","favorite"),("isplayed","played")] {if let Some(value)=q.get(key) {filter.push(format!("{column}={}",i32::from(value.eq_ignore_ascii_case("true"))));}}
        for flag in q.get("filters").map(String::as_str).unwrap_or("").to_ascii_lowercase().split(',') {match flag {"isfavorite"=>filter.push("favorite=1".into()),"isplayed"=>filter.push("played=1".into()),"isunplayed"=>filter.push("played=0".into()),"isresumable"=>filter.push("live=0 AND position>0 AND position<duration*0.9".into()),_=>{}}}
        let list_sort=list_id.map(|list|db.query_row("SELECT sort_mode,sort_direction FROM youtube_watchlists WHERE user_id=?1 AND id=?2",params![user,list],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))).transpose()?;
        let order=if list_sort.as_ref().is_some_and(|s|s.0=="date"){"created".into()}else if let Some(list)=list_id {format!("(SELECT manual_position FROM youtube_watchlist_items i WHERE i.user_id=?1 AND i.watchlist_id={list} AND i.video_id=provider_id)")} else if q.get("sortby").is_some_and(|s|s.to_ascii_lowercase().contains("date")) {"created".into()} else {"name COLLATE NOCASE".into()};
        let descending=if let Some(sort)=list_sort{sort.1=="desc"}else{q.get("sortorder").is_some_and(|s|s.eq_ignore_ascii_case("Descending"))};
        let direction=if descending{"DESC"}else{"ASC"};
        let clause=if filter.is_empty(){"1".into()}else{filter.join(" AND ")};
        let total=db.query_row(&format!("{SELECT} SELECT COUNT(*) FROM entries WHERE {clause}"),rusqlite::params_from_iter(&args),|r|r.get::<_,i64>(0).map(|v|v as usize))?;
        let records=db.prepare(&format!("{SELECT} SELECT kind,provider_id,name,overview,duration,live,image,position,played,favorite,created FROM entries WHERE {clause} ORDER BY {order} {direction},provider_id LIMIT {limit} OFFSET {start}"))?.query_map(rusqlite::params_from_iter(&args),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,json!({"Name":r.get::<_,String>(2)?,"Overview":r.get::<_,String>(3)?,"RunTimeTicks":catalog::ticks(r.get::<_,f64>(4)?),"IsLive":r.get::<_,bool>(5)?,"DateCreated":r.get::<_,Option<String>>(10)?,"UserData":{"PlaybackPositionTicks":catalog::ticks(r.get::<_,f64>(7)?),"Played":r.get::<_,bool>(8)?,"PlayCount":i32::from(r.get::<_,bool>(8)?),"IsFavorite":r.get::<_,bool>(9)?}}),r.get::<_,String>(6)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut items=Vec::new();
        for (kind,key,mut dto,image) in records {
            let id=mapped(db,&user,&kind,&key)?;
            dto["Id"]=json!(id);dto["ServerId"]=json!(server);dto["Type"]=json!("Video");dto["MediaType"]=json!("Video");dto["IsFolder"]=json!(false);dto["LocationType"]=json!("Remote");dto["CanDownload"]=json!(false);dto["ImageTags"]=json!({});dto["PrimaryImageAspectRatio"]=json!(16.0/9.0);
            dto["ParentId"]=json!(ROOTS.iter().find(|r|r.1==kind).unwrap().0);dto["UserData"]["Key"]=json!(id);dto["UserData"]["ItemId"]=json!(id);
            if list_id.is_some(){dto["PlaylistItemId"]=json!(id);}
            if crate::online::artwork_url(&image).is_some(){dto["ImageTags"]["Primary"]=json!("available");}
            dto["MediaSources"]=json!([source_dto(&dto)]);
            items.push(dto);
        }
        Ok((items,total))
    }).await?;
    if items.iter().any(|i| i["ImageTags"]["Primary"].is_string()) {
        let grant = crate::grants::issue(state, p, "artwork", 300).await?;
        for item in &mut items {
            if item["ImageTags"]["Primary"].is_string() {
                item["ImageTags"]["Primary"] = json!(grant);
            }
        }
    }
    if is_single {
        Ok(Some(
            items.into_iter().next().ok_or_else(ApiError::not_found)?,
        ))
    } else {
        Ok(Some(catalog::result(items, total, start)))
    }
}

pub async fn image(
    state: &AppState,
    p: &Principal,
    identity: &Identity,
    head: bool,
) -> Result<Response> {
    let key = identity.key.clone();
    let kind = identity.kind.clone();
    let user = p.user.id.clone();
    let address=state.db.call(move |db| {
        let sql=match kind.as_str(){"youtube"=>"SELECT 'https://i.ytimg.com/vi/'||video_id||'/mqdefault.jpg' FROM youtube_videos WHERE user_id=?1 AND video_id=?2 AND privacy='public'","twitch"=>"SELECT 'https://static-cdn.jtvnw.net/previews-ttv/live_user_'||login||'-640x360.jpg' FROM twitch_streams WHERE user_id=?1 AND channel_id=?2 AND active=1","kick"=>"SELECT COALESCE(thumbnail_url,profile_image_url,'') FROM kick_channels WHERE user_id=?1 AND slug=?2",_=>return Ok(None)};
        Ok(db.query_row(sql,params![user,key],|r|r.get::<_,String>(0)).optional()?)
    }).await?.ok_or_else(ApiError::not_found)?;
    let (mime, bytes) = crate::online::artwork_bytes(state, &address).await?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=300"),
        ],
        if head { Vec::new() } else { bytes },
    )
        .into_response())
}

pub async fn set_flag(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    favorite: bool,
    value: bool,
) -> Result<()> {
    if item.kind == "youtube" {
        let _ = crate::online::feed::edit_for(
            state,
            p,
            item.key.clone(),
            crate::online::feed::Edit {
                watchlist: None,
                pinned: favorite.then_some(value),
                watched: (!favorite).then_some(value),
            },
        )
        .await?;
    } else if favorite {
        let user = p.user.id.clone();
        let key = item.id.clone();
        state
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE compat_online_items SET favorite=?1 WHERE id=?2 AND user_id=?3",
                    params![value, key, user],
                )?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

pub async fn playlist_create(state: &AppState, p: &Principal, input: Value) -> Result<Value> {
    if input["IsPublic"] == true
        || input["UserId"]
            .as_str()
            .is_some_and(|id| canonical(id) != p.user.id)
        || input["Users"].as_array().into_iter().flatten().any(|u| {
            u["UserId"]
                .as_str()
                .is_none_or(|id| canonical(id) != p.user.id)
        })
    {
        return Err(ApiError::forbidden());
    }
    let ids = input["Ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let videos = playlist_videos(state, p, &ids).await?;
    let name = input["Name"].as_str().unwrap_or_default().to_owned();
    let list = watchlists::create_for(state, p, watchlists::Create { name })
        .await?
        .0
        .as_i64()
        .unwrap();
    for (index, video) in videos.into_iter().enumerate() {
        let _ = watchlists::add_for(
            state,
            p,
            list,
            watchlists::Add {
                video_id: video,
                manual_position: index as f64,
            },
        )
        .await?;
    }
    ensure_lists(state, p).await?;
    let user = p.user.id.clone();
    let id = state
        .db
        .call(move |db| mapped(db, &user, "watchlist", &list.to_string()))
        .await?;
    Ok(json!({"Id":id}))
}
async fn playlist_videos(state: &AppState, p: &Principal, ids: &[String]) -> Result<Vec<String>> {
    if ids.len() > 1000 {
        return Err(ApiError::bad("Playlist exceeds the saved video limit"));
    }
    let mut videos = Vec::new();
    for id in ids {
        let item = resolve(state, p, id)
            .await?
            .ok_or_else(ApiError::not_found)?;
        if item.kind != "youtube" {
            return Err(ApiError::bad(
                "YouTube watchlists accept YouTube videos only",
            ));
        }
        videos.push(item.key);
    }
    Ok(videos)
}
pub async fn playlist_change(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    query: &Query,
    add: bool,
) -> Result<()> {
    let list = item.list()?;
    let ids = query
        .get(if add { "ids" } else { "entryids" })
        .map(String::as_str)
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let videos = playlist_videos(state, p, &ids).await?;
    let user = p.user.id.clone();
    let position=state.db.call(move|db|Ok(db.query_row("SELECT COALESCE(MAX(manual_position),-1)+1 FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2",params![user,list],|r|r.get::<_,f64>(0))?)).await?;
    for (index, video) in videos.into_iter().enumerate() {
        if add {
            let _ = watchlists::add_for(
                state,
                p,
                list,
                watchlists::Add {
                    video_id: video,
                    manual_position: position + index as f64,
                },
            )
            .await?;
        } else {
            let _ = watchlists::remove_for(state, p, list, video).await?;
        }
    }
    Ok(())
}

pub async fn playlist_move(
    state: &AppState,
    p: &Principal,
    list: &Identity,
    entry: &str,
    index: usize,
) -> Result<()> {
    let list = list.list()?;
    let entry = resolve(state, p, entry)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if entry.kind != "youtube" {
        return Err(ApiError::not_found());
    }
    let user = p.user.id.clone();
    let moved = state.db.call(move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let sort = tx.query_row("SELECT sort_mode,sort_direction FROM youtube_watchlists WHERE user_id=?1 AND id=?2",params![user,list],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        let Some((mode,direction)) = sort else { return Ok(false); };
        let order = if mode == "date" { "v.published_at" } else { "i.manual_position" };
        let direction = if direction == "desc" { "DESC" } else { "ASC" };
        let mut ids = tx.prepare(&format!("SELECT i.video_id FROM youtube_watchlist_items i JOIN youtube_videos v ON v.user_id=i.user_id AND v.video_id=i.video_id WHERE i.user_id=?1 AND i.watchlist_id=?2 ORDER BY {order} {direction},i.video_id"))?.query_map(params![user,list],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(old) = ids.iter().position(|id| id == &entry.key) else { return Ok(false); };
        if index >= ids.len() { return Ok(false); }
        let entry = ids.remove(old);
        ids.insert(index,entry);
        for (position,video) in ids.iter().enumerate() {
            tx.execute("UPDATE youtube_watchlist_items SET manual_position=?1 WHERE user_id=?2 AND watchlist_id=?3 AND video_id=?4",params![position as i64,user,list,video])?;
        }
        tx.execute("UPDATE youtube_watchlists SET sort_mode='manual',sort_direction='asc',updated_at=?1 WHERE user_id=?2 AND id=?3",params![thelxinoe_core::now(),user,list])?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if !moved {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(p.user.id.clone()), "youtube.changed", json!({}))
        .await?;
    Ok(())
}

pub async fn sources(state: &AppState, p: &Principal, item: &Identity) -> Result<Vec<Value>> {
    if item.kind == "watchlist" {
        return Ok(Vec::new());
    }
    let dto = browse(state, p, &Query::new(), Some(&item.id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(vec![source_dto(&dto)])
}
fn source_dto(dto: &Value) -> Value {
    let mut source = json!({"Id":dto["Id"],"Name":"Public stream","Protocol":"Http","Type":"Default","IsRemote":true,"Container":"hls","RunTimeTicks":dto["RunTimeTicks"],"Bitrate":8192000,"IsInfiniteStream":dto["IsLive"],"SupportsTranscoding":true,"TranscodingSubProtocol":"hls","DefaultAudioStreamIndex":1,"DefaultSubtitleStreamIndex":-1,"MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264","IsDefault":true},{"Type":"Audio","Index":1,"Codec":"aac","Channels":2,"SampleRate":48000,"IsDefault":true}],"MediaAttachments":[],"Formats":[]});
    for field in [
        "ReadAtNativeFramerate",
        "IgnoreDts",
        "IgnoreIndex",
        "GenPtsInput",
        "RequiresLooping",
        "HasSegments",
        "RequiresOpening",
        "RequiresClosing",
        "SupportsProbing",
        "SupportsDirectPlay",
        "SupportsDirectStream",
    ] {
        source[field] = json!(false);
    }
    for stream in source["MediaStreams"].as_array_mut().unwrap() {
        for field in [
            "IsExternal",
            "IsInterlaced",
            "IsForced",
            "IsHearingImpaired",
            "IsTextSubtitleStream",
            "SupportsExternalStream",
        ] {
            stream[field] = json!(false);
        }
    }
    source
}
pub async fn playback_info(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    query: &Query,
    mut input: Value,
) -> Result<Value> {
    if item.kind == "watchlist" {
        return Err(ApiError::bad("Select a video in this watchlist"));
    }
    if input["UserId"]
        .as_str()
        .is_some_and(|u| canonical(u) != p.user.id)
    {
        return Err(ApiError::forbidden());
    }
    let file = input["MediaSourceId"]
        .as_str()
        .or(query.get("mediasourceid").map(String::as_str))
        .map(canonical);
    let media = item.media();
    if item.kind == "youtube" {
        crate::online::downloads::authorize(state, p, &item.key).await?;
        match crate::online::downloads::source(state, &item.key).await {
            Ok(source) => {
                if file
                    .as_ref()
                    .is_some_and(|f| f != &item.id && f != &source.id)
                {
                    return Err(ApiError::not_found());
                }
                // Standard compatibility HLS gives downloaded videos a complete
                // seekable timeline, with the same progress and retention checks.
                input["EnableDirectPlay"] = json!(false);
                input["EnableDirectStream"] = json!(false);
                return super::playback::info_source(state, p, &media, query, input, source).await;
            }
            Err(e) if e.0 == axum::http::StatusCode::CONFLICT => {}
            Err(e) => return Err(e),
        }
    } else {
        crate::online::live::authorize(state, p, &media).await?;
    }
    if file.as_ref().is_some_and(|f| f != &item.id) {
        return Err(ApiError::not_found());
    }
    if input["EnableTranscoding"] == false {
        return Err(ApiError::bad("Public streams require HLS conversion"));
    }
    let profile = &input["DeviceProfile"];
    let max = [
        input["MaxStreamingBitrate"].as_i64(),
        profile["MaxStreamingBitrate"].as_i64(),
    ]
    .into_iter()
    .flatten()
    .filter(|v| *v > 0)
    .min()
    .unwrap_or(8_192_000);
    let quality = if max >= 8_192_000 {
        "8mbps"
    } else if max >= 4_192_000 {
        "4mbps"
    } else if max >= 2_192_000 {
        "2mbps"
    } else {
        return Err(ApiError::bad(
            "This client bitrate limit is below the supported conversion quality",
        ));
    };
    // Negotiate a bounded output envelope, then fit the actual upstream aspect
    // ratio into it. Source URLs and extractor metadata stay inside the server.
    let envelope = thelxinoe_playback::Source {
        id: String::new(),
        media_id: String::new(),
        generation: String::new(),
        edition: String::new(),
        path: "public.ts".into(),
        root: std::path::PathBuf::new(),
        size: 0,
        modified: String::new(),
        probe: json!({"format":{"duration":"1"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30/1","pix_fmt":"yuv420p","field_order":"progressive"},{"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"}]}),
    };
    let mut capabilities = super::profile::negotiate(
        &envelope,
        None,
        profile,
        false,
        false,
        thelxinoe_playback::bitrate(quality)?.unwrap(),
    );
    let conversion = capabilities
        .conversion
        .as_mut()
        .ok_or_else(|| ApiError::bad("The client does not support H.264/AAC HLS playback"))?;
    conversion.fit = true;
    capabilities.video = vec!["h264".into()];
    capabilities.audio = vec!["aac".into()];
    let options = thelxinoe_playback::Options {
        quality: quality.into(),
        audio: None,
        subtitle: Some("off".into()),
        capabilities,
    };
    let mut dto = sources(state, p, item).await?.remove(0);
    let playback = crate::playback::create_with_delivery(
        state,
        p,
        crate::playback::Create {
            queue: None,
            media_id: media,
            file_id: None,
            position: Some(0.0),
            options,
        },
        true,
    )
    .await?;
    let sid = playback["id"].as_str().unwrap().to_owned();
    let key = sid.clone();
    state
        .db
        .call(move |db| {
            db.execute(
                "INSERT INTO compat_playbacks(playback_id) VALUES(?1)",
                [key],
            )?;
            Ok(())
        })
        .await?;
    dto["TranscodingUrl"] = playback["url"].clone();
    dto["TranscodingContainer"] = json!("ts");
    dto["ETag"] = playback["grant"].clone();
    dto["RunTimeTicks"] = json!(catalog::ticks(playback["duration"].as_f64().unwrap_or(0.0)));
    dto["IsInfiniteStream"] = playback["live"].clone();
    Ok(json!({"MediaSources":[dto],"PlaySessionId":sid}))
}
