//! Database operations for managers.metadata.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn service_for_kind(
    kind: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("managers.metadata.service_for_kind", move |db| {
        Ok(db
            .query_row(
                "SELECT id FROM manager_services WHERE enabled=1 AND kind=?1",
                [kind],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn match_item(db: &Database, target: String) -> anyhow::Result<Option<String>> {
    db.read("managers.metadata.match_item", move |db| {
        Ok(db
            .query_row("SELECT kind FROM media WHERE id=?1", [target], |r| {
                r.get::<_, String>(0)
            })
            .optional()?)
    })
    .await
}

pub(super) async fn refresh(db: &Database, target: String) -> anyhow::Result<Option<String>> {
    db.read("managers.metadata.refresh", move |db| {
        Ok(db
            .query_row("SELECT kind FROM media WHERE id=?1", [target], |r| {
                r.get::<_, String>(0)
            })
            .optional()?)
    })
    .await
}

pub(super) async fn stored_binding(
    media: String,
    db: &Database,
) -> anyhow::Result<Option<Binding>> {
    db
        .read("managers.metadata.stored_binding", move |db| {
            Ok(db
                .query_row(
                    "SELECT b.service_id,b.external_id,b.manager_entity_id FROM metadata_bindings b JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1",
                    [media],
                    |r| Ok(Binding { service_id:r.get(0)?, external_id:r.get(1)?, entity_id:r.get(2)? }),
                )
                .optional()?)
        })
        .await
}

pub(super) async fn inferred_binding(
    media: String,
    manager: String,
    db: &Database,
) -> anyhow::Result<Option<Binding>> {
    db.read("managers.metadata.inferred_binding", move |db| {
        let rows=db.prepare("WITH RECURSIVE tree(id) AS (SELECT ?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT DISTINCT b.service_id,b.external_id,b.entity_id FROM tree JOIN media_sources ms ON ms.media_id=tree.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind=?2 AND b.service_generation=s.generation WHERE b.checked_at>=?3 ORDER BY b.service_id,b.external_id,b.entity_id")?.query_map(params![media,manager,now()-60],|r|Ok(Binding{service_id:r.get(0)?,external_id:r.get(1)?,entity_id:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(if rows.len()==1 { rows.into_iter().next() } else { None })
    }).await
}

pub(super) async fn ancestor(
    media: String,
    target_kind: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read("managers.metadata.ancestor", move |db| {
        Ok(db.query_row("WITH RECURSIVE parents(id,parent_id,kind) AS (SELECT id,parent_id,kind FROM media WHERE id=?1 UNION ALL SELECT m.id,m.parent_id,m.kind FROM media m JOIN parents p ON p.parent_id=m.id) SELECT id FROM parents WHERE kind=?2 LIMIT 1",params![media,target_kind],|r|r.get::<_,String>(0)).optional()?)
    }).await
}

pub(super) async fn update_metadata_read_metadata_bindings(
    identity_media: String,
    identity_service: String,
    identity_generation: String,
    identity_external: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("managers.metadata.update_metadata_read_metadata_bindings", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata_bindings WHERE media_id=?1 AND service_id=?2 AND service_generation=?3 AND external_id=?4)",
            params![identity_media,identity_service,identity_generation,identity_external],
            |r| r.get::<_,bool>(0),
        )?)
    }).await
}

pub(super) struct MetadataUpdate {
    pub(super) refreshed_at: i64,
    pub(super) media: String,
    pub(super) service_id: String,
    pub(super) generation: String,
    pub(super) external: String,
    pub(super) manager_entity_id: Option<i64>,
    pub(super) stored: String,
    pub(super) actor: Option<String>,
}

pub(super) async fn update_metadata_write_media(
    db: &Database,
    input: MetadataUpdate,
) -> anyhow::Result<()> {
    let MetadataUpdate {
        refreshed_at,
        media,
        service_id,
        generation,
        external,
        manager_entity_id,
        stored,
        actor,
    } = input;

    db.write("managers.metadata.update_metadata_write_media", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE media SET metadata=?1 WHERE id=?2",params![stored,media])?;
        tx.execute("INSERT INTO metadata_bindings(media_id,service_id,service_generation,external_id,manager_entity_id,refreshed_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(media_id) DO UPDATE SET service_id=excluded.service_id,service_generation=excluded.service_generation,external_id=excluded.external_id,manager_entity_id=excluded.manager_entity_id,refreshed_at=excluded.refreshed_at",params![media,service_id,generation,external,manager_entity_id,refreshed_at])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'metadata.refresh',?2,?3)",params![actor,media,now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn update_show_read_metadata_bindings(
    previous_media: String,
    current_service: String,
    current_generation: String,
    current_external: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("managers.metadata.update_show_read_metadata_bindings", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata_bindings WHERE media_id=?1 AND (service_id<>?2 OR service_generation<>?3 OR external_id<>?4))",
            params![previous_media,current_service,current_generation,current_external],
            |r| r.get::<_,bool>(0),
        )?)
    }).await
}

pub(super) async fn update_show_write_manager_episode_mappings(
    changed_show: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.metadata.update_show_write_manager_episode_mappings", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM manager_episode_mappings WHERE media_id IN (SELECT ep.id FROM media ep JOIN media season ON season.id=ep.parent_id WHERE season.parent_id=?1)",[&changed_show])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn update_show_write_manager_episodes(
    refreshed_at: i64,
    service_id: String,
    service_generation: String,
    series_external: String,
    show_id: String,
    saved: Vec<(i64, i64, i64, String)>,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.metadata.update_show_write_manager_episodes", move |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (episode,season,number,metadata) in saved {
            tx.execute("INSERT INTO manager_episodes(service_id,service_generation,series_external_id,manager_episode_id,season_number,episode_number,metadata,refreshed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(service_id,service_generation,manager_episode_id) DO UPDATE SET series_external_id=excluded.series_external_id,season_number=excluded.season_number,episode_number=excluded.episode_number,metadata=excluded.metadata,refreshed_at=excluded.refreshed_at",params![service_id,service_generation,series_external,episode,season,number,metadata,refreshed_at])?;
        }
        tx.execute("UPDATE manager_episode_mappings SET state='unresolved' WHERE service_id=?1 AND service_generation=?2 AND manager_episode_id IN (SELECT manager_episode_id FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND series_external_id=?3 AND refreshed_at<>?4)",params![service_id,service_generation,series_external,refreshed_at])?;
        let rows=tx.prepare("SELECT ep.id,b.members FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media_sources ms ON ms.media_id=ep.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE ep.kind='episode' AND season.parent_id=?1 AND b.service_id=?2 AND b.service_generation=?3 AND b.checked_at>=?4")?.query_map(params![show_id,service_id,service_generation,now()-60],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut exact=BTreeMap::<String,BTreeSet<i64>>::new();
        for (episode,members) in rows {
            for member in serde_json::from_str::<Vec<i64>>(&members).unwrap_or_default() { exact.entry(episode.clone()).or_default().insert(member); }
        }
        for (episode,members) in exact {
            if members.len()!=1 || tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episode_mappings WHERE media_id=?1)",[&episode],|r|r.get::<_,bool>(0))? {continue;}
            let member=*members.first().unwrap();
            let current=tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND series_external_id=?3 AND manager_episode_id=?4 AND refreshed_at=?5)",params![service_id,service_generation,series_external,member,refreshed_at],|r|r.get::<_,bool>(0))?;
            if current {tx.execute("INSERT INTO manager_episode_mappings VALUES (?1,?2,?3,?4,'confirmed')",params![episode,service_id,service_generation,member])?;}
        }
        tx.execute("UPDATE manager_episode_mappings SET state=CASE WHEN media_id IN (SELECT media_id FROM manager_episode_mappings GROUP BY media_id HAVING COUNT(*)>1) OR (service_id,service_generation,manager_episode_id) IN (SELECT service_id,service_generation,manager_episode_id FROM manager_episode_mappings GROUP BY service_id,service_generation,manager_episode_id HAVING COUNT(*)>1) THEN 'complex' ELSE 'confirmed' END WHERE service_id=?1 AND service_generation=?2 AND state<>'unresolved'",params![service_id,service_generation])?;
        tx.execute("UPDATE media SET metadata=COALESCE((SELECT e.metadata FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id AND b.refreshed_at=e.refreshed_at WHERE m.media_id=media.id AND m.state='confirmed'),'{}') WHERE kind='episode' AND parent_id IN (SELECT id FROM media WHERE parent_id=?1)",[show_id])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn update_album_tracks_write_media(
    album: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write(
        "managers.metadata.update_album_tracks_write_media",
        move |db| {
            db.execute(
                "UPDATE media SET metadata='{}' WHERE kind='track' AND parent_id=?1",
                [album],
            )?;
            Ok(())
        },
    )
    .await
}

pub(super) async fn update_album_tracks_read_media(
    album: String,
    service_id: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<Vec<(String, i64)>> {
    db.read("managers.metadata.update_album_tracks_read_media", move|db|{
        let rows=db.prepare("SELECT track.id,b.members FROM media track JOIN media_sources ms ON ms.media_id=track.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE track.kind='track' AND track.parent_id=?1 AND b.service_id=?2 AND b.service_generation=?3 AND b.checked_at>=?4")?.query_map(params![album,service_id,generation,now()-60],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut exact=BTreeMap::<String,BTreeSet<i64>>::new();
        for (track,members) in rows {for member in serde_json::from_str::<Vec<i64>>(&members).unwrap_or_default(){exact.entry(track.clone()).or_default().insert(member);}}
        Ok(exact.into_iter().filter_map(|(track,members)|(members.len()==1).then(||(track,*members.first().unwrap()))).collect::<Vec<_>>())
    }).await
}

pub(super) async fn replace_track_metadata(
    updates: Vec<(String, String)>,
    album: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.metadata.replace_track_metadata", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE media SET metadata='{}' WHERE kind='track' AND parent_id=?1",
            [&album],
        )?;
        for (media, metadata) in updates {
            tx.execute(
                "UPDATE media SET metadata=?1 WHERE id=?2",
                params![metadata, media],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn enqueue_managed_refreshes(
    cutoff: i64,
    freshness: i64,
    db: &Database,
) -> anyhow::Result<Vec<(String, String, String, String, String)>> {
    db.read("managers.metadata.enqueue_managed_refreshes", move|db|{
        let sql="WITH raw(media_id,kind,service_id,service_generation,external_id) AS (
          SELECT m.id,'movie',b.service_id,b.service_generation,b.external_id FROM media m JOIN media_sources ms ON ms.media_id=m.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='radarr' AND s.generation=b.service_generation WHERE m.kind='movie' AND b.checked_at>=?1
          UNION ALL SELECT show.id,'show',b.service_id,b.service_generation,b.external_id FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media show ON show.id=season.parent_id JOIN media_sources ms ON ms.media_id=ep.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='sonarr' AND s.generation=b.service_generation WHERE ep.kind='episode' AND b.checked_at>=?1
          UNION ALL SELECT album.id,'album',b.service_id,b.service_generation,b.external_id FROM media track JOIN media album ON album.id=track.parent_id JOIN media_sources ms ON ms.media_id=track.id JOIN media_files f ON f.id=ms.file_id AND f.present=1 AND f.ownership='managed' JOIN manager_bindings b ON b.file_id=f.id AND b.generation=f.generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.kind='lidarr' AND s.generation=b.service_generation WHERE track.kind='track' AND b.checked_at>=?1
        ), targets AS (
          SELECT media_id,kind,min(service_id) service_id,min(service_generation) service_generation,min(external_id) external_id,count(DISTINCT service_id||char(31)||service_generation||char(31)||external_id) variants FROM raw GROUP BY media_id,kind
        ) SELECT t.media_id,t.kind,t.service_id,t.service_generation,t.external_id FROM targets t LEFT JOIN metadata_bindings m ON m.media_id=t.media_id WHERE t.variants=1 AND (m.media_id IS NULL OR m.service_id<>t.service_id OR m.service_generation<>t.service_generation OR m.external_id<>t.external_id OR m.refreshed_at<?2) ORDER BY t.kind,t.media_id";
        Ok(db.prepare(sql)?.query_map(params![freshness,cutoff],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

pub(super) async fn manager_episodes(db: &Database, media_id: String) -> anyhow::Result<Value> {
    db.read("managers.metadata.manager_episodes", move|db|{
        let show:String=db.query_row("SELECT CASE WHEN m.kind='episode' THEN season.parent_id WHEN m.kind='season' THEN m.parent_id ELSE m.id END FROM media m LEFT JOIN media season ON season.id=m.parent_id WHERE m.id=?1",[&media_id],|r|r.get(0))?;
        let items=db.prepare("SELECT e.manager_episode_id,e.season_number,e.episode_number,e.metadata FROM manager_episodes e JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id AND b.refreshed_at=e.refreshed_at JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1 ORDER BY e.season_number,e.episode_number")?.query_map([&show],|r|Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"season":r.get::<_,i64>(1)?,"episode":r.get::<_,i64>(2)?,"metadata":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mappings=db.prepare("SELECT m.manager_episode_id,m.state FROM manager_episode_mappings m JOIN metadata_bindings b ON b.service_id=m.service_id AND b.service_generation=m.service_generation JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE m.media_id=?1 AND b.media_id=?2")?.query_map(params![media_id,show],|r|Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"state":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"items":items,"mappings":mappings}))
    }).await
}

pub(super) async fn map_episode(
    db: &Database,
    media_id: String,
    principal: thelxinoe_core::Principal,
    ids: Vec<i64>,
) -> anyhow::Result<bool> {
    db.write("managers.metadata.map_episode", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let show:Option<String>=tx.query_row("SELECT show.id FROM media ep JOIN media season ON season.id=ep.parent_id JOIN media show ON show.id=season.parent_id WHERE ep.id=?1 AND ep.kind='episode'",[&media_id],|r|r.get(0)).optional()?;
        let Some(show)=show else{return Ok(false);};
        let binding:Option<(String,String,String,i64)>=tx.query_row("SELECT b.service_id,b.service_generation,b.external_id,b.refreshed_at FROM metadata_bindings b JOIN manager_services s ON s.id=b.service_id AND s.enabled=1 AND s.generation=b.service_generation WHERE b.media_id=?1",[&show],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((service,generation,external,refreshed))=binding else{return Ok(false);};
        for id in &ids {let belongs:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_episodes WHERE service_id=?1 AND service_generation=?2 AND manager_episode_id=?3 AND series_external_id=?4 AND refreshed_at=?5)",params![service,generation,id,external,refreshed],|r|r.get(0))?;if !belongs{return Ok(false);}}
        tx.execute("DELETE FROM manager_episode_mappings WHERE media_id=?1",[&media_id])?;
        for id in &ids{tx.execute("INSERT INTO manager_episode_mappings VALUES (?1,?2,?3,?4,?5)",params![media_id,service,generation,id,if ids.len()==1{"confirmed"}else{"complex"}])?;}
        tx.execute("UPDATE manager_episode_mappings SET state=CASE WHEN media_id IN (SELECT media_id FROM manager_episode_mappings GROUP BY media_id HAVING COUNT(*)>1) OR (service_id,service_generation,manager_episode_id) IN (SELECT service_id,service_generation,manager_episode_id FROM manager_episode_mappings GROUP BY service_id,service_generation,manager_episode_id HAVING COUNT(*)>1) THEN 'complex' ELSE 'confirmed' END WHERE state<>'unresolved'",[])?;
        tx.execute("UPDATE media SET metadata=COALESCE((SELECT e.metadata FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id WHERE m.media_id=media.id AND m.state='confirmed'),'{}') WHERE id=?1",[&media_id])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'episode.map',?2,?3)",params![principal.user.id,media_id,now()])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn collections(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.metadata.collections", |db|Ok(db.prepare("SELECT json_extract(metadata,'$.belongs_to_collection.id'),json_extract(metadata,'$.belongs_to_collection.name'),count(*) FROM media WHERE kind='movie' AND json_extract(metadata,'$.belongs_to_collection.id') IS NOT NULL GROUP BY json_extract(metadata,'$.belongs_to_collection.id')")?.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"count":r.get::<_,i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
