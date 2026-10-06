use super::{AttentionItem, AttentionTarget, Severity};
use rusqlite::{OptionalExtension, params};
use thelxinoe_core::{Principal, Role, now};
use thelxinoe_database::Database;

fn collect(
    db: &rusqlite::Connection,
    user: &str,
    admin: bool,
) -> anyhow::Result<Vec<AttentionItem>> {
    let mut items = Vec::new();
    if admin {
        let sql = "SELECT 'job:'||id,'error','A background job failed.','jobs',id,CAST(completed_at AS TEXT)||':'||attempts,1 FROM jobs WHERE state='failed' AND completed_at>?1
          AND NOT EXISTS(SELECT 1 FROM acquisition_requests r WHERE r.id=json_extract(jobs.payload,'$.request_id') AND r.state IN ('available','requested','denied','cancelled'))
          UNION ALL SELECT 'service:'||id,'error','An integration is unavailable.','services',kind,generation,0 FROM manager_services WHERE enabled=1 AND error IS NOT NULL
          UNION ALL SELECT 'support:'||id,'error','A support service is unavailable.','services',kind,generation,0 FROM support_services WHERE error IS NOT NULL
          UNION ALL SELECT 'update:'||u.id,CASE WHEN u.state IN ('runtime-failure','recovery-required') THEN 'error' ELSE 'warning' END,'A service update needs attention.','services',s.kind,u.state||':'||u.updated_at,0 FROM service_updates u JOIN stack_provisions s ON s.id=u.service_id WHERE u.state IN ('blocked','failed','incompatible','unable-to-verify','runtime-failure','recovery-required')
          AND NOT EXISTS(SELECT 1 FROM service_updates n WHERE n.service_id=u.service_id AND (n.created_at>u.created_at OR (n.created_at=u.created_at AND n.rowid>u.rowid)))
          UNION ALL SELECT 'health:'||json_extract(j.value,'$.id'),'warning','An indexer or download service needs attention.','services',json_extract(j.value,'$.kind'),'health',0 FROM settings s,json_each(s.value,'$.items') j WHERE s.key='operations.support' AND json_extract(j.value,'$.problem')=1
          UNION ALL SELECT 'product-release','info','Server '||json_extract(value,'$.version')||' is available.','server',NULL,json_extract(value,'$.version'),0 FROM settings WHERE key='product.release' AND json_extract(value,'$.version') IS NOT NULL
          UNION ALL SELECT 'tools-release:'||t.id,'info',t.id||' '||json_extract(t.candidate,'$.version')||' is available.','server',t.id,json_extract(t.candidate,'$.id'),0 FROM server_tools t LEFT JOIN tool_generations g ON g.id=t.installed WHERE t.candidate IS NOT NULL AND t.check_error IS NULL AND (g.id IS NULL OR json_extract(t.candidate,'$.id')!=json_extract(g.manifest,'$.candidate.id')) AND CASE WHEN t.policy='inherit' THEN COALESCE((SELECT json_extract(value,'$.policy') FROM settings WHERE key='product.policy'),'automatic') ELSE t.policy END='notify'
          UNION ALL SELECT 'product-update:'||json_extract(j.value,'$.id'),CASE WHEN json_extract(j.value,'$.stage')='blocked' THEN 'warning' ELSE 'error' END,'A server update needs attention.','server',NULL,json_extract(j.value,'$.stage'),0 FROM settings s,json_each(s.value,'$.items') j WHERE s.key='product.controller' AND j.key=0 AND json_extract(j.value,'$.stage') IN ('blocked','recovery-required','runtime-failure')";
        for row in db.prepare(sql)?.query_map([now() - 7 * 86400], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })? {
            let (id, severity, message, target, resource, revision, dismissible) = row?;
            items.push(AttentionItem {
                id,
                severity: match severity.as_str() {
                    "error" => Severity::Error,
                    "warning" => Severity::Warning,
                    _ => Severity::Info,
                },
                message,
                target: match target.as_str() {
                    "services" => AttentionTarget::Services,
                    "jobs" => AttentionTarget::Jobs,
                    _ => AttentionTarget::Server,
                },
                resource,
                revision,
                dismissible,
                media_ids: vec![],
            });
        }
    }
    for provider in db.prepare("SELECT provider,generation FROM online_accounts WHERE user_id=?1 AND status='reconnect_required' AND NOT EXISTS (SELECT 1 FROM settings WHERE key='online.'||online_accounts.provider||'.enabled' AND value='false')")?.query_map([user], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {
        let (provider, revision) = provider?;
        items.push(AttentionItem { id:format!("account:{provider}"), severity:Severity::Warning,message:"A linked account needs you to sign in again.".into(),target:AttentionTarget::Online,resource:Some(provider),revision,dismissible:false,media_ids:vec![] });
    }
    let requests = db.prepare("SELECT 'request:'||r.id,s.kind,r.external_id,r.state,r.generation,r.id,r.updated_at FROM acquisition_requests r JOIN manager_services s ON s.id=r.service_id WHERE r.user_id=?1 AND r.state IN ('available','denied','failed','requested')
        UNION ALL SELECT 'seerr:'||r.service_id||':'||r.request_id,CASE WHEN r.media_type='movie' THEN 'radarr' ELSE 'seerr-tv' END,r.external_id,r.state,r.revision,CAST(r.request_id AS TEXT),NULL FROM seerr_request_states r WHERE r.user_id=?1 AND r.state IN ('available','denied','failed','requested')")?
        .query_map([user], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<i64>>(6)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, kind, external, state, revision, resource, updated) in requests {
        let revision = updated.map_or_else(
            || revision.clone(),
            |updated| super::native_request_revision(&revision, &state, updated),
        );
        let available = state == "available";
        let (target, media_kind, metadata) = match kind.as_str() {
            "radarr" => (AttentionTarget::Movies, "movie", "$.tmdb_id"),
            "sonarr" => (AttentionTarget::Shows, "show", "$.tvdb_id"),
            "seerr-tv" => (AttentionTarget::Shows, "show", "$.tmdb_id"),
            _ => (
                AttentionTarget::Music,
                "album",
                "$.musicbrainz_release_group_id",
            ),
        };
        let media_ids = if available {
            db.prepare("WITH RECURSIVE related(id,parent_id) AS (SELECT id,parent_id FROM media WHERE kind=?1 AND CAST(json_extract(metadata,?2) AS TEXT)=?3 UNION SELECT m.id,m.parent_id FROM media m JOIN related r ON m.id=r.parent_id) SELECT id FROM related ORDER BY id")?
                .query_map(params![media_kind,metadata,external],|r|r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?
        } else {
            vec![]
        };
        items.push(AttentionItem {
            id,
            severity: match state.as_str() {
                "failed" => Severity::Error,
                "denied" => Severity::Warning,
                _ => Severity::Info,
            },
            message: if available {
                "Your requested media is available."
            } else {
                "Your media request has changed."
            }
            .into(),
            target: if available {
                target
            } else {
                AttentionTarget::Requests
            },
            resource: Some(resource),
            revision,
            dismissible: true,
            media_ids,
        });
    }
    let seen = db
        .prepare("SELECT source,revision FROM attention_seen WHERE user_id=?1")?
        .query_map([user], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
    items.retain(|item| !item.dismissible || seen.get(&item.id) != Some(&item.revision));
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(items)
}

pub(super) async fn summary(
    db: &Database,
    principal: Principal,
) -> anyhow::Result<Vec<AttentionItem>> {
    db.read("attention.summary", move |db| {
        collect(db, &principal.user.id, principal.user.role == Role::Admin)
    })
    .await
}

pub(super) async fn observe(db: &Database) -> anyhow::Result<Vec<String>> {
    db.write("attention.observe", |db| {
        let tx = db.transaction()?;
        let users = tx.prepare("SELECT id,role FROM users")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed = Vec::new();
        for (user,role) in users {
            let value = serde_json::to_string(&collect(&tx,&user,role=="admin")?)?;
            let previous = tx.query_row("SELECT value FROM attention_snapshots WHERE user_id=?1",[&user],|r|r.get::<_,String>(0)).optional()?;
            if previous.as_ref()!=Some(&value) {
                tx.execute("INSERT INTO attention_snapshots VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![user,value])?;
                changed.push(user);
            }
        }
        tx.commit()?;
        Ok(changed)
    }).await
}

pub(super) async fn acknowledge(
    db: &Database,
    principal: Principal,
    id: String,
    revision: String,
) -> anyhow::Result<bool> {
    db.write("attention.acknowledge", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = collect(&tx,&principal.user.id,principal.user.role==Role::Admin)?;
        if !current.iter().any(|item|item.id==id && item.revision==revision && item.dismissible) {
            return Ok(false);
        }
        tx.execute("INSERT INTO attention_seen(user_id,source,revision,seen_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,source) DO UPDATE SET revision=excluded.revision,seen_at=excluded.seen_at",params![principal.user.id,id,revision,now()])?;
        tx.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn cache_seerr(
    db: &Database,
    service: String,
    rows: Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    db.write("attention.cache_seerr",move |db| {
        let tx = db.transaction()?;
        // Only a complete successful snapshot replaces saved request state.
        tx.execute("DELETE FROM seerr_request_states WHERE service_id=?1",[&service])?;
        for row in rows {
            if !row["requestedBy"]["id"].as_i64().is_some_and(|id| id > 1) {continue}
            let Some(user) = row["requestedBy"]["email"].as_str().and_then(|email| email.strip_suffix("@thelxinoe.invalid")) else {continue};
            if !tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1)",[user],|r|r.get::<_,bool>(0))? {continue}
            let Some(id) = row["id"].as_i64().filter(|id|*id>0) else {continue};
            let Some(external) = row["media"]["tmdbId"].as_i64().filter(|id|*id>0) else {continue};
            let media_type = row["type"].as_str().or_else(||row["media"]["mediaType"].as_str()).unwrap_or("");
            if !matches!(media_type,"movie"|"tv") {continue}
            let state = super::seerr_request_state(&row);
            let Some(reference) = super::seerr_request_attention(&service, &row) else {continue};
            let revision = reference.revision;
            tx.execute("INSERT INTO seerr_request_states(service_id,user_id,request_id,media_type,external_id,state,revision) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![service,user,id,media_type,external.to_string(),state,revision])?;
        }
        tx.commit()?;
        Ok(())
    }).await
}
