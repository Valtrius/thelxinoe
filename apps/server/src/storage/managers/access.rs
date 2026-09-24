//! Native routes use registered prefixes without changing service configuration.
use super::*;

fn available(db: &rusqlite::Connection) -> anyhow::Result<Vec<Destination>> {
    let services = db.prepare("SELECT s.id,s.kind,s.container_id,s.port,s.url_base,s.access_revision,s.media_source
        FROM (SELECT id,kind,container_id,port,url_base,access_revision,media_source FROM manager_services WHERE enabled=1
              UNION ALL SELECT id,kind,container_id,port,url_base,access_revision,media_source FROM support_services WHERE kind IN ('prowlarr','bazarr','nzbget')) s
        LEFT JOIN stack_provisions p ON p.kind=s.kind
        WHERE p.id IS NULL OR (p.state='complete' AND p.service_id=s.id AND p.container_id=s.container_id)")?
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
    Ok(available(db)?
        .into_iter()
        .map(|s| (s.id, format!("/services/{}", s.kind)))
        .collect())
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
