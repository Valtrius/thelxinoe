//! Database operations for managers.bindings.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn reconcile_read_manager_services(db: &Database) -> anyhow::Result<Vec<String>> {
    db.read("managers.bindings.reconcile_read_manager_services", |db| {
        Ok(db
            .prepare("SELECT id FROM manager_services WHERE enabled=1 ORDER BY id")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

pub(super) async fn reconcile_write_media_files(
    key: String,
    result: std::prelude::v1::Result<Vec<Claim>, ApiError>,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.bindings.reconcile_write_media_files", move|db| {
        let tx=db.transaction()?;
        match result {
            Ok(claims)=> {
                let mut current=Vec::new();
                for claim in claims {
                    let file=tx.query_row("SELECT id,generation FROM media_files WHERE path=?1 AND present=1",[&claim.server_path],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
                    if let Some((file,generation))=file {
                        current.push(file.clone());
                        tx.execute("INSERT INTO manager_bindings VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(file_id,service_id) DO UPDATE SET generation=excluded.generation,service_generation=excluded.service_generation,manager_file_id=excluded.manager_file_id,entity_id=excluded.entity_id,manager_path=excluded.manager_path,external_id=excluded.external_id,members=excluded.members,checked_at=excluded.checked_at",params![file,claim.service_id,generation,claim.service_generation,claim.manager_file_id,claim.entity_id,claim.manager_path,claim.external_id,serde_json::to_string(&claim.members)?,now()])?;
                    }
                }
                // Retain old rows as historical evidence, but make freshness explicit.
                tx.execute("UPDATE manager_bindings SET checked_at=0 WHERE service_id=?1 AND file_id NOT IN (SELECT value FROM json_each(?2))",params![key,serde_json::to_string(&current)?])?;
                tx.execute("INSERT INTO manager_reconciliations VALUES (?1,?2,?3,NULL) ON CONFLICT(service_id) DO UPDATE SET generation=excluded.generation,checked_at=excluded.checked_at,error=NULL",params![key,generation,now()])?;
            },
            Err(error)=> {tx.execute("INSERT INTO manager_reconciliations VALUES (?1,?2,?3,?4) ON CONFLICT(service_id) DO UPDATE SET generation=excluded.generation,checked_at=excluded.checked_at,error=excluded.error",params![key,generation,now(),error.2])?;}
        }
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn refresh_ownership(db: &Database) -> anyhow::Result<()> {
    db.write("managers.bindings.refresh_ownership", |db| {
        db.execute("UPDATE media_files SET ownership=CASE
          WHEN EXISTS(SELECT 1 FROM manager_services s LEFT JOIN manager_reconciliations r ON r.service_id=s.id JOIN library_roots l ON l.id=media_files.root_id WHERE s.enabled=1 AND s.kind=CASE l.kind WHEN 'movies' THEN 'radarr' WHEN 'shows' THEN 'sonarr' ELSE 'lidarr' END AND (r.error IS NOT NULL OR r.checked_at IS NULL OR r.checked_at<?1 OR r.generation<>s.generation)) THEN 'unresolved'
          WHEN EXISTS(SELECT 1 FROM manager_bindings b JOIN manager_services s ON s.id=b.service_id WHERE b.file_id=media_files.id AND b.generation=media_files.generation AND b.service_generation=s.generation AND b.checked_at>=?1 AND s.enabled=1) THEN 'managed'
          WHEN EXISTS(SELECT 1 FROM manager_bindings b WHERE b.file_id=media_files.id) THEN 'unresolved'
          ELSE 'unmanaged' END WHERE present=1",[now()-60])?;
        Ok(())
    }).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.bindings.list", |db|Ok(db.prepare("SELECT f.id,f.path,f.generation,f.ownership,COALESCE(json_group_array(json_object('service',s.name,'entity_id',b.entity_id,'file_id',b.manager_file_id,'members',json(b.members),'checked_at',b.checked_at)) FILTER (WHERE b.service_id IS NOT NULL),'[]') FROM media_files f LEFT JOIN manager_bindings b ON b.file_id=f.id LEFT JOIN manager_services s ON s.id=b.service_id WHERE f.present=1 GROUP BY f.id ORDER BY f.path LIMIT 1000")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?,"generation":r.get::<_,String>(2)?,"ownership":r.get::<_,String>(3)?,"bindings":serde_json::from_str::<Value>(&r.get::<_,String>(4)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
