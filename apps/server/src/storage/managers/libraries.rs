use crate::error::{ApiError, Result};
use rusqlite::params;
use serde_json::json;
use std::path::Path;
use thelxinoe_database::Database;

const ELIGIBLE: &str = "enabled=1 AND (operational=1 OR NOT EXISTS(SELECT 1 FROM stack_provisions p WHERE p.service_id=manager_services.id))";

pub(super) async fn services(db: &Database) -> anyhow::Result<Vec<(String, String)>> {
    db.read("managers.libraries.services", |db| {
        Ok(db
            .prepare(&format!(
                "SELECT id,kind FROM manager_services WHERE {ELIGIBLE} ORDER BY id"
            ))?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    })
    .await
}

pub(super) async fn ensure_root(
    db: &Database,
    service: Option<(String, String)>,
    kind: String,
    path: String,
) -> Result<()> {
    db.write("managers.libraries.ensure_root", move |db| {
        let tx = db.transaction()?;
        if let Some((id, generation)) = &service
            && !tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND generation=?2 AND {ELIGIBLE})"), params![id,generation], |r| r.get::<_,bool>(0))? {
            return Ok(Err(ApiError::conflict("Manager configuration changed during library discovery")));
        }
        let existing = tx.prepare("SELECT id,kind,path,last_scan FROM library_roots")?
            .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, existing_kind, existing_path, last_scan) in existing {
            if Path::new(&path).starts_with(&existing_path) && existing_kind == kind {
                // Startup can discover folders before the media tools are ready.
                if last_scan.is_none() && !tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='library.scan' AND subject=?1 AND state IN ('queued','running'))",
                    [&id], |r| r.get::<_,bool>(0),
                )? {
                    thelxinoe_jobs::enqueue_in(&tx,"library.scan",&id,&json!({"root_id":id}),0)?;
                    tx.commit()?;
                }
                return Ok(Ok(()));
            }
            if Path::new(&existing_path).starts_with(&path) || Path::new(&path).starts_with(&existing_path) {
                return Ok(Err(ApiError::conflict(format!("Library folder {path} overlaps the existing {existing_kind} library at {existing_path}"))));
            }
        }
        let id = thelxinoe_core::id();
        let label = match kind.as_str() { "movies" => "Movies", "shows" => "Shows", _ => "Music" };
        let folder = Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or(label);
        let name = if folder.eq_ignore_ascii_case(&kind) || (kind == "shows" && folder == "tv") {
            label.to_owned()
        } else {
            format!("{label} ({folder})")
        };
        tx.execute("INSERT INTO library_roots(id,name,kind,path) VALUES (?1,?2,?3,?4)",params![id,name,kind,path])?;
        // Root registration and its first scan survive a restart together.
        thelxinoe_jobs::enqueue_in(&tx,"library.scan",&id,&json!({"root_id":id}),0)?;
        tx.commit()?;
        Ok(Ok(()))
    }).await?
}
