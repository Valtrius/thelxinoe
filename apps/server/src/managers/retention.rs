//! Watched-state retention uses the same captured file generations as manual deletion.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
fn exclusion_endpoint(kind: &str) -> &'static str {
    if kind == "radarr" {
        "exclusions"
    } else {
        "importlistexclusion"
    }
}
#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/retention", get(list))
        .route("/api/v1/admin/retention/policy/{domain}", post(policy))
        .route("/api/v1/admin/retention/root/{id}", post(root_policy))
        .route("/api/v1/admin/retention/evaluate", post(evaluate))
        .route("/api/v1/admin/retention/{id}/{action}", post(action))
}
#[derive(Deserialize, Serialize, Clone)]
struct Policy {
    enabled: bool,
    grace_seconds: i64,
    exclude_specials: bool,
    trigger_users: Vec<String>,
}
#[derive(Clone)]
struct Eligible {
    stamp: String,
    user: String,
    grace: i64,
    automatic: bool,
}

fn eligibility(db: &rusqlite::Connection, media: &str) -> anyhow::Result<Option<Eligible>> {
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
fn complete_season(
    db: &rusqlite::Connection,
    show: &str,
    episodes: &[String],
    exclude_specials: bool,
) -> anyhow::Result<bool> {
    if episodes.is_empty() {
        return Ok(false);
    };
    let (metadata,series)=db.query_row("SELECT m.metadata,p.external_id FROM media m JOIN provider_ids p ON p.media_id=m.id AND p.provider='tmdb' AND p.mapping_state='confirmed' WHERE m.id=?1",[show],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?.unwrap_or_default();
    let metadata: Value = serde_json::from_str(&metadata).unwrap_or_default();
    if series.is_empty()
        || metadata["refreshed_at"]
            .as_i64()
            .is_none_or(|at| at < now() - 7 * 86400 || at > now() + 60)
    {
        return Ok(false);
    };
    let mut identities = BTreeSet::new();
    let mut seasons = BTreeSet::new();
    for episode in episodes {
        let present=db.query_row("SELECT EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1)",[episode],|r|r.get::<_,bool>(0))?;
        let mappings=db.prepare("SELECT p.episode_id,p.season_number,m.state FROM episode_mappings m JOIN provider_episodes p ON p.provider=m.provider AND p.episode_id=m.episode_id WHERE m.media_id=?1 AND p.provider='tmdb' AND p.series_id=?2")?.query_map(params![episode,series],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
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
    let expected=db.prepare("SELECT episode_id,metadata FROM provider_episodes WHERE provider='tmdb' AND series_id=?1 AND season_number=?2")?.query_map(params![series,season],|r|Ok((r.get::<_,String>(0)?,serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or_default())))?.collect::<rusqlite::Result<Vec<_>>>()?;
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
    let ended = matches!(metadata["status"].as_str(), Some("Ended" | "Canceled"));
    let later = metadata["seasons"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["season_number"].as_i64().is_some_and(|n| n > season) && aired(&s["air_date"]));
    let finale = expected.iter().any(|e| e.1["episode_type"] == "finale");
    Ok(ended || later || finale)
}
pub(super) async fn revalidate_operation(
    state: &AppState,
    operation: &str,
    automatic: bool,
) -> Result<()> {
    let operation = operation.to_owned();
    let valid=state.db.call(move|db|{
        let candidate=db.query_row("SELECT media_id,stamp,due_at,state FROM retention_candidates WHERE operation_id=?1",[operation],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional()?;
        let Some((media,stamp,due,status))=candidate else {return Ok(true)};
        Ok(["pending","executing"].contains(&status.as_str()) && eligibility(db,&media)?.is_some_and(|e|e.stamp==stamp && (!automatic||(e.automatic&&now()>=due))))
    }).await?;
    if !valid {
        return Err(ApiError::conflict(
            "Retention eligibility, watched state, protection or file generation changed",
        ));
    }
    Ok(())
}

async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let value=state.db.call(|db|{
        let policies=db.prepare("SELECT domain,enabled,grace_seconds,exclude_specials,trigger_users FROM retention_policies")?.query_map([],|r|Ok(json!({"domain":r.get::<_,String>(0)?,"enabled":r.get::<_,bool>(1)?,"grace_seconds":r.get::<_,i64>(2)?,"exclude_specials":r.get::<_,bool>(3)?,"trigger_users":serde_json::from_str::<Value>(&r.get::<_,String>(4)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=db.prepare("SELECT c.id,c.media_id,m.title,c.state,c.eligible_at,c.due_at,c.error,u.username FROM retention_candidates c JOIN media m ON m.id=c.media_id LEFT JOIN users u ON u.id=c.trigger_user ORDER BY c.eligible_at DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"media_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"eligible_at":r.get::<_,i64>(4)?,"due_at":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?,"trigger_user":r.get::<_,Option<String>>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let users=db.prepare("SELECT id,username FROM users ORDER BY username")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let roots=db.prepare("SELECT id,name,automatic_unmanaged_deletion FROM library_roots WHERE kind IN ('movies','shows')")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"automatic_unmanaged_deletion":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"policies":policies,"items":items,"users":users,"roots":roots}))
    }).await?;
    Ok(Json(value))
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(domain): Path<String>,
    Json(mut input): Json<Policy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if !["movies", "shows"].contains(&domain.as_str())
        || !(0..=31536000).contains(&input.grace_seconds)
        || input.trigger_users.len() > 100
    {
        return Err(ApiError::bad("Invalid retention policy"));
    }
    input.trigger_users.sort();
    input.trigger_users.dedup();
    if input.enabled && input.trigger_users.is_empty() {
        return Err(ApiError::bad("Choose at least one retention-trigger user"));
    }
    let _lease = state.media_operations.write().await;
    let valid=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let users=json!(input.trigger_users).to_string();
        let count=tx.query_row("SELECT COUNT(*) FROM users WHERE id IN (SELECT value FROM json_each(?1))",[&users],|r|r.get::<_,i64>(0))?;
        if count!=input.trigger_users.len() as i64{return Ok(false);}
        tx.execute("UPDATE retention_policies SET enabled=?1,grace_seconds=?2,exclude_specials=?3,trigger_users=?4,updated_at=MAX(updated_at+1,?5) WHERE domain=?6",params![input.enabled,input.grace_seconds,input.exclude_specials,users,now(),domain])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.policy',?2,?3)",params![p.user.id,domain,now()])?;tx.commit()?;Ok(true)
    }).await?;
    if !valid {
        return Err(ApiError::bad("Unknown retention-trigger user"));
    }
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct RootPolicy {
    automatic_unmanaged_deletion: bool,
}
async fn root_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<RootPolicy>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let changed=state.db.call(move|db|{let tx=db.transaction()?;let count=tx.execute("UPDATE library_roots SET automatic_unmanaged_deletion=?1 WHERE id=?2 AND kind IN ('movies','shows')",params![input.automatic_unmanaged_deletion,key])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.root',?2,?3)",params![p.user.id,key,now()])?;tx.commit()?;Ok(count>0)}).await?;
    if !changed {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"saved":true})))
}
async fn evaluate(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let count = evaluate_all(&state, Some(p.user.id)).await?;
    Ok(Json(json!({"created":count})))
}
async fn evaluate_all(state: &AppState, actor: Option<String>) -> Result<usize> {
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    let enabled = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM retention_policies WHERE enabled=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if !enabled {
        return Ok(0);
    };
    bindings::reconcile(state).await?;
    let candidates=state.db.call(|db| {
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
    }).await?;
    let mut count = 0;
    for (media, e) in candidates {
        let operation =
            operations::prepare_locked(state, actor.clone(), media.clone(), "delete".into())
                .await?
                .0;
        let key = id();
        let actor = actor.clone();
        state.db.call(move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,trigger_user,state,eligible_at,due_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7)",params![key,media,operation["id"].as_str(),e.stamp,e.user,now(),now()+e.grace])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'retention.pending',?2,?3)",params![actor,key,now()])?;tx.commit()?;Ok(())}).await?;
        count += 1;
    }
    if count > 0 {
        state
            .emit(None, "retention.changed", json!({"created":count}))
            .await?;
    }
    Ok(count)
}
async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((key, action)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    if action == "delete" {
        delete_locked(&state, &key, Some(p.user.id), false).await?;
    } else if ["cancel", "keep"].contains(&action.as_str()) {
        let changed=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let record=tx.query_row("SELECT media_id,operation_id FROM retention_candidates WHERE id=?1 AND state='pending'",[&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;let Some((media,operation))=record else{return Ok(false)};
            if action=="keep" {tx.execute("INSERT INTO media_protection VALUES (?1,1) ON CONFLICT(media_id) DO UPDATE SET keep=1",[&media])?;}
            tx.execute("UPDATE retention_candidates SET state='cancelled' WHERE id=?1",[&key])?;
            tx.execute("UPDATE media_operations SET state='blocked',error='Retention cancelled' WHERE id=?1 AND state='pending'",[operation])?;
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![p.user.id,format!("retention.{action}"),key,now()])?;tx.commit()?;Ok(true)
        }).await?;
        if !changed {
            return Err(ApiError::conflict(
                "Only pending retention can be cancelled or kept here",
            ));
        }
    } else {
        return Err(ApiError::bad("Unsupported retention action"));
    }
    state.emit(None, "retention.changed", json!({})).await?;
    Ok(Json(json!({"saved":true})))
}
async fn delete_locked(
    state: &AppState,
    key: &str,
    actor: Option<String>,
    automatic: bool,
) -> Result<()> {
    let lookup = key.to_owned();
    let operation = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT operation_id FROM retention_candidates WHERE id=?1 AND state='pending'",
                    [lookup],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?
        .ok_or_else(ApiError::not_found)?;
    let result = operations::execute_locked(state, operation, actor, automatic).await;
    if let Err(error) = result {
        let key = key.to_owned();
        let message = error.2.clone();
        state
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE retention_candidates SET state='blocked',error=?1 WHERE id=?2",
                    params![message, key],
                )?;
                Ok(())
            })
            .await?;
        return Err(error);
    }
    Ok(())
}
pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    loop {
        let _ = evaluate_all(&state, None).await;
        let _lease = state.media_operations.write().await;
        let _guard = state.managers.guard.lock().await;
        let ready=state.db.call(|db|{let items=db.prepare("SELECT id,media_id FROM retention_candidates WHERE state='pending' AND due_at<=?1")?.query_map([now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;let mut ready=Vec::new();for (key,media) in items {if eligibility(db,&media)?.is_some_and(|e|e.automatic){ready.push(key);}}Ok(ready)}).await?;
        for key in ready {
            let _ = delete_locked(&state, &key, None, true).await;
        }
        drop(_guard);
        drop(_lease);
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

pub(super) async fn exclude(
    state: &AppState,
    operation: &str,
    files: &[operations::Target],
) -> Result<()> {
    let operation = operation.to_owned();
    let retained=state.db.call(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM retention_candidates WHERE operation_id=?1 AND state IN ('pending','executing'))",[operation],|r|r.get::<_,bool>(0))?)).await?;
    if !retained {
        return Ok(());
    };
    let mut seen = BTreeSet::new();
    for claim in files.iter().flat_map(|f| &f.claims) {
        if !seen.insert((claim.service_id.clone(), claim.external_id.clone())) {
            continue;
        }
        let s = service(state, &claim.service_id).await?;
        let c = Connection::open(state, &s).await?;
        let field = if s.kind == "radarr" {
            "tmdbId"
        } else if s.kind == "sonarr" {
            "tvdbId"
        } else {
            return Err(ApiError::conflict("Music is excluded from retention"));
        };
        let external = claim
            .external_id
            .parse::<i64>()
            .map_err(|_| unavailable())?;
        let endpoint = exclusion_endpoint(&s.kind);
        let existing = c.get(endpoint).await?;
        let found = existing
            .as_array()
            .ok_or_else(unavailable)?
            .iter()
            .find(|v| v[field].as_i64() == Some(external));
        let (exclusion, created) = if let Some(found) = found {
            (found.clone(), false)
        } else {
            let item = c
                .get(&format!(
                    "{}/{}",
                    requests::endpoint(&s.kind),
                    claim.entity_id
                ))
                .await?;
            if requests::external(&s.kind, &item).as_deref() != Some(&claim.external_id) {
                return Err(ApiError::conflict(
                    "Manager identity changed before retention exclusion",
                ));
            }
            let body = if s.kind == "radarr" {
                json!({"tmdbId":external,"movieTitle":item["title"],"movieYear":item["year"]})
            } else {
                json!({"tvdbId":external,"title":item["title"]})
            };
            (
                c.call(reqwest::Method::POST, endpoint, &[], Some(body))
                    .await?,
                true,
            )
        };
        let eid = exclusion["id"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or_else(unavailable)?;
        let key = s.id.clone();
        let external = claim.external_id.clone();
        state.db.call(move|db|{db.execute("INSERT INTO retention_exclusions(service_id,external_id,exclusion_id,created_here) VALUES (?1,?2,?3,?4) ON CONFLICT(service_id,external_id) DO UPDATE SET exclusion_id=excluded.exclusion_id,created_here=CASE WHEN retention_exclusions.exclusion_id=excluded.exclusion_id THEN MAX(retention_exclusions.created_here,excluded.created_here) ELSE excluded.created_here END",params![key,external,eid,created])?;Ok(())}).await?;
    }
    Ok(())
}

/// Called under the media and manager leases before an approved acquisition search.
pub(super) async fn reacquire(
    state: &AppState,
    service: &Service,
    connection: &Connection<'_>,
    external: &str,
    entity: i64,
) -> Result<()> {
    if !["radarr", "sonarr"].contains(&service.kind.as_str()) {
        return Ok(());
    }
    let sid = service.id.clone();
    let identity = external.to_owned();
    let (records, exclusion) = state.db.call(move |db| {
        let records=db.prepare("SELECT c.id,o.targets FROM retention_candidates c JOIN media_operations o ON o.id=c.operation_id WHERE c.state IN ('pending','executing','complete','blocked')")?
            .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let exclusion=db.query_row("SELECT exclusion_id,created_here FROM retention_exclusions WHERE service_id=?1 AND external_id=?2",params![sid,identity],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,bool>(1)?))).optional()?;
        Ok((records,exclusion))
    }).await?;
    let mut affected = Vec::new();
    let mut episodes = BTreeSet::new();
    for (key, targets) in records {
        let targets: Vec<operations::Target> =
            serde_json::from_str(&targets).map_err(|_| unavailable())?;
        for claim in targets
            .iter()
            .flat_map(|t| &t.claims)
            .filter(|c| c.service_id == service.id && c.external_id == external)
        {
            if claim.service_generation != service.generation || claim.entity_id != entity {
                return Err(ApiError::conflict(
                    "Retained manager identity changed; reconcile before reacquisition",
                ));
            }
            episodes.extend(claim.members.iter().copied());
            affected.push(key.clone());
        }
    }
    if service.kind == "sonarr" && !episodes.is_empty() {
        let current = connection
            .call(
                reqwest::Method::GET,
                "episode",
                &[("seriesId", entity.to_string())],
                None,
            )
            .await?;
        let current = current.as_array().ok_or_else(unavailable)?;
        if episodes.iter().any(|id| {
            !current
                .iter()
                .any(|e| e["id"].as_i64() == Some(*id) && e["seriesId"].as_i64() == Some(entity))
        }) {
            return Err(ApiError::conflict(
                "Retained episode identities changed; reconcile before reacquisition",
            ));
        }
        connection
            .call(
                reqwest::Method::PUT,
                "episode/monitor",
                &[],
                Some(json!({"episodeIds":episodes,"monitored":true})),
            )
            .await?;
    }
    if let Some((id, true)) = exclusion {
        let endpoint = exclusion_endpoint(&service.kind);
        let all = connection.get(endpoint).await?;
        let field = if service.kind == "radarr" {
            "tmdbId"
        } else {
            "tvdbId"
        };
        let external_id = external.parse::<i64>().map_err(|_| unavailable())?;
        if all.as_array().ok_or_else(unavailable)?.iter().any(|item| {
            item["id"].as_i64() == Some(id) && item[field].as_i64() == Some(external_id)
        }) {
            connection
                .call(
                    reqwest::Method::DELETE,
                    &format!("{endpoint}/{id}"),
                    &[],
                    None,
                )
                .await?;
        }
    }
    let sid = service.id.clone();
    let identity = external.to_owned();
    state.db.call(move|db| {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for key in affected {
            tx.execute("UPDATE media_operations SET state='blocked',error='Retention reversed by reacquisition' WHERE state='pending' AND id=(SELECT operation_id FROM retention_candidates WHERE id=?1)",[&key])?;
            tx.execute("UPDATE retention_candidates SET state='cancelled',error='Reacquisition requested' WHERE id=?1",[key])?;
        }
        tx.execute("DELETE FROM retention_exclusions WHERE service_id=?1 AND external_id=?2",params![sid,identity])?;
        tx.commit()?;
        Ok(())
    }).await?;
    Ok(())
}
