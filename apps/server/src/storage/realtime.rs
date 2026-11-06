//! Database operations for realtime.
use super::*;
use rusqlite::{Connection, ToSql, params};
use thelxinoe_database::Database;

pub(super) async fn events(
    db: &Database,
    since: i64,
    user: String,
    session: String,
) -> anyhow::Result<Option<Vec<Value>>> {
    db.read("realtime.events", move |db|{
        let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND expires_at>?2)",rusqlite::params![session,now()],|r|r.get(0))?;
        if !active{return Ok(None);}
        let mut query=db.prepare("SELECT id,kind,payload FROM events WHERE id>?1 AND (user_id IS NULL OR user_id=?2) ORDER BY id LIMIT 100")?;
        Ok(Some(query.query_map(rusqlite::params![since,user],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"payload":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(Value::Null)})))?.collect::<std::result::Result<Vec<_>,_>>()?))
    }).await
}

const JOB_PAGE: i64 = 100;

/// Jobs of the chosen type. Hiding automatic refreshes keeps the ones that failed. The state
/// filter applies after this, so the totals still count what each state would list.
const MATCHING_JOBS: &str = "WITH matching AS (
    SELECT j.rowid AS position,j.* FROM jobs j
    WHERE (?1 IS NULL OR j.kind=?1)
      AND (NOT ?2 OR j.state='failed' OR NOT (
        j.kind IN ('metadata.match','metadata.refresh')
        AND json_extract(j.payload,'$.actor_id') IS NULL
      ))
)";

/// Names what each job in `page` acted on and who started it. The payload stays here:
/// Recyclarr's holds its configuration files.
const JOB_ROWS: &str = "SELECT j.id,j.kind,
        CASE j.kind
          WHEN 'service.update' THEN json_extract(j.payload,'$.action')
          WHEN 'recyclarr.sync' THEN iif(json_extract(j.payload,'$.preview'),'preview',NULL)
        END,
        j.state,j.attempts,j.error,j.created_at,j.available_at,j.started_at,j.completed_at,
        COALESCE(root.name,item.title,request.title),
        CASE j.kind WHEN 'recyclarr.sync' THEN 'recyclarr' ELSE COALESCE(provision.kind,transfer.kind) END,
        CASE
          WHEN j.kind IN ('metadata.match','metadata.refresh') THEN json_extract(j.payload,'$.actor_id') IS NULL
          WHEN j.kind='recyclarr.sync' THEN json_extract(j.payload,'$.automatic')
          WHEN j.kind='service.update' THEN json_extract(j.payload,'$.automatic')
          WHEN j.kind='stack.install' THEN iif(json_extract(j.payload,'$.actor_id') IS NULL,NULL,0)
          WHEN j.kind IN ('service.transfer','manager.request') THEN 0
        END,
        actor.username
    FROM page j
    LEFT JOIN library_roots root ON j.kind='library.scan' AND root.id=j.subject
    LEFT JOIN media_cards item ON j.kind IN ('metadata.match','metadata.refresh') AND item.id=j.subject
    LEFT JOIN acquisition_requests request ON j.kind='manager.request' AND request.id=j.subject
    LEFT JOIN recyclarr_runs run ON j.kind='recyclarr.sync' AND run.id=j.subject
    LEFT JOIN service_updates service_update ON j.kind='service.update' AND service_update.id=j.subject
    LEFT JOIN service_transfers transfer ON j.kind='service.transfer' AND transfer.id=j.subject
    LEFT JOIN stack_provisions provision ON provision.id=CASE j.kind
        WHEN 'stack.install' THEN j.subject
        WHEN 'service.update' THEN service_update.service_id
      END
    LEFT JOIN users actor ON actor.id=CASE
        WHEN j.kind IN ('metadata.match','metadata.refresh','service.update','stack.install') THEN json_extract(j.payload,'$.actor_id')
        WHEN j.kind='recyclarr.sync' THEN run.actor_id
        WHEN j.kind='service.transfer' THEN transfer.actor_id
        WHEN j.kind='manager.request' THEN request.user_id
      END";

fn job_rows(db: &Connection, sql: &str, params: &[&dyn ToSql]) -> rusqlite::Result<Vec<Value>> {
    db.prepare(sql)?
        .query_map(params, |r| {
            Ok(json!({
                "id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,
                "action":r.get::<_,Option<String>>(2)?,"state":r.get::<_,String>(3)?,
                "attempts":r.get::<_,i64>(4)?,"error":r.get::<_,Option<String>>(5)?,
                "created_at":r.get::<_,i64>(6)?,"available_at":r.get::<_,i64>(7)?,
                "started_at":r.get::<_,Option<i64>>(8)?,"completed_at":r.get::<_,Option<i64>>(9)?,
                "target":r.get::<_,Option<String>>(10)?,"service":r.get::<_,Option<String>>(11)?,
                "automatic":r.get::<_,Option<bool>>(12)?,"actor":r.get::<_,Option<String>>(13)?,
            }))
        })?
        .collect()
}

/// The newest `limit` matching jobs and any others since `since`, what to keep above them and
/// the totals per state. Every failure in `attention` is pinned, however many jobs followed it, so each one can be dismissed. Running
/// and queued jobs past the cap on pinned ones are still listed in `items`.
pub(super) async fn jobs(
    db: &Database,
    filter: JobFilter,
    attention: Vec<String>,
) -> anyhow::Result<Value> {
    let attention = serde_json::to_string(&attention)?;
    db.read("realtime.jobs", move |db| {
        let JobFilter { kind, state, hide_automatic, limit, since } = filter;
        let mut totals = json!({"running":0,"queued":0,"failed":0,"complete":0});
        let mut counts = db.prepare(&format!(
            "{MATCHING_JOBS} SELECT state,COUNT(*) FROM matching GROUP BY state"
        ))?;
        for row in counts.query_map(params![kind, hide_automatic], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })? {
            let (state, count) = row?;
            totals[state.as_str()] = count.into();
        }
        let items = job_rows(
            db,
            &format!(
                "{MATCHING_JOBS},page AS (
                    SELECT * FROM (
                        SELECT *,ROW_NUMBER() OVER (ORDER BY created_at DESC,position DESC) AS newest
                        FROM matching WHERE (?3 IS NULL OR state=?3)
                    )
                    WHERE newest<=?4 OR created_at>=?5
                ) {JOB_ROWS} ORDER BY j.created_at DESC,j.position DESC"
            ),
            params![kind, hide_automatic, state, limit.map_or(JOB_PAGE, i64::from), since],
        )?;
        let pinned = job_rows(
            db,
            &format!(
                "{MATCHING_JOBS},page AS (
                    SELECT * FROM matching
                    WHERE (?3 IS NULL OR state=?3)
                      AND state='failed' AND id IN (SELECT value FROM json_each(?4))
                    UNION ALL
                    SELECT * FROM (
                        SELECT * FROM matching
                        WHERE (?3 IS NULL OR state=?3) AND state IN ('running','queued')
                        ORDER BY state='running' DESC,created_at DESC,position DESC LIMIT ?5
                    )
                ) {JOB_ROWS} ORDER BY j.state='failed' DESC,j.state='running' DESC,j.created_at DESC,j.position DESC"
            ),
            params![kind, hide_automatic, state, attention, JOB_PAGE],
        )?;
        Ok(json!({"items":items,"pinned":pinned,"totals":totals}))
    })
    .await
}
