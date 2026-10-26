//! Database operations for jellyfin_import.
use super::matching::{Catalog, Episode, File, Target, Title};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use thelxinoe_core::now;
use thelxinoe_database::Database;

pub(super) const SECRET: &str = "jellyfin_import.server";
const LAST: &str = "jellyfin_import.last";

pub(super) async fn catalog(db: &Database) -> anyhow::Result<Catalog> {
    db.read("jellyfin_import.catalog", |db| {
        let lower = |value: Option<String>| {
            value
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
        };
        let files = db
            .prepare("SELECT f.path,f.edition,m.id,m.kind,season.parent_id,m.sort_number FROM media_files f JOIN media_sources s ON s.file_id=f.id JOIN media m ON m.id=s.media_id LEFT JOIN media season ON season.id=m.parent_id AND m.kind='episode' WHERE f.present=1 AND m.kind IN ('movie','episode')")?
            .query_map([], |r| {
                let kind: String = r.get(3)?;
                Ok(File {
                    path: r.get(0)?,
                    edition: r.get(1)?,
                    media: r.get(2)?,
                    show: r.get(4)?,
                    number: if kind == "episode" { r.get(5)? } else { None },
                    kind,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let titles = |kind: &str| -> anyhow::Result<Vec<Title>> {
            Ok(db
                .prepare("SELECT m.id,c.title,m.title,COALESCE(NULLIF(CAST(json_extract(m.metadata,'$.year') AS INTEGER),0),m.year),CAST(json_extract(m.metadata,'$.tmdb_id') AS TEXT),CAST(json_extract(m.metadata,'$.imdb_id') AS TEXT),CAST(json_extract(m.metadata,'$.tvdb_id') AS TEXT) FROM media m JOIN media_cards c ON c.id=m.id WHERE m.kind=?1")?
                .query_map([kind], |r| {
                    let shown: String = r.get(1)?;
                    let file: String = r.get(2)?;
                    Ok(Title {
                        id: r.get(0)?,
                        titles: if shown == file { vec![shown] } else { vec![shown, file] },
                        year: r.get(3)?,
                        tmdb: lower(r.get(4)?),
                        imdb: lower(r.get(5)?),
                        tvdb: lower(r.get(6)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        };
        let episodes = db
            .prepare("SELECT e.id,s.parent_id,s.sort_number,e.sort_number,CAST(json_extract(e.metadata,'$.tvdb_id') AS TEXT) FROM media e JOIN media s ON s.id=e.parent_id AND s.kind='season' WHERE e.kind='episode' AND s.sort_number IS NOT NULL AND e.sort_number IS NOT NULL")?
            .query_map([], |r| {
                Ok(Episode {
                    id: r.get(0)?,
                    show: r.get(1)?,
                    season: r.get(2)?,
                    number: r.get(3)?,
                    tvdb: lower(r.get(4)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Catalog {
            files,
            movies: titles("movie")?,
            shows: titles("show")?,
            episodes,
        })
    })
    .await
}

/// What the import can match against now, and the last import's report.
pub(super) async fn overview(db: &Database) -> anyhow::Result<Value> {
    db.read("jellyfin_import.overview", |db| {
        let library = db.query_row(
            "SELECT (SELECT COUNT(*) FROM media_cards WHERE kind='movie' AND available=1),
                (SELECT COUNT(*) FROM media_cards WHERE kind='episode' AND available=1),
                (SELECT MAX(last_scan) FROM library_roots),
                (SELECT COUNT(*) FROM media WHERE kind='movie'),
                (SELECT COUNT(*) FROM media WHERE kind='movie' AND COALESCE(json_extract(metadata,'$.tmdb_id'),json_extract(metadata,'$.imdb_id')) IS NOT NULL),
                (SELECT COUNT(*) FROM media WHERE kind='show'),
                (SELECT COUNT(*) FROM media WHERE kind='show' AND COALESCE(json_extract(metadata,'$.tvdb_id'),json_extract(metadata,'$.tmdb_id'),json_extract(metadata,'$.imdb_id')) IS NOT NULL),
                EXISTS(SELECT 1 FROM manager_services WHERE kind='radarr' AND enabled=1),
                EXISTS(SELECT 1 FROM manager_services WHERE kind='sonarr' AND enabled=1)",
            [],
            |r| {
                Ok(json!({
                    "movies":r.get::<_,i64>(0)?,
                    "episodes":r.get::<_,i64>(1)?,
                    "scanned_at":r.get::<_,Option<i64>>(2)?,
                    "radarr":{"connected":r.get::<_,bool>(7)?,"titles":r.get::<_,i64>(3)?,"identified":r.get::<_,i64>(4)?},
                    "sonarr":{"connected":r.get::<_,bool>(8)?,"titles":r.get::<_,i64>(5)?,"identified":r.get::<_,i64>(6)?},
                }))
            },
        )?;
        let last = db
            .query_row("SELECT value FROM settings WHERE key=?1", [LAST], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|value| serde_json::from_str::<Value>(&value).ok());
        Ok(json!({"library":library,"last":last}))
    })
    .await
}

pub(super) async fn people(db: &Database) -> anyhow::Result<Vec<(String, String)>> {
    db.read("jellyfin_import.people", |db| {
        Ok(db
            .prepare("SELECT id,username FROM users ORDER BY username COLLATE NOCASE")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

pub(super) async fn connect(
    db: &Database,
    actor: String,
    encrypted: Vec<u8>,
) -> anyhow::Result<()> {
    db.write("jellyfin_import.connect", move |db| {
        let tx = db.transaction()?;
        tx.execute("INSERT INTO secrets VALUES (?1,?2) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext", params![SECRET, encrypted])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'jellyfin_import.connect','jellyfin',?2)", params![actor, now()])?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub(super) async fn forget(db: &Database, actor: String) -> anyhow::Result<()> {
    db.write("jellyfin_import.forget", move |db| {
        let tx = db.transaction()?;
        tx.execute("DELETE FROM secrets WHERE scope=?1", [SECRET])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'jellyfin_import.forget','jellyfin',?2)", params![actor, now()])?;
        tx.commit()?;
        Ok(())
    })
    .await
}

/// One Jellyfin item's state for one person here.
pub(super) struct Entry {
    pub account: usize,
    pub user: String,
    pub target: Target,
    pub watched: bool,
    pub favorite: bool,
    /// Seconds into the item, and when Jellyfin last played it.
    pub resume: Option<(f64, Option<i64>)>,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Counts {
    pub watched: usize,
    pub already_watched: usize,
    pub resume: usize,
    pub favorites: usize,
}
impl std::ops::AddAssign for Counts {
    fn add_assign(&mut self, other: Self) {
        self.watched += other.watched;
        self.already_watched += other.already_watched;
        self.resume += other.resume;
        self.favorites += other.favorites;
    }
}

pub(super) struct Applied {
    pub accounts: Vec<Counts>,
    /// What each title-matched group adds, per account, so leaving one out can be shown.
    pub groups: HashMap<String, Vec<Counts>>,
    /// Movies and seasons the import makes eligible for watched-state auto-delete.
    pub retention: usize,
    pub users: BTreeSet<String>,
}

pub(super) type Report = Box<dyn FnOnce(&Applied) -> Value + Send>;

/// Applies the entries in one transaction. Without a report to save this is a preview: the
/// same writes run, are counted, then roll back. Nothing here is ever unwatched or removed.
pub(super) async fn apply(
    db: &Database,
    accounts: usize,
    entries: Vec<Entry>,
    commit: Option<(String, Report)>,
) -> anyhow::Result<Applied> {
    db.write("jellyfin_import.apply", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let retention_on = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM retention_policies WHERE enabled=1 AND domain<>'videos')",
            [],
            |r| r.get::<_, bool>(0),
        )?;
        let mut units = BTreeSet::new();
        if retention_on {
            for entry in entries.iter().filter(|e| e.watched) {
                for media in &entry.target.media {
                    let unit = tx
                        .query_row(
                            "SELECT CASE kind WHEN 'movie' THEN id WHEN 'episode' THEN parent_id END FROM media WHERE id=?1",
                            [media],
                            |r| r.get::<_, Option<String>>(0),
                        )
                        .optional()?
                        .flatten();
                    units.extend(unit);
                }
            }
        }
        let mut before = HashSet::new();
        for unit in &units {
            if crate::managers::retention_eligible(&tx, unit)? {
                before.insert(unit.clone());
            }
        }

        let at = now();
        let mut applied = Applied {
            accounts: vec![Counts::default(); accounts],
            groups: HashMap::new(),
            retention: 0,
            users: BTreeSet::new(),
        };
        let mut done: HashSet<(usize, String, &'static str)> = HashSet::new();
        for entry in &entries {
            let mut delta = Counts::default();
            let user = &entry.user;
            let current = |media: &str| -> rusqlite::Result<Option<(String, bool, bool)>> {
                tx.query_row(
                    "SELECT m.kind,COALESCE(s.watched,0),COALESCE(s.favorite,0) FROM media m LEFT JOIN media_state s ON s.media_id=m.id AND s.user_id=?1 WHERE m.id=?2",
                    params![user, media],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
            };
            for media in &entry.target.media {
                let Some((kind, watched, favorite)) = current(media)? else {
                    continue;
                };
                if entry.watched
                    && matches!(kind.as_str(), "movie" | "episode")
                    && done.insert((entry.account, media.clone(), "watched"))
                {
                    if watched {
                        delta.already_watched += 1;
                    } else {
                        tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite,watch_later) VALUES (?1,?2,1,?3,0,0) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=1,updated_at=?3", params![user, media, at])?;
                        tx.execute("UPDATE edition_progress SET position=duration,updated_at=?3 WHERE user_id=?1 AND media_id=?2", params![user, media, at])?;
                        delta.watched += 1;
                    }
                }
                if entry.favorite
                    && !favorite
                    && done.insert((entry.account, media.clone(), "favorite"))
                {
                    tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite,watch_later) VALUES (?1,?2,0,?3,1,0) ON CONFLICT(user_id,media_id) DO UPDATE SET favorite=1,updated_at=?3", params![user, media, at])?;
                    delta.favorites += 1;
                }
            }
            if let Some((position, played_at)) = entry.resume
                && let Some((kind, watched, _)) = current(&entry.target.resume)?
                && matches!(kind.as_str(), "movie" | "episode")
                && !watched
                && done.insert((entry.account, entry.target.resume.clone(), "resume"))
            {
                let media = &entry.target.resume;
                // Progress made here after Jellyfin last played the item stays.
                let latest: Option<i64> = tx.query_row(
                    "SELECT MAX(updated_at) FROM edition_progress WHERE user_id=?1 AND media_id=?2 AND position>0",
                    params![user, media],
                    |r| r.get(0),
                )?;
                let newer = match (latest, played_at) {
                    (None, _) => true,
                    (Some(here), Some(there)) => there > here,
                    (Some(_), None) => false,
                };
                let source: Option<(String, String)> = if newer {
                    tx.query_row(
                        "SELECT f.edition,f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition=?2 DESC,f.edition,f.id LIMIT 1",
                        params![media, entry.target.edition.as_deref().unwrap_or("")],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?
                } else {
                    None
                };
                let duration = source.as_ref().and_then(|(_, probe)| {
                    serde_json::from_str::<Value>(probe).ok()?["format"]["duration"]
                        .as_str()?
                        .parse::<f64>()
                        .ok()
                        .filter(|d| d.is_finite() && *d > 0.0)
                });
                if let (Some((edition, _)), Some(duration)) = (source, duration) {
                    tx.execute("INSERT INTO edition_progress(user_id,media_id,edition,position,duration,updated_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,media_id,edition) DO UPDATE SET position=excluded.position,duration=excluded.duration,updated_at=excluded.updated_at", params![user, media, edition, position.min(duration), duration, played_at.unwrap_or(at).min(at)])?;
                    delta.resume += 1;
                }
            }
            if delta.watched + delta.favorites + delta.resume > 0 {
                applied.users.insert(user.clone());
            }
            applied.accounts[entry.account] += delta;
            if let Some(group) = &entry.target.group {
                applied
                    .groups
                    .entry(group.clone())
                    .or_insert_with(|| vec![Counts::default(); accounts])[entry.account] += delta;
            }
        }

        for unit in &units {
            if !before.contains(unit) && crate::managers::retention_eligible(&tx, unit)? {
                applied.retention += 1;
            }
        }
        if let Some((actor, report)) = commit {
            let report = report(&applied);
            tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![LAST, report.to_string()])?;
            tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'jellyfin_import.import','jellyfin',?2)", params![actor, at])?;
            tx.commit()?;
        }
        Ok(applied)
    })
    .await
}
