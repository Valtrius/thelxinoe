//! Database operations for managers.operations.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn targets(media: String, db: &Database) -> anyhow::Result<Vec<Target>> {
    db.read("managers.operations.targets", move|db|{
        let files=db.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT DISTINCT f.id,f.generation,f.path,l.path,f.size,f.modified,f.fingerprint,f.ownership FROM tree JOIN media_sources ms ON ms.media_id=tree.id JOIN media_files f ON f.id=ms.file_id JOIN library_roots l ON l.id=f.root_id WHERE f.present=1 ORDER BY f.id")?.query_map([&media],|r|Ok(Target{id:r.get(0)?,generation:r.get(1)?,path:r.get(2)?,root:r.get(3)?,size:r.get::<_,i64>(4)? as u64,modified:r.get(5)?,fingerprint:r.get(6)?,ownership:r.get(7)?,claims:vec![]}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut output=Vec::new();
        for mut file in files {
            let outside:bool=db.query_row("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT EXISTS(SELECT 1 FROM media_sources WHERE file_id=?2 AND media_id NOT IN (SELECT id FROM tree))",params![media,file.id],|r|r.get(0))?;
            anyhow::ensure!(!outside,"A file also belongs to media outside this target; choose the complete logical unit");
            file.claims=db.prepare("SELECT service_id,service_generation,manager_file_id,entity_id,manager_path,external_id,members FROM manager_bindings WHERE file_id=?1 AND generation=?2 ORDER BY service_id")?.query_map(params![file.id,file.generation],|r|Ok(bindings::Claim{service_id:r.get(0)?,service_generation:r.get(1)?,manager_file_id:r.get(2)?,entity_id:r.get(3)?,manager_path:r.get(4)?,external_id:r.get(5)?,members:serde_json::from_str(&r.get::<_,String>(6)?).unwrap_or_default(),server_path:file.path.clone()}))?.collect::<rusqlite::Result<Vec<_>>>()?;
            output.push(file);
        }
        Ok(output)
    }).await
}

pub(super) async fn prepare_locked(
    captured: Vec<Target>,
    key: String,
    db: &Database,
    actor: Option<String>,
    media_id: String,
    action: String,
) -> anyhow::Result<()> {
    db.write("managers.operations.prepare_locked", move |db| {
        db.execute(
            "INSERT INTO media_operations VALUES (?1,?2,?3,?4,'pending',?5,?6,?6,NULL)",
            params![
                key,
                actor,
                media_id,
                action,
                serde_json::to_string(&captured)?,
                now()
            ],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn protected_or_active(
    media: String,
    ids: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("managers.operations.protected_or_active", move|db|{
        let protected:bool=db.query_row("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id), ancestors(id,parent_id) AS (SELECT id,parent_id FROM media WHERE id IN (SELECT id FROM tree) UNION SELECT m.id,m.parent_id FROM media m JOIN ancestors a ON a.parent_id=m.id) SELECT EXISTS(SELECT 1 FROM media_protection p JOIN ancestors a ON a.id=p.media_id WHERE p.keep=1)",[media],|r|r.get(0))?;
        let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE file_id IN (SELECT value FROM json_each(?1)) AND state IN ('ready','playing','paused') AND updated_at>?2)",params![ids,now()-120],|r|r.get(0))?;
        Ok(protected||active)
    }).await
}

pub(super) async fn mutate(ids: String, db: &Database) -> anyhow::Result<()> {
    db.write("managers.operations.mutate", move |db| {
        db.execute(
            "UPDATE media_files SET present=0 WHERE id IN (SELECT value FROM json_each(?1))",
            [ids],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn execute_locked_read_media_operations(
    k: String,
    db: &Database,
) -> anyhow::Result<Option<(String, String, String, String)>> {
    db.read(
        "managers.operations.execute_locked_read_media_operations",
        move |db| {
            Ok(db
                .query_row(
                    "SELECT media_id,action,state,targets FROM media_operations WHERE id=?1",
                    [k],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?)
        },
    )
    .await
}

pub(super) async fn execute_locked_write_media_operations(
    k: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write(
        "managers.operations.execute_locked_write_media_operations",
        move |db| {
            db.execute(
                "UPDATE media_operations SET state='executing',updated_at=?1 WHERE id=?2",
                params![now(), k],
            )?;
            Ok(())
        },
    )
    .await
}

pub(super) async fn finish_operation(
    media: String,
    action: String,
    error: Option<String>,
    status: &'static str,
    db: &Database,
    key: String,
    actor: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.operations.finish_operation", move |db| {
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE media_operations SET state=?1,error=?2,updated_at=?3 WHERE id=?4",
            params![status, error, now(), key],
        )?;
        tx.execute(
            "UPDATE retention_candidates SET state=?1,error=?2 WHERE operation_id=?3",
            params![
                if status == "complete" {
                    "complete"
                } else {
                    "blocked"
                },
                error,
                key
            ],
        )?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",
            params![actor, format!("media.{action}.{status}"), media, now()],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn keep(
    db: &Database,
    media: String,
    input: Keep,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<()> {
    db.write("managers.operations.keep", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO media_protection VALUES (?1,?2) ON CONFLICT(media_id) DO UPDATE SET keep=excluded.keep",params![media,input.keep])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'media.keep',?2,?3)",params![p.user.id,media,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.operations.list", |db|Ok(db.prepare("SELECT o.id,o.media_id,m.title,o.action,o.state,o.created_at,o.error,json_array_length(o.targets) FROM media_operations o JOIN media m ON m.id=o.media_id ORDER BY o.created_at DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"media_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"action":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"created_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?,"files":r.get::<_,i64>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
