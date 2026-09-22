//! Database operations for jellyfin.catalog.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn views(server: String, db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("jellyfin.catalog.views", move |db| {
        let mut items = db.prepare("SELECT id,name,kind FROM library_roots ORDER BY name")?.query_map([], |r| {
            let domain: String = r.get(2)?;
            Ok(json!({"Id":r.get::<_,String>(0)?,"Name":r.get::<_,String>(1)?,"Type":"CollectionFolder","IsFolder":true,"CollectionType":if domain=="shows" {"tvshows"}else{&domain},"ServerId":server,"ImageTags":{}}))
        })?.collect::<std::result::Result<Vec<_>,_>>()?;
        items.push(json!({"Id":PLAYLIST_VIEW,"Name":"Playlists","Type":"CollectionFolder","IsFolder":true,"CollectionType":"playlists","ServerId":server,"ImageTags":{}}));
        Ok(items)
    }).await
}

pub(super) struct MediaQuery {
    pub(super) q: std::collections::BTreeMap<String, String>,
    pub(super) uid: String,
    pub(super) server: String,
    pub(super) id: Option<String>,
    pub(super) parent: Option<String>,
    pub(super) start: usize,
    pub(super) limit: usize,
}

pub(super) async fn browse_media(
    db: &Database,
    input: MediaQuery,
) -> anyhow::Result<(Vec<Value>, usize)> {
    let MediaQuery {
        q,
        uid,
        server,
        id,
        parent,
        start,
        limit,
    } = input;

    db.read("jellyfin.catalog.browse_media", move|db| {
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
    }).await
}

pub(super) async fn is_playlist(id: String, db: &Database) -> anyhow::Result<bool> {
    db.read("jellyfin.catalog.is_playlist", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?)
    })
    .await
}

pub(super) async fn playlist_items(
    id: String,
    db: &Database,
) -> anyhow::Result<Option<(i64, Vec<String>)>> {
    db.read("jellyfin.catalog.playlist_items", move |db| {
        let revision = db
            .query_row("SELECT revision FROM playlists WHERE id=?1", [&id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?;
        let Some(revision) = revision else {
            return Ok(None);
        };
        let tracks = db
            .prepare("SELECT media_id FROM playlist_items WHERE playlist_id=?1 ORDER BY position")?
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some((revision, tracks)))
    })
    .await
}

pub(super) struct PlaylistQuery {
    pub(super) user: String,
    pub(super) id: Option<String>,
    pub(super) server: String,
    pub(super) favorite: bool,
    pub(super) start: u32,
    pub(super) limit: u32,
    pub(super) search: Option<String>,
    pub(super) media: Option<String>,
    pub(super) order: &'static str,
    pub(super) direction: &'static str,
}

pub(super) async fn playlists(
    db: &Database,
    input: PlaylistQuery,
) -> anyhow::Result<(Vec<Value>, usize)> {
    let PlaylistQuery {
        user,
        id,
        server,
        favorite,
        start,
        limit,
        search,
        media,
        order,
        direction,
    } = input;

    db.read("jellyfin.catalog.playlists", move|db|{
        let source="WITH lists AS (
          SELECT p.id,p.name,p.description,'Audio' media_type,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.playlist_id=p.id AND f.user_id=?1) favorite,
          (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) children,0 updated_at
          FROM playlists p
          UNION ALL
          SELECT c.id,w.name,CASE WHEN w.auto_download=1 THEN 'YouTube watchlist · Automatic server downloads enabled' ELSE 'YouTube watchlist' END,'Video',c.favorite,
          (SELECT COUNT(*) FROM youtube_watchlist_items i WHERE i.watchlist_id=w.id),w.updated_at
          FROM youtube_watchlists w JOIN compat_online_items c ON c.user_id=w.user_id AND c.watchlist_id=w.id WHERE w.user_id=?1)";
        let filter="(?2 IS NULL OR id=?2) AND (?3=0 OR favorite=1) AND (?4 IS NULL OR instr(lower(name),lower(?4))>0) AND (?5 IS NULL OR instr(lower(?5),lower(media_type))>0)";
        let total=db.query_row(&format!("{source} SELECT COUNT(*) FROM lists WHERE {filter}"),params![user,id,favorite,search,media],|r|r.get::<_,i64>(0).map(|v|v as usize))?;
        let items=db.prepare(&format!("{source} SELECT id,name,description,media_type,favorite,children FROM lists WHERE {filter} ORDER BY {order} {direction},id LIMIT ?6 OFFSET ?7"))?.query_map(params![user,id,favorite,search,media,limit,start],|r|Ok(json!({"Id":r.get::<_,String>(0)?,"Name":r.get::<_,String>(1)?,"Overview":r.get::<_,String>(2)?,"Type":"Playlist","MediaType":r.get::<_,String>(3)?,"IsFolder":true,"LocationType":"Remote","ServerId":server,"ChildCount":r.get::<_,i64>(5)?,"ImageTags":{},"UserData":{"Key":r.get::<_,String>(0)?,"ItemId":r.get::<_,String>(0)?,"IsFavorite":r.get::<_,bool>(4)?,"PlayCount":0,"Played":false,"PlaybackPositionTicks":0}})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((items,total))
    }).await
}

pub(super) async fn display_preferences(
    uid: String,
    id: String,
    client: String,
    db: &Database,
    body: Option<Value>,
) -> anyhow::Result<Option<Value>> {
    db.write("jellyfin.catalog.display_preferences", move|db|{
        if let Some(value)=body {
            let inserted=db.execute("INSERT INTO compat_preferences SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM compat_preferences WHERE user_id=?1 AND client=?2 AND preference_id=?3) OR (SELECT COUNT(*) FROM compat_preferences WHERE user_id=?1)<100 ON CONFLICT(user_id,client,preference_id) DO UPDATE SET value=excluded.value",params![uid,client,id,value.to_string()])?;
            if inserted==0 {return Ok(None);}
        }
        Ok(Some(db.query_row("SELECT value FROM compat_preferences WHERE user_id=?1 AND client=?2 AND preference_id=?3",params![uid,client,id],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str(&v).ok()).unwrap_or_else(||json!({"Id":id,"Client":client,"SortBy":"SortName","SortOrder":"Ascending","RememberIndexing":false,"PrimaryImageHeight":250,"PrimaryImageWidth":250,"CustomPrefs":{},"ScrollDirection":"Horizontal","ShowBackdrop":true,"RememberSorting":false,"ShowSidebar":false}))))
    }).await
}

pub(super) const SELECT: &str = "SELECT m.id,m.kind,c.title,m.parent_id,m.year,m.sort_number,m.metadata,m.overrides,m.created_at,m.root_id,COALESCE(u.favorite,0),COALESCE(u.watched,0),COALESCE(ep.position,0),COALESCE(ep.duration,0),p.kind,p.sort_number,pc.title,p.parent_id,gc.title,(SELECT f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1 ORDER BY f.edition,f.id LIMIT 1),(SELECT COUNT(*) FROM media ch WHERE ch.parent_id=m.id),COUNT(*) OVER() FROM media m JOIN media_cards c ON c.id=m.id LEFT JOIN media_state u ON u.media_id=m.id AND u.user_id=?1 LEFT JOIN edition_progress ep ON ep.rowid=(SELECT rowid FROM edition_progress WHERE media_id=m.id AND user_id=?1 ORDER BY updated_at DESC,edition LIMIT 1) LEFT JOIN media p ON p.id=m.parent_id LEFT JOIN media_cards pc ON pc.id=p.id LEFT JOIN media_cards gc ON gc.id=p.parent_id";

pub(super) fn row(r: &rusqlite::Row<'_>, server: &str) -> rusqlite::Result<Value> {
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
