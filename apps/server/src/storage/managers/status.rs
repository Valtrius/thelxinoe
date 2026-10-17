//! Database operations for managers.status: how catalog items name their manager titles.
use super::*;
use thelxinoe_database::Database;

const PRESENT: &str = "EXISTS(SELECT 1 FROM media_sources ms JOIN media_files f ON f.id=ms.file_id WHERE ms.media_id=e.id AND f.present=1)";
const MAPPED: &str = "(SELECT json_group_array(json_array(x.service_id,x.manager_episode_id)) FROM manager_episode_mappings x WHERE x.media_id=e.id AND x.state='confirmed')";

fn mapped(value: Option<String>) -> Vec<(String, i64)> {
    value
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Operational Radarr and Sonarr services; Lidarr has no title status.
pub(super) async fn services(db: &Database) -> anyhow::Result<Vec<(String, String)>> {
    db.read("managers.status.services", |db| {
        Ok(db
            .prepare("SELECT id,kind FROM manager_services WHERE operational=1 AND kind IN ('radarr','sonarr') ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// Every saved Radarr and Sonarr service, operational or not.
pub(super) async fn configured(db: &Database) -> anyhow::Result<HashSet<String>> {
    db.read("managers.status.configured", |db| {
        Ok(db
            .prepare("SELECT id FROM manager_services WHERE kind IN ('radarr','sonarr')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?)
    })
    .await
}

/// Links for movies, shows, seasons and episodes, and the playable episodes of each show
/// or season among them.
pub(super) async fn catalog(db: &Database, ids: Vec<String>) -> anyhow::Result<Catalog> {
    let ids = serde_json::to_string(&ids)?;
    db.read("managers.status.catalog", move |db| {
        let links = db.prepare(&format!("SELECT e.id,e.kind,{PRESENT},b.service_id,b.manager_entity_id,b.external_id,
                CAST(json_extract(r.metadata,'$.tmdb_id') AS INTEGER),CAST(json_extract(r.metadata,'$.tvdb_id') AS INTEGER),
                CASE e.kind WHEN 'season' THEN e.sort_number WHEN 'episode' THEN p.sort_number END,
                CASE e.kind WHEN 'episode' THEN e.sort_number END,{MAPPED}
            FROM media e LEFT JOIN media p ON p.id=e.parent_id LEFT JOIN media g ON g.id=p.parent_id
            LEFT JOIN media r ON r.id=CASE e.kind WHEN 'season' THEN p.id WHEN 'episode' THEN g.id ELSE e.id END
            LEFT JOIN metadata_bindings b ON b.media_id=r.id
            WHERE e.id IN (SELECT value FROM json_each(?1)) AND e.kind IN ('movie','show','season','episode')"))?
            .query_map([&ids], |r| Ok(Link {
                id: r.get(0)?, kind: r.get(1)?, available: r.get(2)?, service_id: r.get(3)?, entity_id: r.get(4)?,
                external_id: r.get(5)?, tmdb_id: r.get::<_, Option<i64>>(6)?.filter(|v| *v > 0),
                tvdb_id: r.get::<_, Option<i64>>(7)?.filter(|v| *v > 0), season: r.get(8)?, episode: r.get(9)?,
                mapped: mapped(r.get(10)?),
            }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut present = HashMap::<String, Vec<Present>>::new();
        let mut query = db.prepare(&format!("SELECT t.id,s.sort_number,e.sort_number,{MAPPED} FROM media t
                JOIN media s ON s.parent_id=t.id AND s.kind='season' JOIN media e ON e.parent_id=s.id AND e.kind='episode'
                WHERE t.id IN (SELECT value FROM json_each(?1)) AND t.kind='show' AND {PRESENT}
            UNION ALL SELECT t.id,t.sort_number,e.sort_number,{MAPPED} FROM media t JOIN media e ON e.parent_id=t.id AND e.kind='episode'
                WHERE t.id IN (SELECT value FROM json_each(?1)) AND t.kind='season' AND {PRESENT}"))?;
        let rows = query.query_map([&ids], |r| Ok((r.get::<_, String>(0)?, Present {
            season: r.get(1)?, episode: r.get(2)?, mapped: mapped(r.get(3)?),
        })))?;
        for row in rows {
            let (id, episode) = row?;
            present.entry(id).or_default().push(episode);
        }
        Ok(Catalog { links, present })
    })
    .await
}

/// Manager titles already represented by a catalog movie or show.
pub(super) async fn linked(db: &Database, kind: String) -> anyhow::Result<Linked> {
    db.read("managers.status.linked", move |db| {
        let mut linked = Linked::default();
        let mut query = db.prepare("SELECT b.service_id,b.manager_entity_id,CAST(json_extract(m.metadata,'$.tmdb_id') AS INTEGER),CAST(json_extract(m.metadata,'$.tvdb_id') AS INTEGER),b.external_id FROM media m LEFT JOIN metadata_bindings b ON b.media_id=m.id WHERE m.kind=?1")?;
        let rows = query.query_map([&kind], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<String>>(4)?))
        })?;
        for row in rows {
            let (service, entity, mut tmdb, mut tvdb, external) = row?;
            // Radarr bindings hold a TMDB id and Sonarr bindings a TVDB id, even before
            // metadata has been refreshed into the item.
            let external = external.and_then(|id| id.parse::<i64>().ok());
            if kind == "movie" {
                tmdb = tmdb.or(external);
            } else {
                tvdb = tvdb.or(external);
            }
            if let (Some(service), Some(entity)) = (service, entity) {
                linked.entities.insert((service, entity));
            }
            linked.tmdb.extend(tmdb.filter(|v| *v > 0));
            linked.tvdb.extend(tvdb.filter(|v| *v > 0));
        }
        Ok(linked)
    })
    .await
}

/// The catalog movie or show with this provider identity.
pub(super) async fn lookup(
    db: &Database,
    kind: String,
    reference: Ref,
) -> anyhow::Result<Option<String>> {
    let (path, id) = match reference {
        Ref::Tmdb(id) => ("$.tmdb_id", id),
        Ref::Tvdb(id) => ("$.tvdb_id", id),
    };
    db.read("managers.status.lookup", move |db| {
        Ok(db
            .query_row(
                "SELECT id FROM media WHERE kind=?1 AND CAST(json_extract(metadata,?2) AS INTEGER)=?3 ORDER BY created_at,id LIMIT 1",
                params![kind, path, id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

/// A show's library seasons and episodes, with the caller's watched state and latest
/// resume position for each episode.
pub(super) async fn show(
    db: &Database,
    show: String,
    user: String,
) -> anyhow::Result<Vec<LibrarySeason>> {
    db.read("managers.status.show", move |db| {
        let mut query = db.prepare(&format!("SELECT s.id,s.sort_number,sc.title,e.id,e.sort_number,ec.title,
                json_extract(e.metadata,'$.overview'),json_extract(e.metadata,'$.air_date_utc'),CAST(json_extract(e.metadata,'$.runtime') AS INTEGER),
                {PRESENT},COALESCE(u.watched,0),p.position,p.duration,{MAPPED}
            FROM media s JOIN media_cards sc ON sc.id=s.id
            LEFT JOIN media e ON e.parent_id=s.id AND e.kind='episode' LEFT JOIN media_cards ec ON ec.id=e.id
            LEFT JOIN media_state u ON u.media_id=e.id AND u.user_id=?2
            LEFT JOIN edition_progress p ON p.rowid=(SELECT rowid FROM edition_progress WHERE user_id=?2 AND media_id=e.id ORDER BY updated_at DESC,edition LIMIT 1)
            WHERE s.parent_id=?1 AND s.kind='season' ORDER BY s.sort_number,s.id,e.sort_number,e.id"))?;
        let rows = query.query_map(params![show, user], |r| {
            let season = LibrarySeason { id: r.get(0)?, number: r.get(1)?, title: r.get(2)?, episodes: Vec::new() };
            let episode = match r.get::<_, Option<String>>(3)? {
                Some(id) => Some(LibraryEpisode {
                    id, number: r.get(4)?, title: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    overview: r.get::<_, Option<String>>(6)?.filter(|v| !v.trim().is_empty()),
                    air: r.get::<_, Option<String>>(7)?.and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok()).map(|at| at.timestamp()),
                    runtime: r.get::<_, Option<i64>>(8)?.filter(|v| *v > 0), available: r.get(9)?, watched: r.get(10)?,
                    position: r.get(11)?, duration: r.get(12)?, mapped: mapped(r.get(13)?),
                }),
                None => None,
            };
            Ok((season, episode))
        })?;
        let mut seasons: Vec<LibrarySeason> = Vec::new();
        for row in rows {
            let (season, episode) = row?;
            if seasons.last().is_none_or(|last| last.id != season.id) {
                seasons.push(season);
            }
            if let (Some(episode), Some(last)) = (episode, seasons.last_mut()) {
                last.episodes.push(episode);
            }
        }
        Ok(seasons)
    })
    .await
}
