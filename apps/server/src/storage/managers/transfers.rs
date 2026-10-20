//! Database operations for managers.transfers. Ownership, holds and completion change
//! together with the transfer record in single transactions.
use super::*;
use thelxinoe_database::Database;

/// Unfinished transfers: everything except drafts and terminal outcomes.
pub(super) const ACTIVE: &str = "state NOT IN ('review','complete','cancelled','returned')";

#[derive(Clone, Debug)]
pub(super) struct Connected {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) container: String,
    pub(super) container_name: String,
    pub(super) engine: String,
    pub(super) access_revision: String,
    pub(super) port: u16,
    pub(super) url_base: String,
    pub(super) media_source: String,
    pub(super) evidence: Value,
    pub(super) owned: bool,
}

/// A saved connection of any kind, with whether Thelxinoe already owns that kind.
pub(super) async fn connected(db: &Database, service: String) -> anyhow::Result<Option<Connected>> {
    db.read("managers.transfers.connected", move |db| {
        Ok(db.query_row("SELECT s.id,s.kind,s.container_id,s.container_name,s.engine,s.access_revision,s.port,s.url_base,EXISTS(SELECT 1 FROM stack_provisions p WHERE p.kind=s.kind),s.media_source,s.evidence FROM (SELECT id,kind,container_id,container_name,engine,access_revision,port,url_base,media_source,evidence FROM manager_services WHERE enabled=1 UNION ALL SELECT id,kind,container_id,container_name,engine,access_revision,port,url_base,media_source,evidence FROM support_services UNION ALL SELECT id,'recyclarr',container_id,container_name,engine,access_revision,0,'','',evidence FROM recyclarr_connections) s WHERE s.id=?1",
            [service], |r| Ok(Connected { id:r.get(0)?, kind:r.get(1)?, container:r.get(2)?, container_name:r.get(3)?, engine:r.get(4)?, access_revision:r.get(5)?, port:r.get(6)?, url_base:r.get(7)?, owned:r.get(8)?, media_source:r.get(9)?, evidence:r.get::<_,Option<String>>(10)?.and_then(|e| serde_json::from_str(&e).ok()).unwrap_or_default() })).optional()?)
    }).await
}

pub(super) async fn active_for_kind(db: &Database, kind: String) -> anyhow::Result<Option<String>> {
    db.read("managers.transfers.active_for_kind", move |db| {
        Ok(db
            .query_row(
                &format!("SELECT id FROM service_transfers WHERE kind=?1 AND {ACTIVE}"),
                [kind],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Row {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) service_id: String,
    pub(super) actor_id: String,
    pub(super) container_id: String,
    pub(super) engine: String,
    pub(super) connection_revision: String,
    pub(super) review_revision: String,
    pub(super) review: Value,
    pub(super) choices: Value,
    pub(super) declarations: Value,
    pub(super) state: String,
    pub(super) step: String,
    pub(super) checks: Value,
    pub(super) setup: Value,
    pub(super) recovery: Value,
    pub(super) progress: Value,
    pub(super) replacement_id: Option<String>,
    pub(super) error: Option<String>,
    pub(super) approved_by: Option<String>,
    pub(super) approved_at: Option<i64>,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) completed_at: Option<i64>,
}
const COLUMNS: &str = "id,kind,service_id,actor_id,container_id,engine,connection_revision,review_revision,review,choices,declarations,state,step,checks,setup,recovery,progress,replacement_id,error,approved_by,approved_at,created_at,updated_at,completed_at";
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let json = |i: usize| -> rusqlite::Result<Value> {
        Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or(Value::Null))
    };
    Ok(Row {
        id: r.get(0)?,
        kind: r.get(1)?,
        service_id: r.get(2)?,
        actor_id: r.get(3)?,
        container_id: r.get(4)?,
        engine: r.get(5)?,
        connection_revision: r.get(6)?,
        review_revision: r.get(7)?,
        review: json(8)?,
        choices: json(9)?,
        declarations: json(10)?,
        state: r.get(11)?,
        step: r.get(12)?,
        checks: json(13)?,
        setup: json(14)?,
        recovery: json(15)?,
        progress: json(16)?,
        replacement_id: r.get(17)?,
        error: r.get(18)?,
        approved_by: r.get(19)?,
        approved_at: r.get(20)?,
        created_at: r.get(21)?,
        updated_at: r.get(22)?,
        completed_at: r.get(23)?,
    })
}

pub(super) async fn load(db: &Database, id: String) -> anyhow::Result<Option<Row>> {
    db.read("managers.transfers.load", move |db| {
        Ok(db
            .query_row(
                &format!("SELECT {COLUMNS} FROM service_transfers WHERE id=?1"),
                [id],
                row,
            )
            .optional()?)
    })
    .await
}

/// Unfinished transfers and the most recent outcome of each service kind.
pub(super) async fn list(db: &Database) -> anyhow::Result<Vec<Row>> {
    db.read("managers.transfers.list", |db| {
        Ok(db.prepare(&format!("SELECT {COLUMNS} FROM service_transfers t WHERE {ACTIVE} OR (state!='review' AND updated_at=(SELECT MAX(updated_at) FROM service_transfers o WHERE o.kind=t.kind)) OR (state='review' AND updated_at>?1) ORDER BY updated_at DESC LIMIT 50"))?
            .query_map([now() - 86400], row)?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}

pub(super) struct Draft {
    pub(super) id: String,
    pub(super) connected: Connected,
    pub(super) actor: String,
    pub(super) review_revision: String,
    pub(super) review: Value,
}
/// A reviewed plan. A newer review replaces older drafts of the same service.
pub(super) async fn save_review(db: &Database, draft: Draft) -> anyhow::Result<()> {
    db.write("managers.transfers.save_review", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let c = &draft.connected;
        tx.execute("UPDATE service_transfers SET state='cancelled',error='Replaced by a newer review',updated_at=?2 WHERE service_id=?1 AND state='review'", params![c.id, now()])?;
        tx.execute("INSERT INTO service_transfers(id,kind,service_id,actor_id,container_id,engine,connection_revision,review_revision,review,state,step,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'review','review',?10,?10)",
            params![draft.id, c.kind, c.id, draft.actor, c.container, c.engine, c.access_revision, draft.review_revision, draft.review.to_string(), now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'transfer.review',?2,?3)", params![draft.actor, draft.id, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

/// Changing a reviewed choice produces a new revision; approval always names the latest one.
pub(super) async fn choices(
    db: &Database,
    id: String,
    choices: Value,
    revision: String,
) -> anyhow::Result<bool> {
    db.write("managers.transfers.choices", move |db| {
        Ok(db.execute("UPDATE service_transfers SET choices=?2,review_revision=?3,updated_at=?4 WHERE id=?1 AND state='review'", params![id, choices.to_string(), revision, now()])? == 1)
    }).await
}

pub(super) enum Approval {
    Queued,
    Stale,
    Busy,
}
/// Queue an approved transfer once; other tabs and repeats find the same operation.
pub(super) async fn approve(
    db: &Database,
    id: String,
    actor: String,
    declarations: Value,
) -> anyhow::Result<Approval> {
    db.write("managers.transfers.approve", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (kind, service, container, revision, state): (String, String, String, String, String) = tx.query_row(
            "SELECT kind,service_id,container_id,connection_revision,state FROM service_transfers WHERE id=?1", [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        if state != "review" {
            return Ok(if state == "cancelled" { Approval::Stale } else { Approval::Queued });
        }
        // The reviewed connection must be exactly the current one.
        let current: Option<(String, String)> = tx.query_row("SELECT container_id,access_revision FROM manager_services WHERE id=?1 AND enabled=1 UNION ALL SELECT container_id,access_revision FROM support_services WHERE id=?1 UNION ALL SELECT container_id,access_revision FROM recyclarr_connections WHERE id=?1", [&service], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if current != Some((container, revision)) || tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1)", [&kind], |r| r.get::<_, bool>(0))? {
            tx.execute("UPDATE service_transfers SET state='cancelled',error='The connection changed after review',updated_at=?2 WHERE id=?1", params![id, now()])?;
            tx.commit()?;
            return Ok(Approval::Stale);
        }
        if tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM service_transfers WHERE kind=?1 AND id!=?2 AND {ACTIVE})"), params![kind, id], |r| r.get::<_, bool>(0))? {
            return Ok(Approval::Busy);
        }
        tx.execute("UPDATE service_transfers SET state='queued',step='stop',declarations=?2,approved_by=?3,approved_at=?4,updated_at=?4,error=NULL WHERE id=?1", params![id, declarations.to_string(), actor, now()])?;
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.transfer',?2,?3,'queued',?4,?4) ON CONFLICT(dedupe_key) DO NOTHING", params![thelxinoe_core::id(), json!({"id":id}).to_string(), format!("transfer:{id}"), now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'transfer.approve',?2,?3)", params![actor, id, now()])?;
        tx.commit()?;
        Ok(Approval::Queued)
    }).await
}

/// Record progress without changing ownership.
pub(super) async fn update(
    db: &Database,
    id: String,
    state: Option<String>,
    step: Option<String>,
    progress: Option<Value>,
    error: Option<Option<String>>,
) -> anyhow::Result<()> {
    db.write("managers.transfers.update", move |db| {
        db.execute("UPDATE service_transfers SET state=COALESCE(?2,state),step=COALESCE(?3,step),progress=COALESCE(?4,progress),error=CASE WHEN ?5 THEN ?6 ELSE error END,updated_at=?7 WHERE id=?1",
            params![id, state, step, progress.map(|p| p.to_string()), error.is_some(), error.flatten(), now()])?;
        Ok(())
    }).await
}

pub(super) async fn outcome(
    db: &Database,
    id: String,
    state: String,
    error: Option<String>,
    actor: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.transfers.outcome", move |db| {
        let tx = db.transaction()?;
        tx.execute("UPDATE service_transfers SET state=?2,error=?3,updated_at=?4,completed_at=CASE WHEN ?2 IN ('cancelled','returned') THEN ?4 ELSE completed_at END WHERE id=?1", params![id, state, error, now()])?;
        tx.execute("UPDATE jobs SET state='complete' WHERE kind='service.transfer' AND json_extract(payload,'$.id')=?1 AND ?2 IN ('cancelled','returned') AND state='queued'", params![id, state])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)", params![actor, format!("transfer.{state}"), id, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) struct Commit {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) service: String,
    pub(super) actor: String,
    pub(super) replacement: String,
    pub(super) container_name: String,
    pub(super) port: u16,
    pub(super) url_base: String,
    pub(super) media_source: String,
    pub(super) evidence: Value,
    pub(super) host_port: Option<u16>,
    pub(super) credential: Option<Vec<u8>>,
}
/// Commit the accepted container identity as an Owned service under its transfer hold.
pub(super) async fn commit(db: &Database, commit: Commit) -> anyhow::Result<()> {
    db.write("managers.transfers.commit", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state: String = tx.query_row("SELECT state FROM service_transfers WHERE id=?1", [&commit.id], |r| r.get(0))?;
        if state == "committed" {
            return Ok(());
        }
        anyhow::ensure!(state == "running", "The transfer isn't ready for its ownership commit");
        let service = if commit.kind == "recyclarr" {
            tx.execute("DELETE FROM recyclarr_connections WHERE id=?1", [&commit.service])?;
            None
        } else {
            let table = if matches!(commit.kind.as_str(), "radarr" | "sonarr" | "lidarr") { "manager_services" } else { "support_services" };
            anyhow::ensure!(tx.execute(&format!("UPDATE {table} SET container_id=?2,container_name=?3,port=?4,url_base=?5,generation=?6,access_revision=?7,operational=0,verified=1,error=NULL,checked_at=?8,media_source=?9,evidence=?10 WHERE id=?1"),
                params![commit.service, commit.replacement, commit.container_name, commit.port, commit.url_base, thelxinoe_core::id(), thelxinoe_core::id(), now(), commit.media_source, commit.evidence.to_string()])? == 1, "The connection disappeared before ownership commit");
            Some(commit.service.clone())
        };
        let native: String = tx.query_row("SELECT COALESCE((SELECT native_url FROM support_services WHERE id=?1),'')", [&commit.service], |r| r.get(0))?;
        tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,origin,hold,transfer_id,native_url,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,'complete',?6,?7,'transferred','transfer',?1,?8,?9,?9)",
            params![commit.id, commit.kind, commit.actor, commit.host_port, commit.credential, commit.replacement, service, native, now()])?;
        tx.execute("UPDATE service_transfers SET state='committed',step='setup',replacement_id=?2,updated_at=?3,error=NULL WHERE id=?1", params![commit.id, commit.replacement, now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'transfer.commit',?2,?3)", params![commit.actor, commit.id, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn journal(db: &Database, id: String, setup: Value) -> anyhow::Result<()> {
    db.write("managers.transfers.journal", move |db| {
        db.execute(
            "UPDATE service_transfers SET setup=?2,updated_at=?3 WHERE id=?1",
            params![id, setup.to_string(), now()],
        )?;
        Ok(())
    })
    .await
}

/// Record evaluated checks. A required check that failed is a recorded local failure.
pub(super) async fn checks(
    db: &Database,
    id: String,
    checks: Value,
    step: String,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("managers.transfers.checks", move |db| {
        db.execute(&format!("UPDATE service_transfers SET checks=?2,step=?3,error=?4,state=CASE WHEN ?4 IS NULL THEN state ELSE 'failed' END,updated_at=?5 WHERE id=?1 AND {ACTIVE}"), params![id, checks.to_string(), step, error, now()])?;
        Ok(())
    }).await
}

/// A confirmed failure of an unfinished transfer. Its holds stay exactly as they are; a
/// finished or recovering transfer is never reopened or demoted here.
pub(super) async fn fail(
    db: &Database,
    id: String,
    step: Option<String>,
    error: String,
) -> anyhow::Result<()> {
    db.write("managers.transfers.fail", move |db| {
        let tx = db.transaction()?;
        if tx.execute("UPDATE service_transfers SET state='failed',step=COALESCE(?2,step),error=?3,updated_at=?4 WHERE id=?1 AND state IN ('queued','running','committed')", params![id, step, error, now()])? == 1 {
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,'transfer.failed',?1,?2)", params![id, now()])?;
        }
        tx.commit()?;
        Ok(())
    }).await
}

/// Completion and operational-hold release are one durable event, recorded only after the
/// controller agreed. A transfer that failed after that agreement completes the same way.
pub(super) async fn complete(db: &Database, id: String) -> anyhow::Result<()> {
    db.write("managers.transfers.complete", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (state, kind, service): (String, String, String) = tx.query_row("SELECT state,kind,service_id FROM service_transfers WHERE id=?1", [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        if state == "complete" {
            return Ok(());
        }
        anyhow::ensure!(matches!(state.as_str(), "committed" | "failed"), "Only a committed transfer can complete");
        anyhow::ensure!(tx.execute("UPDATE stack_provisions SET hold=NULL,error=NULL,updated_at=?2 WHERE transfer_id=?1 AND hold='transfer'", params![id, now()])? == 1, "The transfer's ownership record changed");
        if kind != "recyclarr" {
            let table = if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") { "manager_services" } else { "support_services" };
            anyhow::ensure!(tx.execute(&format!("UPDATE {table} SET operational=1 WHERE id=?1"), [&service])? == 1, "The transferred connection changed");
        }
        tx.execute("UPDATE service_transfers SET state='complete',step='complete',error=NULL,completed_at=?2,updated_at=?2 WHERE id=?1", params![id, now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (NULL,'transfer.complete',?1,?2)", params![id, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) struct Original {
    pub(super) id: String,
    /// The recovery command that returned it, or none when reconciling the controller's outcome.
    pub(super) operation: Option<String>,
    pub(super) kind: String,
    pub(super) service: String,
    pub(super) container: String,
    pub(super) container_name: String,
    pub(super) port: u16,
    pub(super) url_base: String,
    pub(super) media_source: String,
    pub(super) evidence: Value,
    pub(super) engine: String,
    pub(super) actor: Option<String>,
}
/// End a failed transfer in the Connected stage, after the controller retired its replacement
/// and restarted the original. Repeating it changes nothing.
pub(super) async fn return_original(db: &Database, original: Original) -> anyhow::Result<bool> {
    db.write("managers.transfers.return_original", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: bool = match &original.operation {
            Some(operation) => tx.query_row("SELECT EXISTS(SELECT 1 FROM service_transfers WHERE id=?1 AND state='recovery' AND json_extract(recovery,'$.command.operation')=?2)", params![original.id, operation], |r| r.get(0))?,
            None => tx.query_row("SELECT EXISTS(SELECT 1 FROM service_transfers WHERE id=?1 AND state IN ('queued','running','committed','failed','suspended'))", [&original.id], |r| r.get(0))?,
        };
        if !current {
            return Ok(false);
        }
        tx.execute("DELETE FROM stack_provisions WHERE transfer_id=?1", [&original.id])?;
        if original.kind == "recyclarr" {
            tx.execute("INSERT INTO recyclarr_connections(id,container_id,container_name,engine,access_revision,checked_at,error) VALUES (?1,?2,?3,?4,?5,0,'Returned to the original after a failed transfer') ON CONFLICT(id) DO NOTHING",
                params![original.service, original.container, original.container_name, original.engine, thelxinoe_core::id()])?;
        } else {
            let table = if matches!(original.kind.as_str(), "radarr" | "sonarr" | "lidarr") { "manager_services" } else { "support_services" };
            tx.execute(&format!("UPDATE {table} SET container_id=?2,container_name=?3,port=?4,url_base=?5,generation=?6,access_revision=?7,media_source=?8,evidence=?9,operational=0,verified=0,checked_at=0,error='Returned to the original after a failed transfer; test the connection' WHERE id=?1"),
                params![original.service, original.container, original.container_name, original.port, original.url_base, thelxinoe_core::id(), thelxinoe_core::id(), original.media_source, original.evidence.to_string()])?;
        }
        tx.execute("UPDATE service_transfers SET state='returned',progress='{}',recovery=CASE WHEN json_extract(recovery,'$.command') IS NULL THEN recovery ELSE json_set(recovery,'$.command.outcome','applied','$.command.finished_at',?2) END,completed_at=?2,updated_at=?2 WHERE id=?1", params![original.id, now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'transfer.return-original',?2,?3)", params![original.actor, original.id, now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn recovery(
    db: &Database,
    id: String,
    recovery: Value,
    state: Option<String>,
    actor: Option<String>,
    action: &'static str,
) -> anyhow::Result<()> {
    db.write("managers.transfers.recovery", move |db| {
        let tx = db.transaction()?;
        tx.execute("UPDATE service_transfers SET recovery=?2,state=COALESCE(?3,state),updated_at=?4 WHERE id=?1", params![id, recovery.to_string(), state, now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)", params![actor, action, id, now()])?;
        tx.commit()?;
        Ok(())
    }).await
}

fn queue(db: &rusqlite::Connection, transfer: &str, delay: i64) -> rusqlite::Result<()> {
    let changed = db.execute("UPDATE jobs SET state='queued',available_at=?2,started_at=NULL,error=NULL WHERE kind='service.transfer' AND json_extract(payload,'$.id')=?1", params![transfer, now() + delay])?;
    if changed == 0 {
        db.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'service.transfer',?2,?3,'queued',?4,?4)", params![thelxinoe_core::id(), json!({"id":transfer}).to_string(), format!("transfer:{transfer}"), now() + delay])?;
    }
    Ok(())
}

/// Run the transfer job again soon, without losing its identity.
pub(super) async fn requeue(db: &Database, transfer: String, delay: i64) -> anyhow::Result<()> {
    db.write("managers.transfers.requeue", move |db| {
        queue(db, &transfer, delay)?;
        Ok(())
    })
    .await
}

/// Make an approved recovery command durable before it is sent. The transfer job observes the
/// controller until the command's outcome is known, whatever happens to this request.
pub(super) async fn begin_recovery(
    db: &Database,
    id: String,
    from: String,
    command: Value,
    actor: String,
) -> anyhow::Result<bool> {
    db.write("managers.transfers.begin_recovery", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.execute("UPDATE service_transfers SET state='recovery',error=NULL,progress='{}',recovery=json_set(recovery,'$.command',json(?3)),updated_at=?4 WHERE id=?1 AND state=?2", params![id, from, command.to_string(), now()])? == 0 {
            return Ok(false);
        }
        queue(&tx, &id, 0)?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'transfer.recovery.request',?2,?3)", params![actor, id, now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

/// The controller's outcome of one recovery command and the state it leaves the transfer in.
pub(super) struct Outcome {
    pub(super) id: String,
    pub(super) operation: String,
    pub(super) state: String,
    pub(super) error: Option<String>,
    /// `applied`, `failed` or `refused`.
    pub(super) outcome: &'static str,
    /// A restored candidate replaced the held container.
    pub(super) replacement: Option<String>,
    pub(super) action: &'static str,
}
/// Record a recovery command's outcome once. Later observations of the same outcome change
/// nothing; a different command's outcome never applies to this one.
pub(super) async fn end_recovery(db: &Database, end: Outcome) -> anyhow::Result<bool> {
    db.write("managers.transfers.end_recovery", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let actor: Option<Option<String>> = tx.query_row("SELECT json_extract(recovery,'$.command.by') FROM service_transfers WHERE id=?1 AND state='recovery' AND json_extract(recovery,'$.command.operation')=?2", params![end.id, end.operation], |r| r.get(0)).optional()?;
        let Some(actor) = actor else {
            return Ok(false);
        };
        if let Some(container) = &end.replacement {
            rebind(&tx, &end.id, container)?;
        }
        tx.execute("UPDATE service_transfers SET state=?2,error=?3,progress='{}',recovery=json_set(recovery,'$.command.outcome',?4,'$.command.finished_at',?5),updated_at=?5 WHERE id=?1", params![end.id, end.state, end.error, end.outcome, now()])?;
        if matches!(end.state.as_str(), "running" | "committed") {
            queue(&tx, &end.id, 0)?;
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)", params![actor, end.action, end.id, now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

/// Forget a Connected service: Thelxinoe's record only, never the container or its data.
pub(super) async fn forget(db: &Database, service: String, actor: String) -> anyhow::Result<bool> {
    db.write("managers.transfers.forget", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind: Option<String> = tx.query_row("SELECT kind FROM manager_services WHERE id=?1 AND enabled=1 UNION ALL SELECT kind FROM support_services WHERE id=?1 UNION ALL SELECT 'recyclarr' FROM recyclarr_connections WHERE id=?1", [&service], |r| r.get(0)).optional()?;
        let Some(kind) = kind else { return Ok(false) };
        let blocked: bool = tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind=?1 OR service_id=?2) OR EXISTS(SELECT 1 FROM service_transfers WHERE service_id=?2 AND {ACTIVE})"), params![kind, service], |r| r.get(0))?;
        if blocked {
            return Ok(false);
        }
        match kind.as_str() {
            "radarr" | "sonarr" | "lidarr" => {
                tx.execute("DELETE FROM settings WHERE key='manager.retention.'||?1", [&service])?;
                let referenced: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM acquisition_requests WHERE service_id=?1) OR EXISTS(SELECT 1 FROM manager_bindings WHERE service_id=?1) OR EXISTS(SELECT 1 FROM recyclarr_targets WHERE service_id=?1)", [&service], |r| r.get(0))?;
                if referenced {
                    tx.execute("UPDATE manager_services SET enabled=0,operational=0,verified=0,credential=X'',container_id='forgotten:'||id,port=0,evidence='{}',error='Connection forgotten' WHERE id=?1", [&service])?;
                } else {
                    tx.execute("DELETE FROM manager_reconciliations WHERE service_id=?1", [&service])?;
                    tx.execute("DELETE FROM manager_services WHERE id=?1", [&service])?;
                }
            }
            "recyclarr" => {
                tx.execute("DELETE FROM recyclarr_connections WHERE id=?1", [&service])?;
            }
            _ => {
                tx.execute("DELETE FROM seerr_request_states WHERE service_id=?1", [&service])?;
                tx.execute("DELETE FROM support_services WHERE id=?1", [&service])?;
            }
        }
        tx.execute("DELETE FROM settings WHERE key LIKE 'services.connection.%' AND (json_extract(value,'$.source')=?1 OR json_extract(value,'$.target')=?1)", [&service])?;
        tx.execute("UPDATE service_transfers SET state='cancelled',error='Connection forgotten',updated_at=?2 WHERE service_id=?1 AND state='review'", params![service, now()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'connection.forget',?2,?3)", params![actor, service, now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) struct JobConnection {
    pub(super) container: String,
    pub(super) container_name: String,
    pub(super) engine: String,
    pub(super) version: String,
    pub(super) evidence: Value,
    pub(super) error: Option<String>,
}
/// Save or revalidate Recyclarr's Connected job validation record.
pub(super) async fn connect_recyclarr(
    db: &Database,
    record: JobConnection,
    actor: Option<String>,
) -> anyhow::Result<Option<String>> {
    db.write("managers.transfers.connect_recyclarr", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind='recyclarr') OR EXISTS(SELECT 1 FROM service_transfers WHERE kind='recyclarr' AND {ACTIVE})"), [], |r| r.get::<_, bool>(0))? {
            return Ok(None);
        }
        let key: String = tx.query_row("SELECT id FROM recyclarr_connections", [], |r| r.get(0)).optional()?.unwrap_or_else(id);
        tx.execute("INSERT INTO recyclarr_connections(id,container_id,container_name,engine,version,access_revision,evidence,verified,checked_at,error) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET container_id=excluded.container_id,container_name=excluded.container_name,engine=excluded.engine,version=excluded.version,access_revision=CASE WHEN recyclarr_connections.container_id=excluded.container_id THEN recyclarr_connections.access_revision ELSE excluded.access_revision END,evidence=excluded.evidence,verified=excluded.verified,checked_at=excluded.checked_at,error=excluded.error",
            params![key, record.container, record.container_name, record.engine, record.version, thelxinoe_core::id(), record.evidence.to_string(), record.error.is_none(), now(), record.error])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'connection.validate',?2,?3)", params![actor, key, now()])?;
        tx.commit()?;
        Ok(Some(key))
    }).await
}

/// Per-kind records for the two-stage service list.
pub(super) async fn stages(db: &Database) -> anyhow::Result<Value> {
    db.read("managers.transfers.stages", |db| {
        let connections = db.prepare("SELECT id,kind,container_id,container_name,engine,version,verified,operational,checked_at,error,port,url_base FROM manager_services WHERE enabled=1 UNION ALL SELECT id,kind,container_id,container_name,engine,version,verified,operational,checked_at,error,port,url_base FROM support_services UNION ALL SELECT id,'recyclarr',container_id,container_name,engine,version,verified,0,checked_at,error,0,'' FROM recyclarr_connections")?
            .query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"container_id":r.get::<_,String>(2)?,"container_name":r.get::<_,String>(3)?,
                "engine":r.get::<_,String>(4)?,"version":r.get::<_,String>(5)?,"verified":r.get::<_,bool>(6)?,"operational":r.get::<_,bool>(7)?,
                "checked_at":r.get::<_,i64>(8)?,"error":r.get::<_,Option<String>>(9)?,"port":r.get::<_,u16>(10)?,"url_base":r.get::<_,String>(11)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ownership = db.prepare("SELECT id,kind,state,origin,hold,transfer_id,service_id,container_id,error FROM stack_provisions")?
            .query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"origin":r.get::<_,String>(3)?,
                "hold":r.get::<_,Option<String>>(4)?,"transfer_id":r.get::<_,Option<String>>(5)?,"service_id":r.get::<_,Option<String>>(6)?,
                "container_id":r.get::<_,Option<String>>(7)?,"error":r.get::<_,Option<String>>(8)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let transfers = db.prepare(&format!("SELECT id,kind,service_id,state,step,error,updated_at FROM service_transfers WHERE {ACTIVE} OR (state='review' AND updated_at>?1)"))?
            .query_map([now() - 86400], |r| Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"service_id":r.get::<_,String>(2)?,
                "state":r.get::<_,String>(3)?,"step":r.get::<_,String>(4)?,"error":r.get::<_,Option<String>>(5)?,"updated_at":r.get::<_,i64>(6)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"connections":connections,"ownership":ownership,"transfers":transfers}))
    }).await
}

/// The approving administrator, for Seerr's administrator identity provisioning.
pub(super) async fn administrator(
    db: &Database,
    user: String,
) -> anyhow::Result<Option<thelxinoe_core::Principal>> {
    db.read("managers.transfers.administrator", move |db| {
        let row: Option<(String, String, String, String)> = db
            .query_row(
                "SELECT id,username,role,timezone FROM user_profiles WHERE id=?1 AND role='admin'",
                [user],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(match row {
            Some((id, username, role, timezone)) => Some(thelxinoe_core::Principal {
                user: thelxinoe_core::User {
                    id,
                    username,
                    role: serde_json::from_value(json!(role))?,
                    timezone,
                },
                session_id: String::new(),
                transport: "transfer".into(),
            }),
            None => None,
        })
    })
    .await
}

/// A transferred Recyclarr keeps its reviewed guide selections on a paused schedule.
pub(super) async fn recyclarr_schedule(
    db: &Database,
    provision: String,
    actor: String,
    targets: Value,
) -> anyhow::Result<()> {
    db.write("managers.transfers.recyclarr_schedule", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let zone = crate::timezones::server_zone(&tx)?;
        tx.execute("INSERT OR IGNORE INTO recyclarr_settings(provision_id,actor_id,paused,next_run,timezone) VALUES (?1,?2,1,?3,?4)", params![provision, actor, now() + 86400, zone.name()])?;
        for target in targets.as_array().into_iter().flatten() {
            anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND operational=1)", params![target["service_id"].as_str()], |r| r.get::<_, bool>(0))?, "A Recyclarr target is no longer an operational Owned service");
            tx.execute("INSERT OR IGNORE INTO recyclarr_targets(service_id,provision_id,trash_id,revision,quality_sizes,groups,reset_scores,overrides) VALUES (?1,?2,?3,?4,?5,?6,0,?7)",
                params![target["service_id"].as_str(), provision, target["trash_id"].as_str(), thelxinoe_core::id(), target["quality_sizes"].as_bool().unwrap_or(false), target["groups"].to_string(), target["overrides"].to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) async fn recyclarr_paused(db: &Database, provision: String) -> anyhow::Result<bool> {
    db.read("managers.transfers.recyclarr_paused", move |db| {
        Ok(db
            .query_row(
                "SELECT paused FROM recyclarr_settings WHERE provision_id=?1",
                [provision],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    })
    .await
}

/// A restored candidate replaced the held container: point ownership at it, still held.
/// Its appdata is a new generation, so setup and checks of the discarded one stop counting;
/// they are kept only as that generation's evidence.
fn rebind(tx: &rusqlite::Transaction<'_>, id: &str, container: &str) -> anyhow::Result<()> {
    let (kind, service, replacement, setup, checks, recovery): (String, String, Option<String>, String, String, String) = tx.query_row(
        "SELECT kind,service_id,replacement_id,setup,checks,recovery FROM service_transfers WHERE id=?1", [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
    if replacement.as_deref() == Some(container) {
        return Ok(());
    }
    let parse = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
    let mut recovery = parse(&recovery);
    let superseded =
        json!({"replacement":replacement,"setup":parse(&setup),"checks":parse(&checks),"at":now()});
    match recovery["superseded"].as_array_mut() {
        Some(list) => list.push(superseded),
        None => recovery["superseded"] = json!([superseded]),
    }
    tx.execute("UPDATE stack_provisions SET container_id=?2,updated_at=?3 WHERE transfer_id=?1 AND hold IS NOT NULL", params![id, container, now()])?;
    if kind != "recyclarr" {
        let table = if matches!(kind.as_str(), "radarr" | "sonarr" | "lidarr") {
            "manager_services"
        } else {
            "support_services"
        };
        tx.execute(&format!("UPDATE {table} SET container_id=?2,generation=?3,access_revision=?4,operational=0 WHERE id=?1"), params![service, container, thelxinoe_core::id(), thelxinoe_core::id()])?;
    }
    tx.execute("UPDATE service_transfers SET replacement_id=?2,setup='[]',checks='[]',recovery=?3,updated_at=?4 WHERE id=?1", params![id, container, recovery.to_string(), now()])?;
    Ok(())
}
