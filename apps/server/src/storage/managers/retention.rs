//! Database operations for managers.retention.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn revalidate_operation(
    operation: String,
    db: &Database,
    automatic: bool,
) -> anyhow::Result<bool> {
    db.read("managers.retention.revalidate_operation", move|db|{
        let candidate=db.query_row("SELECT media_id,stamp,due_at,state FROM retention_candidates WHERE operation_id=?1",[operation],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional()?;
        let Some((media,stamp,due,status))=candidate else {return Ok(true)};
        Ok(["pending","executing"].contains(&status.as_str()) && eligibility(db,&media)?.is_some_and(|e|e.stamp==stamp && (!automatic||(e.automatic&&now()>=due))))
    }).await
}

pub(super) async fn list(db: &Database) -> anyhow::Result<Value> {
    db.read("managers.retention.list", |db|{
        let policies=db.prepare("SELECT domain,enabled,grace_seconds,exclude_specials,trigger_users FROM retention_policies")?.query_map([],|r|Ok(json!({"domain":r.get::<_,String>(0)?,"enabled":r.get::<_,bool>(1)?,"grace_seconds":r.get::<_,i64>(2)?,"exclude_specials":r.get::<_,bool>(3)?,"trigger_users":serde_json::from_str::<Value>(&r.get::<_,String>(4)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=db.prepare("SELECT c.id,c.media_id,m.title,c.state,c.eligible_at,c.due_at,c.error,u.username FROM retention_candidates c JOIN media m ON m.id=c.media_id LEFT JOIN users u ON u.id=c.trigger_user ORDER BY c.eligible_at DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"media_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"eligible_at":r.get::<_,i64>(4)?,"due_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?,"trigger_user":r.get::<_,Option<String>>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let users=db.prepare("SELECT id,username FROM users ORDER BY username")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let roots=db.prepare("SELECT id,name,automatic_unmanaged_deletion FROM library_roots WHERE kind IN ('movies','shows')")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"automatic_unmanaged_deletion":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"policies":policies,"items":items,"users":users,"roots":roots}))
    }).await
}

pub(super) async fn policy(
    db: &Database,
    domain: String,
    input: Policy,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<bool> {
    db.write("managers.retention.policy", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let users=json!(input.trigger_users).to_string();
        let count=tx.query_row("SELECT COUNT(*) FROM users WHERE id IN (SELECT value FROM json_each(?1))",[&users],|r|r.get::<_,i64>(0))?;
        if count!=input.trigger_users.len() as i64{return Ok(false);}
        tx.execute("UPDATE retention_policies SET enabled=?1,grace_seconds=?2,exclude_specials=?3,trigger_users=?4,updated_at=MAX(updated_at+1,?5) WHERE domain=?6",params![input.enabled,input.grace_seconds,input.exclude_specials,users,now(),domain])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.policy',?2,?3)",params![p.user.id,domain,now()])?;tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn root_policy(
    db: &Database,
    key: String,
    input: RootPolicy,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<bool> {
    db.write("managers.retention.root_policy", move|db|{let tx=db.transaction()?;let count=tx.execute("UPDATE library_roots SET automatic_unmanaged_deletion=?1 WHERE id=?2 AND kind IN ('movies','shows')",params![input.automatic_unmanaged_deletion,key])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.root',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(count>0)}).await
}

pub(super) async fn evaluate_all_read_retention_policies(db: &Database) -> anyhow::Result<bool> {
    db.read(
        "managers.retention.evaluate_all_read_retention_policies",
        |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM retention_policies WHERE enabled=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?)
        },
    )
    .await
}

pub(super) async fn evaluate_all_write_media(
    db: &Database,
) -> anyhow::Result<Vec<(String, Eligible)>> {
    db.write("managers.retention.evaluate_all_write_media", |db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids=tx.prepare("SELECT id FROM media WHERE kind IN ('movie','season')")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut eligible=Vec::new();
        for media in ids {
            if let Some(e)=eligibility(&tx,&media)? {
                tx.execute("UPDATE media_operations SET state='blocked',error='Retention eligibility changed' WHERE state='pending' AND id IN (SELECT operation_id FROM retention_candidates WHERE media_id=?1 AND stamp<>?2 AND state='pending')",params![media,e.stamp])?;
                tx.execute("UPDATE retention_candidates SET state='blocked',error='Retention eligibility changed; a fresh grace period is required' WHERE media_id=?1 AND stamp<>?2 AND state='pending'",params![media,e.stamp])?;
                let exists=tx.query_row("SELECT EXISTS(SELECT 1 FROM retention_candidates WHERE media_id=?1 AND (stamp=?2 OR state IN ('pending','executing')))",params![media,e.stamp],|r|r.get::<_,bool>(0))?;
                if !exists {eligible.push((media,e));}
            }
        }
        tx.commit()?;
        Ok(eligible)
    }).await
}

pub(super) async fn evaluate_all_write_retention_candidates(
    media: String,
    e: Eligible,
    operation: Value,
    key: String,
    actor: Option<String>,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.retention.evaluate_all_write_retention_candidates", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,trigger_user,state,eligible_at,due_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7)",params![key,media,operation["id"].as_str(),e.stamp,e.user,now(),now()+e.grace])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.pending',?2,?3)",params![actor,key,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn action(
    db: &Database,
    key: String,
    action: String,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<bool> {
    db.write("managers.retention.action", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let record=tx.query_row("SELECT media_id,operation_id FROM retention_candidates WHERE id=?1 AND state='pending'",[&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;let Some((media,operation))=record else{return Ok(false)};
        if action=="keep" {tx.execute("INSERT INTO media_protection VALUES (?1,1) ON CONFLICT(media_id) DO UPDATE SET keep=1",[&media])?;}
        tx.execute("UPDATE retention_candidates SET state='cancelled' WHERE id=?1",[&key])?;
        tx.execute("UPDATE media_operations SET state='blocked',error='Retention cancelled' WHERE id=?1 AND state='pending'",[operation])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![p.user.id,format!("retention.{action}"),key,now()])?;tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn delete_locked_read_retention_candidates(
    lookup: String,
    db: &Database,
) -> anyhow::Result<Option<String>> {
    db.read(
        "managers.retention.delete_locked_read_retention_candidates",
        move |db| {
            Ok(db
                .query_row(
                    "SELECT operation_id FROM retention_candidates WHERE id=?1 AND state='pending'",
                    [lookup],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        },
    )
    .await
}

pub(super) async fn delete_locked_write_retention_candidates(
    key: String,
    message: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write(
        "managers.retention.delete_locked_write_retention_candidates",
        move |db| {
            db.execute(
                "UPDATE retention_candidates SET state='blocked',error=?1 WHERE id=?2",
                params![message, key],
            )?;
            Ok(())
        },
    )
    .await
}

pub(super) async fn run(db: &Database) -> anyhow::Result<Vec<String>> {
    db.read("managers.retention.run", |db| {
        let items = db
            .prepare(
                "SELECT id,media_id FROM retention_candidates WHERE state='pending' AND due_at<=?1",
            )?
            .query_map([now()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut ready = Vec::new();
        for (key, media) in items {
            if eligibility(db, &media)?.is_some_and(|e| e.automatic) {
                ready.push(key);
            }
        }
        Ok(ready)
    })
    .await
}

pub(super) async fn exclude_read_retention_candidates(
    operation: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("managers.retention.exclude_read_retention_candidates", move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM retention_candidates WHERE operation_id=?1 AND state IN ('pending','executing'))",[operation],|r|r.get::<_,bool>(0))?)).await
}

pub(super) async fn exclude_write_retention_exclusions(
    created: bool,
    eid: i64,
    key: String,
    external: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.retention.exclude_write_retention_exclusions", move|db|{db.execute("INSERT INTO retention_exclusions(service_id,external_id,exclusion_id,created_here) VALUES (?1,?2,?3,?4) ON CONFLICT(service_id,external_id) DO UPDATE SET exclusion_id=excluded.exclusion_id,created_here=CASE WHEN retention_exclusions.exclusion_id=excluded.exclusion_id THEN MAX(retention_exclusions.created_here,excluded.created_here) ELSE excluded.created_here END",params![key,external,eid,created])?;Ok(())}).await
}

pub(super) async fn reacquire_read_retention_candidates(
    sid: String,
    identity: String,
    db: &Database,
) -> anyhow::Result<(Vec<(String, String)>, Option<(i64, bool)>)> {
    db.read("managers.retention.reacquire_read_retention_candidates", move |db| {
        let records=db.prepare("SELECT c.id,o.targets FROM retention_candidates c JOIN media_operations o ON o.id=c.operation_id WHERE c.state IN ('pending','executing','complete','blocked')")?
            .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let exclusion=db.query_row("SELECT exclusion_id,created_here FROM retention_exclusions WHERE service_id=?1 AND external_id=?2",params![sid,identity],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional()?;
        Ok((records,exclusion))
    }).await
}

pub(super) async fn reacquire_write_media_operations(
    affected: Vec<String>,
    sid: String,
    identity: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("managers.retention.reacquire_write_media_operations", move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for key in affected {
            tx.execute("UPDATE media_operations SET state='blocked',error='Retention reversed by reacquisition' WHERE state='pending' AND id=(SELECT operation_id FROM retention_candidates WHERE id=?1)",[&key])?;
            tx.execute("UPDATE retention_candidates SET state='cancelled',error='Reacquisition requested' WHERE id=?1",[key])?;
        }
        tx.execute("DELETE FROM retention_exclusions WHERE service_id=?1 AND external_id=?2",params![sid,identity])?;
        tx.commit()?;
        Ok(())
    }).await
}

pub(super) fn eligibility(
    db: &rusqlite::Connection,
    media: &str,
) -> anyhow::Result<Option<Eligible>> {
    let Some((kind, root, parent, number)) = db
        .query_row(
            "SELECT kind,root_id,parent_id,sort_number FROM media WHERE id=?1",
            [media],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let domain = match kind.as_str() {
        "movie" => "movies",
        "season" => "shows",
        _ => return Ok(None),
    };
    let (enabled,grace,specials,users,revision)=db.query_row("SELECT enabled,grace_seconds,exclude_specials,trigger_users,updated_at FROM retention_policies WHERE domain=?1",[domain],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,i64>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?)))?;
    if !enabled || (kind == "season" && specials && number == Some(0)) {
        return Ok(None);
    };
    let protected=db.query_row("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id), ancestors(id,parent_id) AS (SELECT id,parent_id FROM media WHERE id IN (SELECT id FROM tree) UNION SELECT m.id,m.parent_id FROM media m JOIN ancestors a ON a.parent_id=m.id) SELECT EXISTS(SELECT 1 FROM media_protection p JOIN ancestors a ON a.id=p.media_id WHERE p.keep=1)",[media],|r|r.get::<_,bool>(0))?;
    if protected {
        return Ok(None);
    };
    let files=db.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM media WHERE id=?1 UNION ALL SELECT m.id FROM media m JOIN tree t ON m.parent_id=t.id) SELECT DISTINCT f.id,f.generation,f.ownership FROM tree JOIN media_sources ms ON ms.media_id=tree.id JOIN media_files f ON f.id=ms.file_id WHERE f.present=1 ORDER BY f.id")?.query_map([media],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if files.is_empty()
        || files
            .iter()
            .any(|f| !["managed", "unmanaged"].contains(&f.2.as_str()))
    {
        return Ok(None);
    };
    let active=db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE file_id IN (SELECT value FROM json_each(?1)) AND state IN ('ready','playing','paused') AND updated_at>?2)",params![json!(files.iter().map(|f|&f.0).collect::<Vec<_>>()).to_string(),now()-120],|r|r.get::<_,bool>(0))?;
    if active {
        return Ok(None);
    };
    let units = if kind == "movie" {
        vec![media.to_owned()]
    } else {
        let Some(show) = parent else { return Ok(None) };
        let episodes = db
            .prepare("SELECT id FROM media WHERE parent_id=?1 AND kind='episode' ORDER BY id")?
            .query_map([media], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !complete_season(db, &show, &episodes, specials)? {
            return Ok(None);
        };
        episodes
    };
    let users: Vec<String> = serde_json::from_str(&users)?;
    for user in users {
        let mut watched = Vec::new();
        for unit in &units {
            let value=db.query_row("SELECT s.watched_revision FROM media_state s JOIN users u ON u.id=s.user_id WHERE s.user_id=?1 AND s.media_id=?2 AND s.watched=1",params![user,unit],|r|r.get::<_,i64>(0)).optional()?;
            if let Some(at) = value {
                watched.push((unit.clone(), at));
            }
        }
        if watched.len() == units.len() {
            let optin = db.query_row(
                "SELECT automatic_unmanaged_deletion FROM library_roots WHERE id=?1",
                [&root],
                |r| r.get::<_, bool>(0),
            )?;
            let stamp = Sha256::digest(serde_json::to_vec(&(
                media, revision, &files, &user, &watched,
            ))?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
            return Ok(Some(Eligible {
                stamp,
                user,
                grace,
                automatic: optin || files.iter().all(|f| f.2 == "managed"),
            }));
        }
    }
    Ok(None)
}

pub(super) fn complete_season(
    db: &rusqlite::Connection,
    show: &str,
    episodes: &[String],
    exclude_specials: bool,
) -> anyhow::Result<bool> {
    if episodes.is_empty() {
        return Ok(false);
    };
    let binding=db.query_row("SELECT m.metadata,b.service_id,b.external_id,b.refreshed_at FROM media m JOIN metadata_bindings b ON b.media_id=m.id JOIN manager_services s ON s.id=b.service_id AND s.kind='sonarr' AND s.enabled=1 AND s.generation=b.service_generation WHERE m.id=?1",[show],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?))).optional()?;
    let Some((metadata, service, series, refreshed_at)) = binding else {
        return Ok(false);
    };
    let metadata: Value = serde_json::from_str(&metadata).unwrap_or_default();
    if metadata["refreshed_at"]
        .as_i64()
        .is_none_or(|at| at != refreshed_at || at < now() - 7 * 86400 || at > now() + 60)
    {
        return Ok(false);
    };
    let mut identities = BTreeSet::new();
    let mut seasons = BTreeSet::new();
    for episode in episodes {
        let present=db.query_row("SELECT EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1)",[episode],|r|r.get::<_,bool>(0))?;
        let mappings=db.prepare("SELECT e.manager_episode_id,e.season_number,m.state FROM manager_episode_mappings m JOIN manager_episodes e ON e.service_id=m.service_id AND e.service_generation=m.service_generation AND e.manager_episode_id=m.manager_episode_id JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id WHERE m.media_id=?1 AND b.media_id=?2 AND e.service_id=?3 AND e.series_external_id=?4 AND e.refreshed_at=?5")?.query_map(params![episode,show,service,series,refreshed_at],|r|Ok((r.get::<_,i64>(0)?.to_string(),r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if !present
            || mappings.len() != 1
            || mappings[0].2 != "confirmed"
            || !identities.insert(mappings[0].0.clone())
        {
            return Ok(false);
        };
        seasons.insert(mappings[0].1);
    }
    if seasons.len() != 1 {
        return Ok(false);
    };
    let season = *seasons.first().unwrap();
    if exclude_specials && season == 0 {
        return Ok(false);
    };
    let expected=db.prepare("SELECT e.manager_episode_id,e.metadata FROM manager_episodes e JOIN metadata_bindings b ON b.service_id=e.service_id AND b.service_generation=e.service_generation AND b.external_id=e.series_external_id WHERE b.media_id=?1 AND e.service_id=?2 AND e.series_external_id=?3 AND e.season_number=?4 AND e.refreshed_at=?5")?.query_map(params![show,service,series,season,refreshed_at],|r|Ok((r.get::<_,i64>(0)?.to_string(),serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default())))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if expected
        .iter()
        .map(|e| e.0.clone())
        .collect::<BTreeSet<_>>()
        != identities
    {
        return Ok(false);
    };
    let described = metadata["seasons"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["season_number"].as_i64() == Some(season));
    if described.and_then(|s| s["episode_count"].as_u64()) != Some(expected.len() as u64) {
        return Ok(false);
    };
    let today = chrono::Utc::now().date_naive();
    let aired = |v: &Value| {
        v.as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .is_some_and(|date| date < today)
    };
    if expected.iter().any(|e| !aired(&e.1["air_date"])) {
        return Ok(false);
    };
    let ended = metadata["status"].as_str().is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "ended" | "canceled" | "cancelled"
        )
    });
    let later = metadata["seasons"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["season_number"].as_i64().is_some_and(|n| n > season) && aired(&s["air_date"]));
    let finale = expected
        .iter()
        .any(|e| matches!(e.1["finale_type"].as_str(), Some("season" | "series")));
    Ok(ended || later || finale)
}
