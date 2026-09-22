//! Database operations for jellyfin.online.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn resolve(
    item: String,
    user: String,
    db: &Database,
) -> anyhow::Result<Option<Identity>> {
    db.read("jellyfin.online.resolve", move |db| Ok(db.query_row("SELECT id,CASE WHEN youtube_video_id IS NOT NULL THEN 'youtube' WHEN twitch_channel_id IS NOT NULL THEN 'twitch' WHEN kick_slug IS NOT NULL THEN 'kick' ELSE 'watchlist' END,CAST(COALESCE(youtube_video_id,twitch_channel_id,kick_slug,watchlist_id) AS TEXT) FROM compat_online_items WHERE id=?1 AND user_id=?2", params![item,user], |r| Ok(Identity{id:r.get(0)?,kind:r.get(1)?,key:r.get(2)?})).optional()?)).await
}

pub(super) async fn ensure_lists(user: String, db: &Database) -> anyhow::Result<()> {
    db.write("jellyfin.online.ensure_lists", move |db| {
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
    .await
}

pub(super) struct BrowseQuery {
    pub(super) single: Option<Identity>,
    pub(super) provider: Option<&'static str>,
    pub(super) mapped_ids: Vec<Identity>,
    pub(super) list_id: Option<i64>,
    pub(super) q: std::collections::BTreeMap<String, String>,
    pub(super) user: String,
    pub(super) server: String,
    pub(super) start: usize,
    pub(super) limit: usize,
}

pub(super) async fn browse(
    db: &Database,
    input: BrowseQuery,
) -> anyhow::Result<(Vec<Value>, usize)> {
    let BrowseQuery {
        single,
        provider,
        mapped_ids,
        list_id,
        q,
        user,
        server,
        start,
        limit,
    } = input;

    db.write("jellyfin.online.browse", move |db| {
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
    }).await
}

pub(super) async fn image(
    key: String,
    kind: String,
    user: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("jellyfin.online.image", move |db| {
        let sql=match kind.as_str(){"youtube"=>"SELECT 'https://i.ytimg.com/vi/'||video_id||'/mqdefault.jpg' FROM youtube_videos WHERE user_id=?1 AND video_id=?2 AND privacy='public'","twitch"=>"SELECT 'https://static-cdn.jtvnw.net/previews-ttv/live_user_'||login||'-640x360.jpg' FROM twitch_streams WHERE user_id=?1 AND channel_id=?2 AND active=1","kick"=>"SELECT COALESCE(thumbnail_url,profile_image_url,'') FROM kick_channels WHERE user_id=?1 AND slug=?2",_=>return Ok(None)};
        Ok(db.query_row(sql,params![user,key],|r|r.get::<_,String>(0)).optional()?)
    }).await
}

pub(super) async fn set_flag(
    user: String,
    key: String,
    db: &Database,
    value: bool,
) -> anyhow::Result<()> {
    db.write("jellyfin.online.set_flag", move |db| {
        db.execute(
            "UPDATE compat_online_items SET favorite=?1 WHERE id=?2 AND user_id=?3",
            params![value, key, user],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn playlist_create(
    list: i64,
    user: String,
    db: &Database,
) -> anyhow::Result<String> {
    db.write("jellyfin.online.playlist_create", move |db| {
        mapped(db, &user, "watchlist", &list.to_string())
    })
    .await
}

pub(super) async fn playlist_change(list: i64, user: String, db: &Database) -> anyhow::Result<f64> {
    db.read("jellyfin.online.playlist_change", move|db|Ok(db.query_row("SELECT COALESCE(MAX(manual_position),-1)+1 FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2",params![user,list],|r|r.get::<_,f64>(0))?)).await
}

pub(super) async fn playlist_move(
    list: i64,
    entry: Identity,
    user: String,
    db: &Database,
    index: usize,
) -> anyhow::Result<bool> {
    db.write("jellyfin.online.playlist_move", move |db| {
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
    }).await
}

pub(super) async fn playback_info(key: String, db: &Database) -> anyhow::Result<()> {
    db.write("jellyfin.online.playback_info", move |db| {
        db.execute(
            "INSERT INTO compat_playbacks(playback_id) VALUES(?1)",
            [key],
        )?;
        Ok(())
    })
    .await
}

pub(super) fn mapped(db: &Connection, user: &str, kind: &str, key: &str) -> anyhow::Result<String> {
    let column = column(kind);
    db.execute(&format!("INSERT INTO compat_online_items(id,user_id,{column}) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING"), params![id(), user, key])?;
    Ok(db.query_row(
        &format!("SELECT id FROM compat_online_items WHERE user_id=?1 AND {column}=?2"),
        params![user, key],
        |r| r.get(0),
    )?)
}

pub(super) const SELECT: &str = "WITH entries AS (

 SELECT 'youtube' kind,v.video_id provider_id,v.title name,v.channel_title overview,COALESCE(v.duration,0) duration,v.broadcast='live' live,

 'https://i.ytimg.com/vi/'||v.video_id||'/mqdefault.jpg' image,COALESCE(s.position,0) position,COALESCE(s.watched,0) played,COALESCE(s.pinned,0) favorite,strftime('%Y-%m-%dT%H:%M:%SZ',v.published_at,'unixepoch') created

 FROM youtube_videos v LEFT JOIN youtube_video_state s ON s.user_id=v.user_id AND s.video_id=v.video_id

 WHERE v.user_id=?1 AND v.privacy='public' AND v.available=1 AND v.broadcast<>'upcoming'

 AND (EXISTS(SELECT 1 FROM youtube_subscriptions sub WHERE sub.user_id=v.user_id AND sub.channel_id=v.channel_id AND sub.active=1) OR COALESCE(s.watchlist,0)=1 OR COALESCE(s.pinned,0)=1)

 UNION ALL

 SELECT 'twitch',t.channel_id,t.display_name,t.title||' · '||t.category,0,1,'https://static-cdn.jtvnw.net/previews-ttv/live_user_'||t.login||'-640x360.jpg',0,0,COALESCE(c.favorite,0),t.started_at

 FROM twitch_streams t LEFT JOIN compat_online_items c ON c.user_id=t.user_id AND c.twitch_channel_id=t.channel_id WHERE t.user_id=?1 AND t.active=1

 UNION ALL

 SELECT 'kick',k.slug,COALESCE(NULLIF(k.display_name,''),k.slug),k.title||' · '||k.category,0,1,COALESCE(k.thumbnail_url,k.profile_image_url,''),0,0,COALESCE(c.favorite,0),k.started_at

 FROM kick_channels k LEFT JOIN compat_online_items c ON c.user_id=k.user_id AND c.kick_slug=k.slug WHERE k.user_id=?1 AND k.live=1

 )";
