use super::{Candidate, Generation, Job, Settings, Tool};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use thelxinoe_core::now;
use thelxinoe_database::Database;

pub async fn inventory(db: &Database) -> Result<Vec<Tool>> {
    db.read("tools.inventory", |db| {
        let mut query = db.prepare("SELECT id,policy,channel,pinned,revision,installed,previous,held,candidate,checked_at,next_check_at,check_error,integrity_error FROM server_tools ORDER BY CASE id WHEN 'yt-dlp' THEN 0 WHEN 'deno' THEN 1 WHEN 'streamlink' THEN 2 ELSE 3 END")?;
        let rows = query.query_map([], |r| Ok(Tool {
            id:r.get(0)?, policy:r.get(1)?, channel:r.get(2)?, pinned:r.get(3)?, revision:r.get(4)?,
            installed:r.get(5)?, previous:r.get(6)?, held:r.get(7)?, candidate:None, candidate_json:r.get(8)?,
            checked_at:r.get(9)?, next_check_at:r.get(10)?, check_error:r.get(11)?, integrity_error:r.get(12)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|mut row| { row.candidate=row.candidate_json.as_deref().map(serde_json::from_str).transpose()?; Ok(row) }).collect()
    }).await
}

pub async fn generations(db: &Database) -> Result<Vec<Generation>> {
    db.read("tools.generations", |db| {
        let mut query = db.prepare("SELECT manifest FROM tool_generations")?;
        let values = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
            .into_iter()
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .collect()
    })
    .await
}

pub async fn jobs(db: &Database) -> Result<Vec<Job>> {
    db.read("tools.jobs", |db| {
        let mut query=db.prepare("SELECT id,tool,action,candidate,manual,revision,generation,stage,received,total,reason,error,validation,retry_at FROM tool_jobs ORDER BY (stage NOT IN ('complete','failed','canceled')) DESC,(stage NOT IN ('complete','failed','canceled') AND action IN ('bootstrap','validate')) DESC,(stage NOT IN ('complete','failed','canceled') AND action!='check') DESC,created_at DESC,rowid DESC LIMIT 100")?;
        let rows=query.query_map([], |r| Ok(Job { id:r.get(0)?,tool:r.get(1)?,action:r.get(2)?,candidate:None,candidate_json:r.get(3)?,manual:r.get(4)?,revision:r.get(5)?,generation:r.get(6)?,stage:r.get(7)?,received:r.get::<_,i64>(8)? as u64,total:r.get::<_,i64>(9)? as u64,reason:r.get(10)?,error:r.get(11)?,validation:r.get::<_,Option<String>>(12)?.and_then(|v|serde_json::from_str(&v).ok()),retry_at:r.get(13)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|mut row| { row.candidate=row.candidate_json.as_deref().map(serde_json::from_str).transpose()?; Ok(row) }).collect()
    }).await
}

pub async fn settings(db: &Database, id: String, input: Settings, actor: String) -> Result<()> {
    db.write("tools.settings", move |db| {
        let tx=db.transaction()?;
        let channel:String=tx.query_row("SELECT channel FROM server_tools WHERE id=?1",[&id],|r|r.get(0))?;
        tx.execute("UPDATE server_tools SET policy=?2,channel=?3,pinned=?4,revision=revision+1,candidate=CASE WHEN channel=?3 THEN candidate ELSE NULL END,next_check_at=CASE WHEN channel=?3 THEN next_check_at ELSE 0 END,check_error=CASE WHEN channel=?3 THEN check_error ELSE NULL END WHERE id=?1",params![id,input.policy,input.channel,input.pinned])?;
        if channel!=input.channel { tx.execute("UPDATE tool_jobs SET stage='canceled',reason='Channel changed',updated_at=?2 WHERE tool=?1 AND action!='validate' AND stage NOT IN ('complete','failed','canceled')",params![id,now()])?; }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'tools.settings',?2,?3)",params![actor,id,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub async fn enqueue(
    db: &Database,
    tool: Tool,
    action: String,
    candidate: Option<Candidate>,
    manual: bool,
    actor: Option<String>,
) -> Result<String> {
    db.write("tools.enqueue", move |db| {
        let tx=db.transaction()?;
        if let Some((id, existing, pending))=tx.query_row("SELECT id,action,candidate FROM tool_jobs WHERE tool=?1 AND CASE WHEN action='check' THEN 0 WHEN action IN ('bootstrap','validate') THEN 1 ELSE 2 END = CASE WHEN ?2='check' THEN 0 WHEN ?2 IN ('bootstrap','validate') THEN 1 ELSE 2 END AND stage NOT IN ('complete','failed','canceled')",params![tool.id,action],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional()? {
            ensure!(existing==action && pending.as_deref().map(serde_json::from_str::<Candidate>).transpose()?==candidate,"Another operation is pending for this tool");
            if manual { tx.execute("UPDATE tool_jobs SET manual=1,retry_at=0 WHERE id=?1",[&id])?; }
            tx.commit()?;
            return Ok(id);
        }
        let revision:i64=tx.query_row("SELECT revision FROM server_tools WHERE id=?1",[&tool.id],|r|r.get(0))?;
        ensure!(revision==tool.revision,"Tool settings changed; refresh and retry");
        let id=thelxinoe_core::id();
        tx.execute("INSERT INTO tool_jobs(id,tool,action,candidate,manual,revision,stage,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,'queued',?7,?7)",params![id,tool.id,action,candidate.map(|v|serde_json::to_string(&v)).transpose()?,manual,revision,now()])?;
        if action=="rollback" {
            let group:Option<String>=tx.query_row("SELECT members FROM tool_activation_groups,json_each(members) m WHERE json_extract(m.value,'$.tool')=?1 AND json_extract(m.value,'$.after')=?2 ORDER BY created_at DESC LIMIT 1",params![tool.id,tool.installed],|r|r.get(0)).optional()?;
            let members:Vec<serde_json::Value>=group.map(|v|serde_json::from_str(&v)).transpose()?.unwrap_or_default();
            for member in members {
                let key=member["tool"].as_str().context("Invalid activation group")?;
                if key==tool.id { continue; }
                let (current,previous,pinned,revision,channel):(Option<String>,Option<String>,bool,i64,String)=tx.query_row("SELECT installed,previous,pinned,revision,channel FROM server_tools WHERE id=?1",[key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
                ensure!(!pinned && current.as_deref()==member["after"].as_str() && previous.as_deref()==member["before"].as_str(),"Unpin the dependency and restore the matching group before rollback");
                let manifest:String=tx.query_row("SELECT manifest FROM tool_generations WHERE id=?1",[previous],|r|r.get(0))?;
                let generation:Generation=serde_json::from_str(&manifest)?;
                ensure!(generation.candidate.channel==channel,"Dependency channel changed before rollback");
                tx.execute("INSERT INTO tool_jobs(id,tool,action,candidate,manual,revision,stage,created_at,updated_at) VALUES (?1,?2,'rollback',?3,1,?4,'queued',?5,?5)",params![thelxinoe_core::id(),key,serde_json::to_string(&generation.candidate)?,revision,now()])?;
            }
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![actor,format!("tools.{action}"),tool.id,now()])?;
        tx.commit()?;Ok(id)
    }).await
}

pub async fn discover(
    db: &Database,
    tool: Tool,
    candidate: Option<Candidate>,
    error: Option<String>,
    retry_at: Option<i64>,
) -> Result<()> {
    db.write("tools.discovery", move |db| {
        let success=error.is_none();
        let retry=retry_at.unwrap_or(now()+3600).clamp(now()+60,now()+86400);
        db.execute("UPDATE server_tools SET candidate=CASE WHEN ?4 THEN ?2 ELSE candidate END,checked_at=CASE WHEN ?4 THEN ?3 ELSE checked_at END,next_check_at=?5,check_error=?6 WHERE id=?1 AND channel=?7",params![tool.id,candidate.map(|v|serde_json::to_string(&v)).transpose()?,now(),success,if success{now()+86400}else{retry},error,tool.channel])?; Ok(())
    }).await
}

pub async fn progress(
    db: &Database,
    id: String,
    stage: String,
    received: u64,
    total: u64,
    reason: Option<String>,
    error: Option<String>,
) -> Result<()> {
    db.write("tools.progress", move |db| {
        db.execute("UPDATE tool_jobs SET stage=?2,received=?3,total=?4,reason=?5,error=?6,updated_at=?7 WHERE id=?1 AND stage NOT IN ('complete','failed','canceled')",params![id,stage,i64::try_from(received)?,i64::try_from(total)?,reason,error,now()])?; Ok(())
    }).await
}

pub async fn prepared(db: &Database, job: String, generation: Generation) -> Result<()> {
    db.write("tools.prepared", move |db| {
        let tx=db.transaction()?;
        tx.execute("INSERT INTO tool_generations(id,tool,manifest) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![generation.id,generation.candidate.tool,serde_json::to_string(&generation)?])?;
        tx.execute("UPDATE tool_jobs SET generation=?2,stage='waiting',updated_at=?3 WHERE id=?1 AND stage NOT IN ('complete','failed','canceled')",params![job,generation.id,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub async fn validation(
    db: &Database,
    id: String,
    result: serde_json::Value,
    retry_at: i64,
) -> Result<()> {
    db.write("tools.validation", move |db| {
        db.execute(
            "UPDATE tool_jobs SET validation=?2,retry_at=?3 WHERE id=?1",
            params![id, serde_json::to_string(&result)?, retry_at],
        )?;
        Ok(())
    })
    .await
}

pub async fn activate(db: &Database, selections: Vec<(Job, Tool, String)>) -> Result<bool> {
    db.write("tools.activate", move |db| {
        let tx=db.transaction()?;
        let mut members=Vec::new();
        for (job,tool,generation) in selections {
            let (revision,pinned,current):(i64,bool,Option<String>)=tx.query_row("SELECT revision,pinned,installed FROM server_tools WHERE id=?1",[&tool.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            if revision!=tool.revision || pinned || current!=tool.installed { return Ok(false); }
            let (stage,manual):(String,bool)=tx.query_row("SELECT stage,manual FROM tool_jobs WHERE id=?1",[&job.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if stage!="waiting" { return Ok(false); }
            if !manual {
                let default:crate::product::Policy=tx.query_row("SELECT value FROM settings WHERE key='product.policy'",[],|r|r.get::<_,String>(0)).optional()?.map(|v|serde_json::from_str(&v)).transpose()?.unwrap_or_default();
                let effective=if tool.policy=="inherit"{&default.policy}else{&tool.policy};
                if effective!="automatic" || !crate::timezones::maintenance_window(&tx,chrono::Utc::now(),u32::from(default.window_start),u32::from(default.window_end))? { return Ok(false); }
            }
            let manifest:String=tx.query_row("SELECT manifest FROM tool_generations WHERE id=?1 AND tool=?2",params![generation,tool.id],|r|r.get(0))?;
            let selected:Generation=serde_json::from_str(&manifest)?;
            ensure!(selected.candidate.channel==tool.channel,"Selected channel changed");
            if !manual && Some(&selected.candidate.id)==tool.held.as_ref() { return Ok(false); }
            let held=if job.action=="rollback" {
                tool.installed.as_ref().map(|id|->Result<String>{let value:String=tx.query_row("SELECT manifest FROM tool_generations WHERE id=?1",[id],|r|r.get(0))?;Ok(serde_json::from_str::<Generation>(&value)?.candidate.id)}).transpose()?
            }else{tool.held};
            tx.execute("UPDATE server_tools SET previous=installed,installed=?2,held=?3,integrity_error=NULL WHERE id=?1",params![tool.id,generation,held])?;
            tx.execute("UPDATE tool_jobs SET stage='complete',reason=NULL,error=NULL,updated_at=?2 WHERE id=?1",params![job.id,now()])?;
            tx.execute("INSERT INTO audit(action,target,created_at) VALUES ('tools.activated',?1,?2)",params![generation,now()])?;
            members.push(serde_json::json!({"tool":tool.id,"before":tool.installed,"after":generation}));
        }
        tx.execute("INSERT INTO tool_activation_groups(id,members,created_at) VALUES (?1,?2,?3)",params![thelxinoe_core::id(),serde_json::to_string(&members)?,now()])?;
        tx.commit()?;Ok(true)
    }).await
}

pub async fn rebuild(db: &Database, previous: String, generation: Generation) -> Result<()> {
    db.write("tools.rebuild_runtime", move |db| {
        let tx=db.transaction()?;
        tx.execute("INSERT INTO tool_generations(id,tool,manifest) VALUES (?1,?2,?3)",params![generation.id,generation.candidate.tool,serde_json::to_string(&generation)?])?;
        tx.execute("UPDATE server_tools SET installed=?2,integrity_error=NULL WHERE id=?1 AND installed=?3",params![generation.candidate.tool,generation.id,previous])?;
        tx.commit()?;Ok(())
    }).await
}

pub async fn integrity(db: &Database, id: String, error: Option<String>) -> Result<()> {
    db.write("tools.integrity", move |db| {
        db.execute(
            "UPDATE server_tools SET integrity_error=?2 WHERE id=?1",
            params![id, error],
        )?;
        Ok(())
    })
    .await
}

pub async fn recovery_group(db: &Database, tool: Tool) -> Result<Vec<serde_json::Value>> {
    db.read("tools.recovery_group",move |db| {
        let group:Option<String>=db.query_row("SELECT members FROM tool_activation_groups,json_each(members) m WHERE json_extract(m.value,'$.tool')=?1 AND json_extract(m.value,'$.after')=?2 ORDER BY created_at DESC,tool_activation_groups.rowid DESC LIMIT 1",params![tool.id,tool.installed],|r|r.get(0)).optional()?;
        Ok(group.map(|v|serde_json::from_str(&v)).transpose()?.unwrap_or_else(||vec![serde_json::json!({"tool":tool.id,"before":tool.previous,"after":tool.installed})]))
    }).await
}

pub async fn restore_previous(db: &Database, tools: Vec<Tool>, reason: String) -> Result<bool> {
    db.write("tools.restore_previous",move |db| {
        let tx=db.transaction()?;
        for tool in tools {
            let (current,previous,revision,pinned):(Option<String>,Option<String>,i64,bool)=tx.query_row("SELECT installed,previous,revision,pinned FROM server_tools WHERE id=?1",[&tool.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            if current!=tool.installed || previous!=tool.previous || previous.is_none() || revision!=tool.revision || pinned { return Ok(false); }
            let manifest:String=tx.query_row("SELECT manifest FROM tool_generations WHERE id=?1",[&current],|r|r.get(0))?;
            let failed:Generation=serde_json::from_str(&manifest)?;
            tx.execute("UPDATE server_tools SET installed=?2,previous=?3,held=?4,integrity_error=NULL WHERE id=?1",params![tool.id,previous,current,failed.candidate.id])?;
            tx.execute("UPDATE tool_jobs SET stage='canceled',reason='Recovered the previous working version',updated_at=?2 WHERE tool=?1 AND action!='check' AND stage NOT IN ('complete','failed','canceled')",params![tool.id,now()])?;
            tx.execute("INSERT INTO tool_jobs(id,tool,action,manual,revision,generation,stage,reason,created_at,updated_at) VALUES (?1,?2,'recovery',0,?3,?4,'complete',?5,?6,?6)",params![thelxinoe_core::id(),tool.id,tool.revision,previous,reason,now()])?;
            tx.execute("INSERT INTO audit(action,target,created_at) VALUES ('tools.recovered',?1,?2)",params![tool.id,now()])?;
        }
        tx.commit()?;Ok(true)
    }).await
}

pub async fn recover(db: &Database) -> Result<()> {
    db.write("tools.recover", |db| {
        db.execute("UPDATE tool_jobs SET stage=CASE WHEN generation IS NULL THEN 'queued' ELSE 'waiting' END WHERE stage NOT IN ('complete','failed','canceled')",[])?;
        Ok(())
    }).await
}

pub async fn retained(db: &Database) -> Result<Vec<String>> {
    db.read("tools.retained", |db| {
        let mut query=db.prepare("SELECT installed FROM server_tools WHERE installed IS NOT NULL UNION SELECT previous FROM server_tools WHERE previous IS NOT NULL UNION SELECT generation FROM tool_jobs WHERE generation IS NOT NULL AND stage NOT IN ('complete','failed','canceled') UNION SELECT value FROM youtube_downloads,json_each(youtube_downloads.tools,'$.generations') WHERE youtube_downloads.state NOT IN ('ready','deleted')")?;
        Ok(query.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?)
    }).await
}

pub async fn remove_generation(db: &Database, id: String) -> Result<()> {
    db.write("tools.collect", move |db| {
        db.execute("DELETE FROM tool_generations WHERE id=?1", [id])?;
        Ok(())
    })
    .await
}

pub async fn busy(db: &Database, tool: String) -> Result<bool> {
    db.read("tools.activity", move |db| {
        let media=tool=="ffmpeg";
        let youtube=tool=="yt-dlp" || tool=="deno" || media;
        let busy:bool=db.query_row("SELECT (?1 AND (EXISTS(SELECT 1 FROM playback_sessions WHERE state IN ('ready','playing','paused') AND updated_at>?3-120) OR EXISTS(SELECT 1 FROM jobs WHERE kind='library.scan' AND state='running') OR EXISTS(SELECT 1 FROM segment_analysis WHERE state='running'))) OR (?2 AND EXISTS(SELECT 1 FROM youtube_downloads WHERE state='downloading'))",params![media,youtube,now()],|r|r.get(0)).context("Read tool activity")?;
        Ok(busy)
    }).await
}
