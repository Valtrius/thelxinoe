//! Database operations for managers.operations.
use crate::managers::{
    bindings,
    domain::{MediaAction, PreparedOperation, Target, ValidatedSelection},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::now;
use thelxinoe_database::Database;

#[derive(Debug)]
pub(super) struct SharedFile;
impl std::fmt::Display for SharedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "A file also belongs to media outside this selection. Select all titles that share it.",
        )
    }
}
impl std::error::Error for SharedFile {}

pub(super) async fn targets(media: String, db: &Database) -> anyhow::Result<Vec<Target>> {
    db.read("managers.operations.targets", move |db| {
        load_targets(db, &media)
    })
    .await
}
pub(in crate::managers) fn load_targets(
    db: &rusqlite::Connection,
    media: &str,
) -> anyhow::Result<Vec<Target>> {
    let mut selected = db.prepare(
        "WITH RECURSIVE tree(id) AS (
             SELECT id FROM media WHERE id=?1
             UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id
         )
         SELECT f.id,f.generation,f.path,l.path,f.size,f.modified,f.fingerprint,f.ownership,
                EXISTS(SELECT 1 FROM media_sources WHERE file_id=f.id AND media_id NOT IN (SELECT id FROM tree))
         FROM media_files f JOIN library_roots l ON l.id=f.root_id
         WHERE f.present=1 AND f.id IN (
             SELECT file_id FROM media_sources WHERE media_id IN (SELECT id FROM tree)
         ) ORDER BY f.id"
    )?;
    let mut claims = db.prepare("SELECT service_id,service_generation,manager_file_id,entity_id,manager_path,external_id,members FROM manager_bindings WHERE file_id=?1 AND generation=?2 ORDER BY service_id")?;
    let mut rows = selected.query([media])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let outside: bool = row.get(8)?;
        anyhow::ensure!(!outside, SharedFile);
        let id: String = row.get(0)?;
        let generation: String = row.get(1)?;
        let path: String = row.get(2)?;
        let file_claims = claims
            .query_map(params![id, generation], |r| {
                Ok(bindings::Claim {
                    service_id: r.get(0)?,
                    service_generation: r.get(1)?,
                    manager_file_id: r.get(2)?,
                    entity_id: r.get(3)?,
                    manager_path: r.get(4)?,
                    external_id: r.get(5)?,
                    members: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                    server_path: path.clone(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        output.push(Target {
            id,
            generation,
            path,
            root: row.get(3)?,
            size: row.get::<_, i64>(4)? as u64,
            modified: row.get(5)?,
            fingerprint: row.get(6)?,
            ownership: row.get(7)?,
            claims: file_claims,
        });
    }
    Ok(output)
}

pub(super) async fn prepare_deletion(
    db: &Database,
    id: String,
    media_id: String,
    actor: Option<String>,
) -> anyhow::Result<Option<usize>> {
    db.write("managers.operations.prepare_deletion", move |db| {
        let tx = db.transaction()?;
        let files = load_targets(&tx, &media_id)?;
        let count = files.len();
        if count == 0 {
            return Ok(None);
        }
        insert_operation(
            &tx,
            &PreparedOperation {
                id,
                actor,
                media_id,
                action: MediaAction::Delete,
                selection: ValidatedSelection { files },
            },
        )?;
        tx.commit()?;
        Ok(Some(count))
    })
    .await
}

pub(super) async fn prepare_locked(
    operation: PreparedOperation,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.operations.prepare_locked", move |db| {
        let tx = db.transaction()?;
        insert_prepared(&tx, &operation)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub(in crate::managers) fn insert_prepared(
    db: &rusqlite::Connection,
    operation: &PreparedOperation,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        load_targets(db, &operation.media_id)? == operation.selection.files,
        "File generation or ownership changed before proposal persistence"
    );
    for claim in operation.selection.files.iter().flat_map(|f| &f.claims) {
        anyhow::ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND generation=?2 AND operational=1)",params![claim.service_id,claim.service_generation],|r|r.get::<_,bool>(0))?,"Manager configuration changed before proposal persistence");
    }
    insert_operation(db, operation)
}

fn insert_operation(
    db: &rusqlite::Connection,
    operation: &PreparedOperation,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO media_operations VALUES (?1,?2,?3,?4,'pending',?5,?6,?6,NULL)",
        params![
            operation.id,
            operation.actor,
            operation.media_id,
            operation.action.as_str(),
            serde_json::to_string(&operation.selection.files)?,
            now()
        ],
    )?;
    Ok(())
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
        // As after a scan, the scan time of a missing file is when it went missing.
        db.execute(
            "UPDATE media_files SET present=0,scanned_at=?2 WHERE id IN (SELECT value FROM json_each(?1))",
            params![ids, now()],
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

/// Whether the idle preflight can still use the managers named in a local preview.
pub(super) async fn preview_managers_current(
    db: &Database,
    targets: String,
) -> anyhow::Result<bool> {
    db.read("managers.operations.preview_managers_current", move |db| {
        Ok(db.query_row(
            "SELECT NOT EXISTS(
                SELECT 1 FROM json_each(?1) f, json_each(f.value,'$.claims') c
                WHERE NOT EXISTS(SELECT 1 FROM manager_services s WHERE s.operational=1
                    AND s.id=c.value->>'service_id' AND s.generation=c.value->>'service_generation')
            )",
            [targets],
            |row| row.get(0),
        )?)
    })
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

/// An attempt that changed nothing leaves its operation to run again.
pub(super) async fn reopen_operation(key: String, db: &Database) -> anyhow::Result<()> {
    db.write("managers.operations.reopen_operation", move |db| {
        db.execute(
            "UPDATE media_operations SET state='pending',updated_at=?1 WHERE id=?2 AND state='executing'",
            params![now(), key],
        )?;
        Ok(())
    })
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
    keep: bool,
    actor: String,
) -> anyhow::Result<()> {
    db.write("managers.operations.keep", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO media_protection VALUES (?1,?2) ON CONFLICT(media_id) DO UPDATE SET keep=excluded.keep",params![media,keep])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'media.keep',?2,?3)",params![actor,media,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Value>> {
    db.read("managers.operations.list", |db|Ok(db.prepare("SELECT o.id,o.media_id,m.title,o.action,o.state,o.created_at,o.error,json_array_length(o.targets) FROM media_operations o JOIN media m ON m.id=o.media_id ORDER BY o.created_at DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"media_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"action":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"created_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?,"files":r.get::<_,i64>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
