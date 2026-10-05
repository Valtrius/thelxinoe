//! Native routes use registered prefixes without changing service configuration.
use super::*;

fn available(db: &rusqlite::Connection) -> anyhow::Result<Vec<Destination>> {
    let services = db.prepare("SELECT s.id,s.kind,s.container_id,s.port,s.url_base,s.access_revision,s.media_source
        FROM (SELECT id,kind,container_id,port,url_base,access_revision,media_source FROM manager_services WHERE enabled=1
              UNION ALL SELECT id,kind,container_id,port,url_base,access_revision,media_source FROM support_services WHERE kind IN ('prowlarr','bazarr','nzbget')) s
        LEFT JOIN stack_provisions p ON p.kind=s.kind
        WHERE p.id IS NULL OR ((p.state='complete' OR p.origin='adopted') AND p.service_id=s.id AND p.container_id=s.container_id)")?
        .query_map([], |r| Ok(Destination {
            id:r.get(0)?, kind:r.get(1)?, container:r.get(2)?, port:r.get(3)?,
            url_base:r.get(4)?, access_revision:r.get(5)?, media_source:r.get(6)?
        }))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter().filter(|s| can_mount(&s.kind, &s.url_base)).collect::<Vec<_>>();
    // Ambiguous prefixes stay API-only. Never choose one service over another.
    Ok(services
        .iter()
        .filter(|s| {
            !services.iter().any(|other| {
                s.id != other.id
                    && (within(
                        mount(&s.kind, &s.url_base),
                        mount(&other.kind, &other.url_base),
                    ) || within(
                        mount(&other.kind, &other.url_base),
                        mount(&s.kind, &s.url_base),
                    ))
            })
        })
        .cloned()
        .collect())
}

pub(super) fn launch_urls(
    db: &rusqlite::Connection,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut urls = available(db)?
        .into_iter()
        .map(|s| (s.id, format!("/services/{}", s.kind)))
        .collect::<std::collections::HashMap<_, _>>();
    for (key, url) in native_urls(db)? {
        if !url.is_empty() {
            urls.entry(key).or_insert(url);
        }
    }
    Ok(urls)
}
pub(super) fn native_urls(
    db: &rusqlite::Connection,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    Ok(db.prepare("SELECT s.id,COALESCE(v.value,'') FROM (SELECT id FROM manager_services WHERE enabled=1 UNION ALL SELECT id FROM support_services WHERE kind IN ('prowlarr','bazarr','nzbget')) s LEFT JOIN settings v ON v.key='service.native-url.'||s.id")?.query_map([], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
}
pub(super) async fn external_fallback(
    state: &AppState,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let key = key.to_owned();
    state
        .db
        .read("managers.access.external_fallback", move |db| {
            Ok(launch_urls(db)?
                .remove(&key)
                .filter(|v| v.starts_with("http://") || v.starts_with("https://")))
        })
        .await
}
pub(super) async fn save_native_url(
    db: &thelxinoe_database::Database,
    key: String,
    url: String,
    actor: String,
) -> anyhow::Result<bool> {
    db.write("managers.access.save_native_url", move |db| {
        let tx=db.transaction()?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM manager_services WHERE id=?1 AND enabled=1) OR EXISTS(SELECT 1 FROM support_services WHERE id=?1 AND kind IN ('prowlarr','bazarr','nzbget'))",[&key],|r|r.get::<_,bool>(0))? { return Ok(false); }
        tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("service.native-url.{key}"),url])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'service.native-url',?2,?3)",params![actor,key,now()])?;
        tx.commit()?; Ok(true)
    }).await
}

pub(super) async fn routes(state: &AppState) -> anyhow::Result<Vec<(String, String, String)>> {
    state
        .db
        .read("managers.access.routes", |db| {
            Ok(available(db)?
                .into_iter()
                .map(|s| {
                    let base = mount(&s.kind, &s.url_base).to_owned();
                    (s.id, s.kind, base)
                })
                .collect())
        })
        .await
}

pub(super) async fn load(state: &AppState, key: String) -> anyhow::Result<Option<Destination>> {
    state
        .db
        .read("managers.access.load", move |db| {
            Ok(available(db)?.into_iter().find(|s| s.id == key))
        })
        .await
}
