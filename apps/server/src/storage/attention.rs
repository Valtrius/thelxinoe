use super::{
    ActivityItem, ActivityState, AttentionItem, AttentionSummary, AttentionTarget, Severity,
};
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
          UNION ALL SELECT 'transfer:'||id,'error',CASE state WHEN 'suspended' THEN 'An ownership transfer is suspended for manual resolution.' ELSE 'An ownership transfer needs recovery.' END,'services',kind,state||':'||updated_at,0 FROM service_transfers WHERE state IN ('failed','suspended')
          UNION ALL SELECT 'health:'||json_extract(j.value,'$.id'),'warning','An indexer or download service needs attention.','services',json_extract(j.value,'$.kind'),'health',0 FROM settings s,json_each(s.value,'$.items') j WHERE s.key='operations.support' AND json_extract(j.value,'$.problem')=1
          UNION ALL SELECT 'product-release','info','Server '||json_extract(value,'$.version')||' is available.','server',NULL,json_extract(value,'$.version'),0 FROM settings WHERE key='product.release' AND json_extract(value,'$.version') IS NOT NULL
          UNION ALL SELECT 'tools-release:'||t.id,'info',t.id||' '||json_extract(t.candidate,'$.version')||' is available.','server',t.id,json_extract(t.candidate,'$.id'),0 FROM server_tools t LEFT JOIN tool_generations g ON g.id=t.installed WHERE t.candidate IS NOT NULL AND t.check_error IS NULL AND (g.id IS NULL OR json_extract(t.candidate,'$.id')!=json_extract(g.manifest,'$.candidate.id')) AND CASE WHEN t.policy='inherit' THEN COALESCE((SELECT json_extract(value,'$.policy') FROM settings WHERE key='product.policy'),'automatic') ELSE t.policy END='notify'
          UNION ALL SELECT 'product-update:'||json_extract(j.value,'$.id'),CASE WHEN json_extract(j.value,'$.stage')='blocked' THEN 'warning' ELSE 'error' END,'A server update needs attention.','server',NULL,json_extract(j.value,'$.stage'),0 FROM settings s,json_each(s.value,'$.items') j WHERE s.key='product.controller' AND j.key=0 AND json_extract(j.value,'$.stage') IN ('blocked','recovery-required','runtime-failure')
            AND NOT (json_extract(j.value,'$.stage')='blocked' AND ((json_extract(s.value,'$.generation') IS NOT NULL AND json_extract(j.value,'$.source_generation') IS NOT json_extract(s.value,'$.generation')) OR json_extract(j.value,'$.failure') IS 'interrupted' OR json_extract(j.value,'$.id') IS (SELECT json_extract(value,'$') FROM settings WHERE key='product.dismissed')))
          UNION ALL SELECT 'product-automatic','warning','Automatic server updates keep failing.','server',NULL,json_extract(a.value,'$.version')||':'||json_extract(a.value,'$.failures'),1 FROM settings a JOIN settings r ON r.key='product.release' AND json_extract(r.value,'$.version')=json_extract(a.value,'$.version') WHERE a.key='product.automatic' AND json_extract(a.value,'$.failures')>=3 AND COALESCE((SELECT json_extract(value,'$.policy') FROM settings WHERE key='product.policy'),'automatic')='automatic'
          UNION ALL SELECT 'backup-passphrase','warning','Automatic backups need a saved passphrase.','backups',NULL,'missing',0 WHERE (SELECT json_extract(value,'$.policy') FROM settings WHERE key='backups.policy')='automatic' AND NOT EXISTS(SELECT 1 FROM secrets WHERE scope='backups.passphrase')
          UNION ALL SELECT 'backup-automatic',CASE WHEN json_extract(value,'$.state')='failed' THEN 'error' ELSE 'warning' END,CASE WHEN json_extract(value,'$.state')='failed' THEN 'The automatic backup failed.' ELSE 'The automatic backup was skipped.' END,'backups',NULL,json_extract(value,'$.window')||':'||json_extract(value,'$.state'),1 FROM settings WHERE key='backups.automatic' AND json_extract(value,'$.state') IN ('failed','missed')";
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
                    "backups" => AttentionTarget::Backups,
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

fn activity_target(value: &str) -> AttentionTarget {
    match value {
        "services" => AttentionTarget::Services,
        "server" => AttentionTarget::Server,
        "backups" => AttentionTarget::Backups,
        "online" => AttentionTarget::Online,
        "movies" => AttentionTarget::Movies,
        "shows" => AttentionTarget::Shows,
        "music" => AttentionTarget::Music,
        "analysis" => AttentionTarget::Analysis,
        _ => AttentionTarget::Jobs,
    }
}

fn collect_activity(
    db: &rusqlite::Connection,
    user: &str,
    admin: bool,
) -> anyhow::Result<Vec<ActivityItem>> {
    let mut items = Vec::new();
    if admin {
        let sql = "SELECT 'setup:'||id,'services',kind,CASE WHEN state='queued' THEN 'queued' ELSE 'running' END,
          CASE state WHEN 'queued' THEN 'Service installation queued' WHEN 'installing' THEN 'Installing service' WHEN 'connecting' THEN 'Connecting service API' ELSE 'Retiring service installation' END,NULL
          FROM stack_provisions WHERE state IN ('queued','installing','connecting','retiring') AND error IS NULL
          UNION ALL SELECT 'transfer:'||id,'services',kind,CASE WHEN state='queued' THEN 'queued' ELSE 'running' END,
          CASE WHEN state='recovery' THEN 'Recovering service ownership transfer' ELSE 'Transferring service ownership' END,NULL
          FROM service_transfers WHERE state IN ('queued','running','committed','recovery')
          UNION ALL SELECT 'update:'||u.id,'services',s.kind,CASE WHEN u.state IN ('queued','queued-activate','queued-recover') THEN 'queued' ELSE 'running' END,
          CASE WHEN u.state IN ('queued-recover','recovery-snapshot','rollback-copying','rollback-activating') THEN 'Recovering service update' WHEN u.state IN ('activating','queued-activate') THEN 'Activating service update' ELSE 'Preparing service update' END,NULL
          FROM service_updates u JOIN stack_provisions s ON s.id=u.service_id WHERE u.state IN ('queued','submitting','preparing','snapshotting','preflight','queued-activate','recovery-snapshot','isolated-live-validation','rollback-copying','rollback-activating','activating','queued-recover') AND u.error IS NULL
          AND NOT EXISTS(SELECT 1 FROM service_updates n WHERE n.service_id=u.service_id AND (n.created_at>u.created_at OR (n.created_at=u.created_at AND n.rowid>u.rowid)))
          UNION ALL SELECT 'tool:'||id,'server',tool,CASE WHEN stage IN ('queued','waiting') THEN 'queued' ELSE 'running' END,
          CASE stage WHEN 'waiting' THEN 'Tool replacement waiting for idle' WHEN 'queued' THEN 'Tool operation queued' WHEN 'downloading' THEN 'Downloading tool' WHEN 'verifying' THEN 'Verifying tool' WHEN 'validating' THEN 'Validating tool' WHEN 'activating' THEN 'Activating tool' WHEN 'checking' THEN 'Checking tool updates' ELSE 'Installing tool' END,NULL
          FROM tool_jobs WHERE stage IN ('queued','waiting','checking','downloading','verifying','validating','installing','activating')
          UNION ALL SELECT 'product:'||json_extract(j.value,'$.id'),'server',NULL,CASE WHEN json_extract(s.value,'$.available')=0 THEN 'unknown' WHEN json_extract(j.value,'$.stage')='queued' THEN 'queued' ELSE 'running' END,'Updating Thelxinoe server',NULL
          FROM settings s,json_each(s.value,'$.items') j WHERE s.key='product.controller' AND j.key=0 AND json_extract(j.value,'$.stage') IN ('queued','preparing','snapshotting','validating','preparing-activation','isolated-migration','creating-successor','handoff','activating','recovering','restoring-release')
          UNION ALL SELECT 'backup:'||json_extract(j.value,'$.id'),'backups',json_extract(j.value,'$.id'),CASE WHEN json_extract(s.value,'$.available')=0 THEN 'unknown' WHEN json_extract(j.value,'$.stage') IN ('queued','restore-queued') THEN 'queued' ELSE 'running' END,
          CASE WHEN json_extract(j.value,'$.stage') IN ('restore-queued','restoring','restore-activating','release-handoff') THEN 'Restoring backup' ELSE 'Creating encrypted backup' END,NULL
          FROM settings s,json_each(s.value,'$.items') j WHERE s.key='backups.controller' AND COALESCE(json_extract(j.value,'$.needs_attention'),0)=0 AND json_extract(j.value,'$.stage') IN ('queued','quiescing','snapshotting','encrypting','restore-queued','restoring','restore-activating','release-handoff','rollback-copying','rollback-activating')
          UNION ALL SELECT 'backup-wait:'||json_extract(value,'$.id'),'backups',json_extract(value,'$.id'),'queued','Automatic backup waiting for idle',NULL FROM settings WHERE key='backups.automatic' AND json_extract(value,'$.state')='waiting'
          UNION ALL SELECT 'job:'||id,'jobs',id,state,CASE kind WHEN 'library.scan' THEN 'Scanning library' WHEN 'manager.request' THEN 'Acquiring requested media' WHEN 'recyclarr.sync' THEN 'Synchronizing Recyclarr configuration' WHEN 'stack.install' THEN 'Installing service' WHEN 'service.update' THEN 'Updating service' WHEN 'checkpoint' THEN 'Creating server checkpoint' ELSE 'Refreshing media metadata' END,json_extract(payload,'$.media_id')
          FROM jobs WHERE state IN ('queued','running') AND (kind IN ('library.scan','manager.request','recyclarr.sync','stack.install','service.update','checkpoint') OR (kind IN ('metadata.match','metadata.refresh') AND json_extract(payload,'$.actor_id') IS NOT NULL))
          UNION ALL SELECT 'scan:'||j.id,r.kind,r.id,j.state,'Scanning '||r.name,NULL FROM jobs j JOIN library_roots r ON r.id=json_extract(j.payload,'$.root_id') WHERE j.kind='library.scan' AND j.state IN ('queued','running')
          UNION ALL SELECT 'recyclarr:'||id,'services','recyclarr',state,'Synchronizing Recyclarr configuration',NULL FROM jobs WHERE kind='recyclarr.sync' AND state IN ('queued','running')
          UNION ALL SELECT 'media:'||o.id,r.kind,o.media_id,'running','Applying media changes',o.media_id FROM media_operations o JOIN media m ON m.id=o.media_id JOIN library_roots r ON r.id=m.root_id WHERE o.state='executing'
          UNION ALL SELECT 'metadata:'||j.id,r.kind,m.id,j.state,'Refreshing media metadata',m.id FROM jobs j JOIN media m ON m.id=json_extract(j.payload,'$.media_id') JOIN library_roots r ON r.id=m.root_id WHERE j.kind IN ('metadata.match','metadata.refresh') AND j.state IN ('queued','running') AND json_extract(j.payload,'$.actor_id') IS NOT NULL
          UNION ALL SELECT 'analysis:'||media_id||':'||file_id,'analysis',media_id,state,'Analyzing episode',media_id FROM segment_analysis WHERE state='running'
          UNION ALL SELECT 'connection:'||json_extract(value,'$.id'),'services',json_extract(value,'$.source_kind'),CASE WHEN json_extract(value,'$.state') IN ('connecting','cleaning') THEN 'running' ELSE 'queued' END,
          CASE WHEN json_extract(value,'$.cleanup')=1 THEN 'Disconnecting service connection' ELSE 'Connecting services' END,NULL
          FROM settings WHERE key LIKE 'services.connection.%' AND json_extract(value,'$.state') IN ('pending','disconnecting','connecting','cleaning') AND (json_extract(value,'$.enabled')=1 OR json_extract(value,'$.cleanup')=1)";
        for row in db.prepare(sql)?.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })? {
            let (id, target, resource, state, message, media) = row?;
            items.push(ActivityItem {
                id,
                target: activity_target(&target),
                resource,
                state: match state.as_str() {
                    "queued" => ActivityState::Queued,
                    "unknown" => ActivityState::Unknown,
                    _ => ActivityState::Running,
                },
                message,
                media_ids: media.into_iter().collect(),
            });
        }
    }
    let sql = "SELECT 'download:'||d.video_id,CASE WHEN d.state IN ('queued','waiting_for_space') THEN 'queued' ELSE 'running' END,CASE d.state WHEN 'waiting_for_space' THEN 'Video download waiting for space' WHEN 'queued' THEN 'Video download queued' WHEN 'deleting' THEN 'Removing downloaded video' ELSE 'Downloading video' END
      FROM youtube_downloads d JOIN youtube_download_requests r ON r.video_id=d.video_id WHERE r.user_id=?1 AND d.state IN ('queued','waiting_for_space','downloading','deleting')
      AND NOT EXISTS(SELECT 1 FROM settings WHERE key='online.youtube.enabled' AND value='false')
      UNION ALL SELECT 'account:twitch',CASE WHEN phase='connecting' THEN 'running' ELSE 'queued' END,CASE WHEN phase='connecting' THEN 'Connecting Twitch account' ELSE 'Waiting for Twitch authorization' END FROM twitch_attempts WHERE user_id=?1 AND expires_at>?2
      UNION ALL SELECT 'sync:youtube',CASE WHEN y.failures>0 OR y.last_turn<=json_extract(s.value,'$.last_turn') THEN 'queued' ELSE 'running' END,CASE WHEN y.failures>0 THEN 'YouTube synchronization waiting to retry' ELSE 'Synchronizing YouTube' END
      FROM youtube_sync y JOIN online_accounts a ON a.user_id=y.user_id AND a.provider='youtube' AND a.status='connected' AND a.generation=y.generation JOIN settings s ON s.key='activity.youtube-sync:'||y.user_id
      WHERE y.user_id=?1 AND json_extract(s.value,'$.generation')=y.generation AND COALESCE(y.last_complete,0)=json_extract(s.value,'$.last_complete')
      AND NOT EXISTS(SELECT 1 FROM settings WHERE key='online.youtube.enabled' AND value='false')";
    for row in db.prepare(sql)?.query_map(params![user, now()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, state, message) = row?;
        let resource = if id == "account:twitch" {
            "twitch"
        } else {
            "youtube"
        };
        items.push(ActivityItem {
            id,
            target: AttentionTarget::Online,
            resource: Some(resource.into()),
            state: if state == "queued" {
                ActivityState::Queued
            } else {
                ActivityState::Running
            },
            message,
            media_ids: vec![],
        });
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(items)
}

fn collect_summary(
    db: &rusqlite::Connection,
    user: &str,
    admin: bool,
) -> anyhow::Result<AttentionSummary> {
    Ok(AttentionSummary {
        items: collect(db, user, admin)?,
        activity: collect_activity(db, user, admin)?,
    })
}

pub(super) async fn summary(
    db: &Database,
    principal: Principal,
) -> anyhow::Result<AttentionSummary> {
    db.read("attention.summary", move |db| {
        collect_summary(db, &principal.user.id, principal.user.role == Role::Admin)
    })
    .await
}

pub(super) async fn observe(db: &Database) -> anyhow::Result<Vec<String>> {
    db.write("attention.observe", |db| {
        let tx = db.transaction()?;
        let users = tx.prepare("SELECT id,role FROM users")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed = Vec::new();
        for (user,role) in users {
            let value = serde_json::to_string(&collect_summary(&tx,&user,role=="admin")?)?;
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
