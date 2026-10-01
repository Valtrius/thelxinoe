use super::*;
use chrono::{TimeZone, Timelike, Utc};
use thelxinoe_database::Database;

fn next(db: &rusqlite::Connection, hour: u32) -> anyhow::Result<i64> {
    let zone = crate::timezones::server_zone(db)?;
    let instant = Utc::now().timestamp();
    let current = zone.timestamp_opt(instant, 0).single().unwrap();
    // Search UTC minutes, which also handles ambiguous/missing local DST hours.
    for minute in 1..=2880 {
        let at = (instant / 60 + minute) * 60;
        let local = zone.timestamp_opt(at, 0).single().unwrap();
        if local.hour() == hour
            && local.minute() == 0
            && (local.date_naive() > current.date_naive() || current.hour() < hour)
        {
            return Ok(at);
        }
    }
    Ok(instant + 86400)
}
pub(super) async fn installed(
    db: &Database,
    provision: String,
    actor: String,
) -> anyhow::Result<()> {
    db.write("recyclarr.installed",move |c| {
        c.execute("INSERT OR IGNORE INTO recyclarr_settings(provision_id,actor_id,next_run,timezone) VALUES (?1,?2,?3,?4)",params![provision,actor,next(c,4)?,crate::timezones::server_zone(c)?.name()])?;
        Ok(())
    }).await
}
pub(super) async fn provision(db: &Database) -> anyhow::Result<Option<String>> {
    db.read("recyclarr.provision", |c| {
        Ok(c.query_row(
            "SELECT id FROM stack_provisions WHERE kind='recyclarr' AND state='complete'",
            [],
            |r| r.get(0),
        )
        .optional()?)
    })
    .await
}
pub(super) async fn owns(db: &Database, service: String) -> anyhow::Result<bool> {
    db.read("recyclarr.owns",move |c| Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM recyclarr_targets t JOIN stack_provisions p ON p.id=t.provision_id WHERE t.service_id=?1 AND p.state='complete')",[service],|r|r.get(0))?)).await
}
fn enqueue(
    c: &rusqlite::Connection,
    provision: &str,
    actor: &str,
    preview: bool,
    automatic: bool,
) -> anyhow::Result<String> {
    let revision:String=c.query_row("SELECT COALESCE(group_concat(revision),'') FROM (SELECT t.revision || m.generation AS revision FROM recyclarr_targets t JOIN manager_services m ON m.id=t.service_id WHERE t.provision_id=?1 ORDER BY t.service_id,t.trash_id)",[provision],|r|r.get(0))?;
    if let Some(run) = c.query_row("SELECT r.id FROM recyclarr_runs r JOIN jobs j ON json_extract(j.payload,'$.id')=r.id AND j.kind='recyclarr.sync' WHERE r.provision_id=?1 AND r.state IN ('queued','running','retrying') AND json_extract(j.payload,'$.preview')=?2 AND json_extract(j.payload,'$.revision')=?3 ORDER BY r.created_at LIMIT 1",params![provision,preview,revision],|r|r.get(0)).optional()? { return Ok(run); }
    let run = id();
    c.execute("INSERT INTO recyclarr_runs(id,provision_id,actor_id,state,created_at,updated_at) VALUES (?1,?2,?3,'queued',?4,?4)",params![run,provision,actor,now()])?;
    c.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'recyclarr.sync',?2,?3,'queued',?4,?4)",params![id(),json!({"id":run,"preview":preview,"automatic":automatic,"revision":revision}).to_string(),format!("recyclarr:{run}"),now()])?;
    Ok(run)
}
pub(super) async fn queue(
    db: &Database,
    provision: String,
    actor: String,
    preview: bool,
) -> anyhow::Result<String> {
    db.write("recyclarr.queue",move |c| {
        let tx=c.transaction()?;
        let run=enqueue(&tx,&provision,&actor,preview,false)?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'recyclarr.sync',?2,?3)",params![actor,run,now()])?;
        tx.commit()?; Ok(run)
    }).await
}
pub(super) async fn tick(db: &Database) -> anyhow::Result<()> {
    db.write("recyclarr.tick", |c| {
        let tx=c.transaction()?;
        let settings=tx.query_row("SELECT r.provision_id,r.actor_id,r.paused,r.hour,r.next_run,r.timezone FROM recyclarr_settings r JOIN stack_provisions p ON p.id=r.provision_id WHERE p.state='complete'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,u32>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?))).optional()?;
        if let Some((provision,actor,paused,hour,mut due,zone))=settings {
            let current_zone=crate::timezones::server_zone(&tx)?;
            if current_zone.name()!=zone { due=next(&tx,hour)?; tx.execute("UPDATE recyclarr_settings SET next_run=?1,timezone=?2 WHERE provision_id=?3",params![due,current_zone.name(),provision])?; }
            let services=tx.prepare("SELECT id,kind FROM manager_services WHERE enabled=1 AND kind IN ('radarr','sonarr')")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for (service,kind) in services {
                let enrolled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recyclarr_targets WHERE service_id=?1)",[&service],|r|r.get(0))?;
                if !enrolled {
                    tx.execute("INSERT INTO recyclarr_targets(service_id,provision_id,trash_id,revision) VALUES (?1,?2,?3,?4)",params![service,provision,if kind=="radarr" {"05fbf054ac8ad0303335026cc2632f1a"} else {"c4cadd6b35b95f62c3d47a408e53e2f7"},id()])?;
                }
            }
            let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recyclarr_targets t JOIN manager_services m ON m.id=t.service_id WHERE t.provision_id=?1 AND m.enabled=1 AND (t.applied_revision IS NULL OR t.applied_revision<>t.revision) AND t.error IS NULL)",[&provision],|r|r.get(0))?;
            if !paused && (pending || due<=now()) {
                enqueue(&tx,&provision,&actor,false,true)?;
                if due<=now() { tx.execute("UPDATE recyclarr_settings SET next_run=?1 WHERE provision_id=?2",params![next(&tx,hour)?,provision])?; }
            }
        }
        tx.commit()?; Ok(())
    }).await
}
pub(super) async fn list(db: &Database) -> anyhow::Result<Value> {
    db.read("recyclarr.list", |c| {
        let settings=c.query_row("SELECT provision_id,paused,hour,next_run FROM recyclarr_settings",[],|r|Ok(json!({"provision_id":r.get::<_,String>(0)?,"paused":r.get::<_,bool>(1)?,"hour":r.get::<_,u32>(2)?,"next_run":r.get::<_,i64>(3)?}))).optional()?;
        let targets=c.prepare("SELECT t.service_id,m.kind,m.name,t.trash_id,t.revision,t.profile_id,t.profile_name,t.applied_revision,t.error,t.quality_sizes,t.groups,t.overrides,t.reset_scores FROM recyclarr_targets t JOIN manager_services m ON m.id=t.service_id WHERE m.enabled=1 ORDER BY m.kind,m.name")?.query_map([],|r|Ok(json!({"service_id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"trash_id":r.get::<_,String>(3)?,"revision":r.get::<_,String>(4)?,"profile_id":r.get::<_,Option<i64>>(5)?,"profile_name":r.get::<_,Option<String>>(6)?,"applied_revision":r.get::<_,Option<String>>(7)?,"error":r.get::<_,Option<String>>(8)?,"quality_sizes":r.get::<_,bool>(9)?,"groups":serde_json::from_str::<Value>(&r.get::<_,String>(10)?).unwrap_or_default(),"overrides":serde_json::from_str::<Value>(&r.get::<_,String>(11)?).unwrap_or_default(),"reset_scores":r.get::<_,bool>(12)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let runs=c.prepare("SELECT id,state,created_at,updated_at,error FROM recyclarr_runs ORDER BY created_at DESC LIMIT 30")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"created_at":r.get::<_,i64>(2)?,"updated_at":r.get::<_,i64>(3)?,"error":r.get::<_,Option<String>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"settings":settings,"targets":targets,"runs":runs,"timezone":crate::timezones::server_zone(c)?.name()}))
    }).await
}
pub(super) async fn details(db: &Database, id: String) -> anyhow::Result<Option<Value>> {
    db.read("recyclarr.details", move |c| {
        c.query_row("SELECT id,state,evidence,error FROM recyclarr_runs WHERE id=?1", [id], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"evidence":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or_default(),"error":r.get::<_,Option<String>>(3)?}))
        }).optional().map_err(Into::into)
    }).await
}
pub(super) async fn schedule(
    db: &Database,
    provision: String,
    actor: String,
    input: Schedule,
) -> anyhow::Result<()> {
    db.write("recyclarr.schedule",move |c| {
        c.execute("UPDATE recyclarr_settings SET paused=?1,hour=?2,next_run=?3,actor_id=?4,timezone=?6 WHERE provision_id=?5",params![input.paused,input.hour,next(c,input.hour)?,actor,provision,crate::timezones::server_zone(c)?.name()])?;
        Ok(())
    }).await
}
pub(super) async fn select(
    db: &Database,
    service: String,
    input: Selection,
    actor: String,
) -> anyhow::Result<String> {
    db.write("recyclarr.select",move |c| {
        let tx=c.transaction()?;
        let provision:String=tx.query_row("SELECT provision_id FROM recyclarr_targets WHERE service_id=?1",[&service],|r|r.get(0))?;
        tx.execute("INSERT INTO recyclarr_targets(service_id,provision_id,trash_id,revision,quality_sizes,groups,overrides,reset_scores) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(service_id,trash_id) DO UPDATE SET revision=excluded.revision,error=NULL,quality_sizes=excluded.quality_sizes,groups=excluded.groups,overrides=excluded.overrides,reset_scores=excluded.reset_scores",params![service,provision,input.trash_id,id(),input.quality_sizes,input.groups.to_string(),input.overrides.to_string(),input.reset_scores])?;
        let run=enqueue(&tx,&provision,&actor,false,false)?;
        tx.commit()?; Ok(run)
    }).await
}
pub(super) async fn run(db: &Database, run: String) -> anyhow::Result<(String, String, bool)> {
    db.read("recyclarr.run",move |c|Ok(c.query_row("SELECT provision_id,state,EXISTS(SELECT 1 FROM users WHERE id=actor_id AND role='admin') FROM recyclarr_runs WHERE id=?1",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?)).await
}
pub(super) async fn paused(db: &Database, provision: String) -> anyhow::Result<bool> {
    db.read("recyclarr.paused", move |c| {
        Ok(c.query_row(
            "SELECT paused FROM recyclarr_settings WHERE provision_id=?1",
            [provision],
            |r| r.get(0),
        )?)
    })
    .await
}
pub(super) async fn targets(db: &Database, provision: String) -> anyhow::Result<Vec<Target>> {
    db.read("recyclarr.targets",move |c|Ok(c.prepare("SELECT t.service_id,m.kind,t.trash_id,t.revision,t.profile_id,t.quality_sizes,t.groups,t.overrides,t.reset_scores FROM recyclarr_targets t JOIN manager_services m ON m.id=t.service_id WHERE t.provision_id=?1 AND m.enabled=1 ORDER BY m.kind,m.id,t.trash_id")?.query_map([provision],|r|Ok(Target {service_id:r.get(0)?,kind:r.get(1)?,trash_id:r.get(2)?,revision:r.get(3)?,profile_id:r.get(4)?,quality_sizes:r.get(5)?,groups:serde_json::from_str(&r.get::<_,String>(6)?).unwrap_or_default(),overrides:serde_json::from_str(&r.get::<_,String>(7)?).unwrap_or_default(),reset_scores:r.get(8)?}))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}
pub(super) async fn enroll(
    db: &Database,
    provision: String,
    service: String,
    catalog: Value,
) -> anyhow::Result<()> {
    db.write("recyclarr.enroll", move |c| {
        let tx = c.transaction()?;
        let items = catalog["items"].as_array().ok_or_else(|| anyhow::anyhow!("Missing guide catalog"))?;
        anyhow::ensure!(!items.is_empty(), "Guide catalog is empty");
        for profile in items {
            let trash = profile["trash_id"].as_str().ok_or_else(|| anyhow::anyhow!("Missing guide identity"))?;
            tx.execute("INSERT INTO recyclarr_targets(service_id,provision_id,trash_id,revision,guide_url) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(service_id,trash_id) DO UPDATE SET guide_url=excluded.guide_url", params![service,provision,trash,id(),profile["url"].as_str()])?;
        }
        tx.execute("DELETE FROM recyclarr_targets WHERE service_id=?1 AND trash_id NOT IN (SELECT json_extract(value,'$.trash_id') FROM json_each(?2))",params![service,catalog["items"].to_string()])?;
        tx.commit()?;
        Ok(())
    }).await
}
pub(super) async fn profiles(db: &Database, service: String) -> anyhow::Result<Vec<Profile>> {
    db.read("recyclarr.profiles", move |c| {
        Ok(c.prepare("SELECT t.trash_id,t.profile_id,t.guide_url FROM recyclarr_targets t JOIN stack_provisions p ON p.id=t.provision_id WHERE t.service_id=?1 AND t.profile_id IS NOT NULL AND p.state='complete' ORDER BY t.trash_id")?
            .query_map([service], |r| Ok(Profile { trash_id:r.get(0)?, profile_id:r.get(1)?, url:r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }).await
}
pub(super) async fn progress(
    db: &Database,
    run: String,
    stage: String,
    evidence: Value,
    error: Option<String>,
) -> anyhow::Result<()> {
    db.write("recyclarr.progress",move |c| {
        let tx=c.transaction()?;
        let previous:String=tx.query_row("SELECT state FROM recyclarr_runs WHERE id=?1",[&run],|r|r.get(0))?;
        let stage = if previous == "partial" && stage == "blocked" { previous.clone() } else { stage };
        tx.execute("UPDATE recyclarr_runs SET state=?1,evidence=CASE WHEN ?2='{}' THEN evidence ELSE ?2 END,error=?3,updated_at=?4 WHERE id=?5",params![stage,evidence.to_string(),error,now(),run])?;
        if previous!=stage && matches!(stage.as_str(),"complete"|"blocked"|"partial"|"cancelled") {
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) SELECT actor_id,?1,id,?2 FROM recyclarr_runs WHERE id=?3",params![format!("recyclarr.{stage}"),now(),run])?;
        }
        tx.commit()?; Ok(())
    }).await
}
pub(super) async fn applied(
    db: &Database,
    target: Target,
    generation: String,
    profile: i64,
    name: String,
) -> anyhow::Result<bool> {
    db.write("recyclarr.applied",move |c| {
        let tx=c.transaction()?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recyclarr_targets t JOIN manager_services m ON m.id=t.service_id WHERE t.service_id=?1 AND t.trash_id=?4 AND t.revision=?2 AND m.generation=?3 AND m.enabled=1)",params![target.service_id,target.revision,generation,target.trash_id],|r|r.get(0))?;
        if valid {
            let preferred=if target.kind=="radarr" {"05fbf054ac8ad0303335026cc2632f1a"} else {"c4cadd6b35b95f62c3d47a408e53e2f7"};
            tx.execute("UPDATE manager_services SET defaults=json_set(defaults,'$.quality_profile',?1) WHERE id=?2 AND json_extract(defaults,'$.quality_profile_trash_id')=?3",params![profile,target.service_id,target.trash_id])?;
            if target.trash_id==preferred {
                tx.execute("UPDATE manager_services SET defaults=json_set(defaults,'$.quality_profile',?1,'$.quality_profile_trash_id',?4,'$.root_folder',?2,'$.monitored',json('true')) WHERE id=?3 AND json_extract(defaults,'$.quality_profile') IS NULL",params![profile,canonical_root(&target.kind),target.service_id,target.trash_id])?;
            }
            tx.execute("UPDATE recyclarr_targets SET profile_id=?1,profile_name=?2,applied_revision=revision,error=NULL WHERE service_id=?3 AND trash_id=?4",params![profile,name,target.service_id,target.trash_id])?;
        }
        tx.commit()?; Ok(valid)
    }).await
}
pub(super) async fn failure(
    db: &Database,
    targets: Vec<Target>,
    error: String,
) -> anyhow::Result<()> {
    db.write("recyclarr.failure", move |c| {
        for target in targets {
            c.execute(
                "UPDATE recyclarr_targets SET error=?1 WHERE service_id=?2 AND revision=?3 AND trash_id=?4",
                params![error, target.service_id, target.revision,target.trash_id],
            )?;
        }
        Ok(())
    })
    .await
}
pub(super) async fn retry(db: &Database, job: String, seconds: i64) -> anyhow::Result<()> {
    db.write("recyclarr.retry", move |c| {
        c.execute(
            "UPDATE jobs SET state='queued',available_at=?1,started_at=NULL WHERE id=?2",
            params![now() + seconds, job],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn review(db: &Database, review: String, value: Value) -> anyhow::Result<()> {
    db.write("recyclarr.review",move |c| { c.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("recyclarr.review.{review}"),value.to_string()])?; Ok(()) }).await
}
pub(super) async fn adopt(db: &Database, actor: String, input: Adoption) -> anyhow::Result<()> {
    db.write("recyclarr.adopt",move |c| {
        let tx=c.transaction()?;
        anyhow::ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM stack_provisions WHERE kind='recyclarr')",[],|r|r.get::<_,bool>(0))?,"Recyclarr already installed");
        let raw:String=tx.query_row("SELECT value FROM settings WHERE key=?1",[format!("recyclarr.review.{}",input.review_id)],|r|r.get(0))?;
        let review:Value=serde_json::from_str(&raw)?;
        anyhow::ensure!(review["container_id"]==input.container_id && review["actor_id"]==actor,"Review does not match this transfer");
        anyhow::ensure!(now()-review["created_at"].as_i64().unwrap_or(0)<1800,"Review expired; review the transfer again");
        tx.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,container_id,origin,created_at,updated_at) VALUES (?1,'recyclarr',?2,'queued',?3,'adopted',?4,?4)",params![input.review_id,actor,input.container_id,now()])?;
        tx.execute("INSERT INTO recyclarr_settings(provision_id,actor_id,paused,next_run,timezone) VALUES (?1,?2,1,?3,?4)",params![input.review_id,actor,next(&tx,4)?,crate::timezones::server_zone(&tx)?.name()])?;
        for target in review["targets"].as_array().ok_or_else(||anyhow::anyhow!("Missing import targets"))? {
            anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND generation=?2 AND enabled=1)",params![target["service_id"].as_str(),target["generation"].as_str()],|r|r.get::<_,bool>(0))?,"Imported target changed; review the transfer again");
            tx.execute("INSERT INTO recyclarr_targets(service_id,provision_id,trash_id,revision,quality_sizes,groups,reset_scores,overrides) VALUES (?1,?2,?3,?4,?5,?6,0,?7)",params![target["service_id"].as_str(),input.review_id,target["trash_id"].as_str(),id(),target["quality_sizes"].as_bool().unwrap_or(false),target["groups"].to_string(),target["overrides"].to_string()])?;
        }
        tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'stack.install',?2,?3,'queued',?4,?4)",params![id(),json!({"id":input.review_id}).to_string(),format!("stack:{}",input.review_id),now()])?;
        tx.commit()?; Ok(())
    }).await
}
