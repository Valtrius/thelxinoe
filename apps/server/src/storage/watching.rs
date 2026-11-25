//! Watching: each user's Watched marks, resume points, In progress and Next up. The web app,
//! the Jellyfin-compatible API, playback and the Jellyfin import read and write them here and
//! only translate the answers to their wire format. Where those surfaces disagree, the
//! difference is an option named here.
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use thelxinoe_database::Database;

/// A resume point at or past this share of its duration is finished: playback that far marks
/// the item watched, and the item is no longer in progress nor resumed from there.
pub(crate) const FINISHED: f64 = 0.9;

pub(crate) fn finished(position: f64, duration: f64) -> bool {
    position >= duration * FINISHED
}

/// The kinds a user watches and resumes themselves.
const PLAYABLE: [&str; 3] = ["movie", "episode", "track"];

/// A media file of the item under alias `m` is on disk.
fn present(m: &str) -> String {
    format!(
        "EXISTS(SELECT 1 FROM media_sources ps JOIN media_files pf ON pf.id=ps.file_id WHERE ps.media_id={m}.id AND pf.present=1)"
    )
}

fn kind(db: &Connection, media: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT kind FROM media WHERE id=?1", [media], |r| r.get(0))
        .optional()
}

fn ids<P: rusqlite::Params>(db: &Connection, sql: &str, args: P) -> rusqlite::Result<Vec<String>> {
    db.prepare(sql)?.query_map(args, |r| r.get(0))?.collect()
}

// Writes. Each runs inside the caller's write transaction.

/// A user's change to their marks on one media item; `None` leaves a mark as it is.
#[derive(Default)]
pub(crate) struct Change {
    pub(crate) watched: Option<bool>,
    pub(crate) favorite: Option<bool>,
    pub(crate) watch_later: Option<bool>,
    /// A resume point, in seconds, stored at the edition last resumed.
    pub(crate) resume: Option<f64>,
}

pub(crate) enum Changed {
    Missing,
    Unsupported,
    /// No edition with a file present has a known length to hold the resume point.
    NoEdition,
    Done,
}

/// Changes one item's marks. Watched and resume points belong to playable items, Watch
/// Later to movies, shows, seasons and episodes, and any item can be a favorite. Watching
/// fills the item's resume points and unwatching rewinds them, unless the same change sets
/// one: then that resume point stays.
pub(crate) fn change(
    tx: &Connection,
    user: &str,
    media: &str,
    change: Change,
    at: i64,
) -> rusqlite::Result<Changed> {
    let Some(kind) = kind(tx, media)? else {
        return Ok(Changed::Missing);
    };
    let kind = kind.as_str();
    if change.watch_later.is_some() && !["movie", "show", "season", "episode"].contains(&kind)
        || (change.watched.is_some() || change.resume.is_some()) && !PLAYABLE.contains(&kind)
    {
        return Ok(Changed::Unsupported);
    }
    if let Some(position) = change.resume
        && !set_resume_point(tx, user, media, position, Edition::LastResumed, at)?
    {
        return Ok(Changed::NoEdition);
    }
    tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite,watch_later) VALUES (?1,?2,COALESCE(?3,0),?4,COALESCE(?5,0),COALESCE(?6,0)) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=COALESCE(?3,watched),favorite=COALESCE(?5,favorite),watch_later=COALESCE(?6,watch_later),updated_at=?4", params![user, media, change.watched, at, change.favorite, change.watch_later])?;
    if let Some(watched) = change.watched
        && (watched || change.resume.is_none())
    {
        follow(tx, user, media, watched, at)?;
    }
    Ok(Changed::Done)
}

/// What a Watched mark covers.
pub(crate) enum Scope<'a> {
    /// A movie, episode or track alone. Jellyfin clients mark only these.
    Item,
    /// A season's episodes, or one episode.
    Episodes,
    /// An episode and every one before it in its show: earlier seasons and episodes,
    /// Specials only among Specials.
    Through,
    /// The other episodes this file holds. Playback marks them when it finishes the file;
    /// marking one of them by hand leaves the others.
    Siblings { file: &'a str },
}

pub(crate) enum Marked {
    Missing,
    Unsupported,
    Items(usize),
}

/// A season's episodes, or an episode with every one before it in its show.
const THROUGH: &str = "WITH target AS (
    SELECT e.id,COALESCE(e.sort_number,0) AS episode,COALESCE(s.sort_number,0) AS season,s.parent_id AS show
    FROM media e JOIN media s ON s.id=e.parent_id WHERE e.id=?1 AND e.kind='episode' AND s.kind='season')
SELECT e.id FROM target t JOIN media s ON s.parent_id=t.show AND s.kind='season'
    JOIN media e ON e.parent_id=s.id AND e.kind='episode'
WHERE e.id=t.id OR (CASE WHEN t.season=0
    THEN COALESCE(s.sort_number,0)=0 AND COALESCE(e.sort_number,0)<=t.episode
    ELSE COALESCE(s.sort_number,0)>0 AND (s.sort_number<t.season
        OR (s.sort_number=t.season AND COALESCE(e.sort_number,0)<=t.episode)) END)";

/// Marks what `scope` covers from `media` watched or not; resume points follow as in
/// [`change`].
pub(crate) fn mark(
    tx: &Connection,
    user: &str,
    media: &str,
    watched: bool,
    scope: Scope<'_>,
    at: i64,
) -> rusqlite::Result<Marked> {
    let Some(kind) = kind(tx, media)? else {
        return Ok(Marked::Missing);
    };
    let items = match (scope, kind.as_str()) {
        (Scope::Item, kind) if PLAYABLE.contains(&kind) => vec![media.to_owned()],
        (Scope::Episodes, "season") => ids(
            tx,
            "SELECT id FROM media WHERE parent_id=?1 AND kind='episode'",
            [media],
        )?,
        (Scope::Episodes, "episode") => vec![media.to_owned()],
        (Scope::Through, "episode") => ids(tx, THROUGH, [media])?,
        (Scope::Siblings { file }, _) => ids(
            tx,
            "SELECT o.media_id FROM media_sources o JOIN media m ON m.id=o.media_id WHERE o.file_id=?1 AND o.media_id<>?2 AND m.kind='episode'",
            [file, media],
        )?,
        _ => return Ok(Marked::Unsupported),
    };
    for item in &items {
        tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=?3,updated_at=?4", params![user, item, watched, at])?;
        follow(tx, user, item, watched, at)?;
    }
    Ok(Marked::Items(items.len()))
}

/// Watching fills an item's resume points, so none is in progress; unwatching rewinds them,
/// which also clears when the item was last played.
fn follow(
    tx: &Connection,
    user: &str,
    media: &str,
    watched: bool,
    at: i64,
) -> rusqlite::Result<()> {
    tx.execute("UPDATE edition_progress SET position=CASE WHEN ?1 THEN duration ELSE 0 END,updated_at=?2 WHERE user_id=?3 AND media_id=?4", params![watched, at, user, media])?;
    Ok(())
}

/// The edition a resume point is stored at, among those with a file present.
pub(crate) enum Edition<'a> {
    /// The one the user resumed last, or else the first.
    LastResumed,
    /// This one, or else the first.
    Named(&'a str),
}

/// Stores a resume point, clamped to its edition's length. False when no edition with a
/// file present has a known length.
pub(crate) fn set_resume_point(
    tx: &Connection,
    user: &str,
    media: &str,
    position: f64,
    edition: Edition<'_>,
    at: i64,
) -> rusqlite::Result<bool> {
    let read = |r: &rusqlite::Row<'_>| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?));
    let source = match edition {
        Edition::LastResumed => tx.query_row("SELECT f.edition,f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY (f.edition=(SELECT edition FROM edition_progress WHERE user_id=?2 AND media_id=?1 ORDER BY updated_at DESC,edition LIMIT 1)) DESC,f.edition,f.id LIMIT 1", params![media, user], read),
        Edition::Named(edition) => tx.query_row("SELECT f.edition,f.probe FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition=?2 DESC,f.edition,f.id LIMIT 1", params![media, edition], read),
    }
    .optional()?;
    let Some((edition, probe)) = source else {
        return Ok(false);
    };
    let Some(duration) = serde_json::from_str::<serde_json::Value>(&probe)
        .ok()
        .and_then(|probe| probe["format"]["duration"].as_str()?.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
    else {
        return Ok(false);
    };
    tx.execute("INSERT INTO edition_progress(user_id,media_id,edition,position,duration,updated_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,media_id,edition) DO UPDATE SET position=excluded.position,duration=excluded.duration,updated_at=excluded.updated_at", params![user, media, edition, position.min(duration), duration, at])?;
    Ok(true)
}

/// When the user last left a resume point on the item.
pub(crate) fn last_resumed_at(
    db: &Connection,
    user: &str,
    media: &str,
) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT MAX(updated_at) FROM edition_progress WHERE user_id=?1 AND media_id=?2 AND position>0",
        params![user, media],
        |r| r.get(0),
    )
}

/// A playback progress report.
pub(crate) struct Played<'a> {
    pub(crate) media: &'a str,
    pub(crate) edition: &'a str,
    /// The file played, when it is a library file.
    pub(crate) file: Option<&'a str>,
    pub(crate) position: f64,
    pub(crate) duration: f64,
}

/// Playback moves the resume point of the edition it plays. Reaching [`FINISHED`] marks the
/// item watched, never unwatched, and with it the other episodes its file holds.
pub(crate) fn record_playback(
    tx: &Connection,
    user: &str,
    played: Played<'_>,
    at: i64,
) -> rusqlite::Result<()> {
    let Played {
        media,
        edition,
        file,
        position,
        duration,
    } = played;
    tx.execute("INSERT INTO edition_progress(user_id,media_id,edition,position,duration,updated_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,media_id,edition) DO UPDATE SET position=excluded.position,duration=excluded.duration,updated_at=excluded.updated_at", params![user, media, edition, position, duration, at])?;
    let finished = finished(position, duration);
    tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=MAX(watched,excluded.watched),updated_at=excluded.updated_at", params![user, media, finished, at])?;
    if finished && let Some(file) = file {
        mark(tx, user, media, true, Scope::Siblings { file }, at)?;
    }
    Ok(())
}

// Reads.

/// A resume point at one edition.
pub(crate) struct ResumePoint {
    pub(crate) edition: String,
    pub(crate) position: f64,
    pub(crate) duration: f64,
}

/// Whether the user watched the playable item, and its resume point at each edition.
pub(crate) fn item(
    db: &Connection,
    user: &str,
    media: &str,
) -> rusqlite::Result<(bool, Vec<ResumePoint>)> {
    let points = db
        .prepare("SELECT edition,position,duration FROM edition_progress WHERE user_id=?1 AND media_id=?2")?
        .query_map(params![user, media], |r| {
            Ok(ResumePoint {
                edition: r.get(0)?,
                position: r.get(1)?,
                duration: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let watched = db
        .query_row(
            "SELECT watched FROM media_state WHERE user_id=?1 AND media_id=?2",
            params![user, media],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false);
    Ok((watched, points))
}

/// The episodes of the show or season under alias `m` that its Watched rolls up: all of
/// them, Specials and episodes without a file too. Jellyfin's UnplayedItemCount counts
/// fewer: see [`unwatched_count_sql`].
fn all_episodes(m: &str) -> String {
    format!(
        "SELECT id FROM media WHERE parent_id={m}.id AND kind IN ('movie','episode') UNION ALL SELECT e.id FROM media s JOIN media e ON e.parent_id=s.id WHERE s.parent_id={m}.id AND s.kind IN ('show','season') AND e.kind IN ('movie','episode')"
    )
}

/// SQL: whether `user` watched the item under alias `m`, whose own marks join as `u`. A
/// show or season is watched when it has episodes and the user watched all of them.
pub(crate) fn watched_sql(m: &str, u: &str, user: &str) -> String {
    let episodes = all_episodes(m);
    format!(
        "(CASE WHEN {m}.kind IN ('show','season') THEN EXISTS({episodes}) AND NOT EXISTS(SELECT 1 FROM ({episodes}) x LEFT JOIN media_state xw ON xw.media_id=x.id AND xw.user_id={user} WHERE COALESCE(xw.watched,0)=0) ELSE COALESCE({u}.watched,0) END)"
    )
}

/// SQL: how many episodes of the show or season under alias `m` `user` has not watched,
/// counting only those with a file present and, for a show, those outside Specials. NULL
/// for other kinds. Watched counts every episode: see [`watched_sql`].
pub(crate) fn unwatched_count_sql(m: &str, user: &str) -> String {
    let present = present("e");
    format!(
        "(CASE {m}.kind WHEN 'show' THEN (SELECT COUNT(*) FROM media s JOIN media e ON e.parent_id=s.id LEFT JOIN media_state es ON es.media_id=e.id AND es.user_id={user} WHERE s.parent_id={m}.id AND s.kind='season' AND COALESCE(s.sort_number,1)>0 AND e.kind='episode' AND COALESCE(es.watched,0)=0 AND {present}) \
    WHEN 'season' THEN (SELECT COUNT(*) FROM media e LEFT JOIN media_state es ON es.media_id=e.id AND es.user_id={user} WHERE e.parent_id={m}.id AND e.kind='episode' AND COALESCE(es.watched,0)=0 AND {present}) END)"
    )
}

/// SQL: the rowid of the resume point `user` left last on `media`.
pub(crate) fn latest_resume_point(media: &str, user: &str) -> String {
    format!(
        "(SELECT rowid FROM edition_progress WHERE media_id={media} AND user_id={user} ORDER BY updated_at DESC,edition LIMIT 1)"
    )
}

/// SQL: whether the resume point under alias `ep` is in progress.
pub(crate) fn in_progress_sql(ep: &str) -> String {
    format!("({ep}.position>0 AND {ep}.position<{ep}.duration*{FINISHED})")
}

/// How the web app's cards read an item.
pub(crate) struct CardFacts {
    pub(crate) watched: bool,
    /// Something in it is watched or has a resume point.
    pub(crate) started: bool,
}

/// The web app's facts for the cards `ids` (a JSON array). Only movies, episodes, seasons
/// and shows read as watched or started: a track's own mark is left out, which Jellyfin
/// clients do see.
pub(crate) fn cards(
    db: &Connection,
    user: &str,
    ids: &str,
) -> rusqlite::Result<HashMap<String, CardFacts>> {
    let watched = watched_sql("m", "u", "?2");
    let episodes = all_episodes("m");
    let resumed = |m: &str| {
        format!(
            "EXISTS(SELECT 1 FROM edition_progress r WHERE r.user_id=?2 AND r.media_id={m} AND r.position>0)"
        )
    };
    let (episode_resumed, own_resumed) = (resumed("x.id"), resumed("m.id"));
    let sql = format!("SELECT m.id,CASE WHEN m.kind IN ('movie','episode','show','season') THEN {watched} ELSE 0 END,
        CASE WHEN m.kind IN ('show','season') THEN EXISTS(SELECT 1 FROM ({episodes}) x LEFT JOIN media_state xw ON xw.media_id=x.id AND xw.user_id=?2 WHERE COALESCE(xw.watched,0)=1 OR {episode_resumed})
            WHEN m.kind IN ('movie','episode') THEN COALESCE(u.watched,0) OR {own_resumed} ELSE 0 END
        FROM media m LEFT JOIN media_state u ON u.media_id=m.id AND u.user_id=?2 WHERE m.id IN (SELECT value FROM json_each(?1))");
    db.prepare(&sql)?
        .query_map(params![ids, user], |r| {
            Ok((
                r.get(0)?,
                CardFacts {
                    watched: r.get(1)?,
                    started: r.get(2)?,
                },
            ))
        })?
        .collect()
}

/// A movie or episode on the web app's Continue watching shelf.
pub(crate) struct InProgress {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) available: bool,
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) edition: String,
    /// A present file of the resume point's edition.
    pub(crate) file: String,
    pub(crate) last_played: i64,
}

/// Continue watching: each item's latest resume point, when it is in progress and a file of
/// its own edition is present. Jellyfin's Resume list needs any file of the item.
pub(crate) fn continue_watching(db: &Connection, user: &str) -> rusqlite::Result<Vec<InProgress>> {
    let in_progress = in_progress_sql("e");
    db.prepare(&format!("WITH latest AS (SELECT e.*,ROW_NUMBER() OVER(PARTITION BY media_id ORDER BY updated_at DESC,edition) AS rn FROM edition_progress e WHERE user_id=?1) SELECT c.id,c.kind,c.title,c.available,e.position,e.duration,e.edition,(SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1 ORDER BY f.id LIMIT 1),e.updated_at FROM latest e JOIN media_cards c ON c.id=e.media_id WHERE e.rn=1 AND EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1) AND c.kind IN ('movie','episode') AND c.available=1 AND {in_progress} ORDER BY e.updated_at DESC LIMIT 50"))?
        .query_map([user], |r| {
            Ok(InProgress {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                available: r.get(3)?,
                position: r.get(4)?,
                duration: r.get(5)?,
                edition: r.get(6)?,
                file: r.get(7)?,
                last_played: r.get(8)?,
            })
        })?
        .collect()
}

/// The episodes of the shows `shows` selects, in viewing order. Specials stand outside it.
/// Only those shows are read and ranked, not every episode of the library: the unary plus
/// keeps SQLite, which has no statistics, from reading every episode by its kind instead of
/// finding the shows' seasons and their episodes through `media_parent`.
fn show_order(shows: &str) -> String {
    format!(
        "SELECT e.id,s.parent_id AS show_id,ROW_NUMBER() OVER(PARTITION BY s.parent_id ORDER BY s.sort_number,e.sort_number,e.id) AS ord FROM media e JOIN media s ON s.id=e.parent_id WHERE +e.kind='episode' AND +s.kind='season' AND s.sort_number>0 AND s.parent_id IN ({shows})"
    )
}

/// Each show's next episode is its `candidates` row with `rn=1`: the first available,
/// unwatched one after the last watched (`anchor`), or from the start. Follows `ordered`.
const NEXT_UP: &str = "anchor AS (SELECT o.show_id,MAX(o.ord) AS ord FROM ordered o JOIN media_state u ON u.media_id=o.id WHERE u.user_id=?1 AND u.watched=1 GROUP BY o.show_id), candidates AS (SELECT o.id,o.show_id,a.ord AS anchor,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY o.ord) AS rn FROM ordered o LEFT JOIN anchor a ON a.show_id=o.show_id LEFT JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 JOIN media_cards c ON c.id=o.id WHERE c.available=1 AND COALESCE(u.watched,0)=0 AND o.ord>COALESCE(a.ord,0))";

/// When each episode was last played: its last progress report or watched mark. Unmarking
/// rewinds the progress to 0, which clears it as Jellyfin clears LastPlayedDate.
const PLAYED: &str = "played AS (SELECT media_id,MAX(at) AS at FROM (SELECT media_id,updated_at AS at FROM edition_progress WHERE user_id=?1 AND position>0 UNION ALL SELECT media_id,updated_at FROM media_state WHERE user_id=?1 AND watched=1) GROUP BY media_id)";

/// With rewatching, Jellyfin also offers the watched episode that follows the one played last.
const REWATCH: &str = "UNION ALL SELECT id,show_id,1,at FROM (SELECT o.id,o.show_id,r.at,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY o.ord) AS rn FROM (SELECT o.show_id,o.ord,p.at,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY p.at DESC,o.ord DESC) AS rn FROM ordered o JOIN played p ON p.media_id=o.id JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 WHERE u.watched=1) r JOIN ordered o ON o.show_id=r.show_id AND o.ord>r.ord JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 JOIN media_cards c ON c.id=o.id WHERE r.rn=1 AND u.watched=1 AND c.available=1) WHERE rn=1";

/// An episode on the web app's Next up shelf.
pub(crate) struct NextUpEpisode {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) available: bool,
    pub(crate) show_title: String,
    /// The length of its first present file.
    pub(crate) duration: Option<f64>,
}

/// The web app's Next up shelf: one episode for each show the user watched an episode of,
/// or keeps as a favorite or for later, by show title.
pub(crate) fn home_next_up(db: &Connection, user: &str) -> rusqlite::Result<Vec<NextUpEpisode>> {
    db.prepare(&home_next_up_sql())?
        .query_map([user], |r| {
            Ok(NextUpEpisode {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                available: r.get(3)?,
                show_title: r.get(4)?,
                duration: r.get(5)?,
            })
        })?
        .collect()
}

/// It ranks only the shows it can list.
fn home_next_up_sql() -> String {
    let ordered = show_order(
        "SELECT s.parent_id FROM media_state u JOIN media e ON e.id=u.media_id JOIN media s ON s.id=e.parent_id WHERE u.user_id=?1 AND u.watched=1 AND e.kind='episode' UNION SELECT media_id FROM media_state WHERE user_id=?1 AND (favorite=1 OR watch_later=1)",
    );
    format!(
        "WITH ordered AS ({ordered}), {NEXT_UP} SELECT c.id,c.kind,c.title,c.available,sh.title,(SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.present=1 ORDER BY f.id LIMIT 1) FROM candidates n JOIN media_cards c ON c.id=n.id JOIN media_cards sh ON sh.id=n.show_id LEFT JOIN media_state show_state ON show_state.media_id=n.show_id AND show_state.user_id=?1 WHERE n.rn=1 AND (n.anchor IS NOT NULL OR COALESCE(show_state.favorite,0)=1 OR COALESCE(show_state.watch_later,0)=1) ORDER BY sh.title LIMIT 50"
    )
}

/// What Jellyfin's Next up asks for.
pub(crate) struct NextUp {
    /// This show alone, whether or not it was played.
    pub(crate) show: Option<String>,
    pub(crate) root: Option<String>,
    pub(crate) since: Option<i64>,
    pub(crate) resumable: bool,
    pub(crate) rewatching: bool,
}

/// Jellyfin's Next up (`TVSeriesManager`), as episode IDs. Without a show, it lists the shows
/// played since the cutoff, ordered by when their last watched episode was played. An episode
/// in progress is dropped unless resumable ones are wanted; a rewatch always is.
pub(crate) async fn next_up(
    db: &Database,
    user: String,
    query: NextUp,
) -> anyhow::Result<Vec<String>> {
    db.read("watching.next_up", move |db| {
        let sql = next_up_sql(query.show.is_some(), query.rewatching);
        Ok(ids(
            db,
            &sql,
            params![user, query.show, query.since, query.root, query.resumable],
        )?)
    })
    .await
}

/// It ranks the episodes of the show asked for, or else of those played since the cutoff:
/// the only ones it can list.
fn next_up_sql(show: bool, rewatching: bool) -> String {
    let rewatch = if rewatching { REWATCH } else { "" };
    let ordered = show_order(if show {
        "?2"
    } else {
        "SELECT id FROM shows WHERE at>=COALESCE(?3,0)"
    });
    let in_progress = in_progress_sql("r");
    format!("WITH {PLAYED},
            shows AS (SELECT s.parent_id AS id,MAX(p.at) AS at FROM played p JOIN media e ON e.id=p.media_id JOIN media s ON s.id=e.parent_id WHERE +e.kind='episode' AND +s.kind='season' GROUP BY s.parent_id),
            ordered AS ({ordered}), {NEXT_UP},
            entries AS (SELECT n.id,n.show_id,0 AS again,p.at FROM candidates n LEFT JOIN ordered o ON o.show_id=n.show_id AND o.ord=n.anchor LEFT JOIN played p ON p.media_id=o.id WHERE n.rn=1 {rewatch})
            SELECT x.id FROM entries x JOIN media sh ON sh.id=x.show_id LEFT JOIN shows a ON a.id=x.show_id
            WHERE CASE WHEN ?2 IS NULL THEN a.at>=COALESCE(?3,0) AND (?4 IS NULL OR sh.root_id=?4) ELSE x.show_id=?2 END
                AND NOT ((x.again OR NOT ?5) AND EXISTS(SELECT 1 FROM edition_progress r WHERE r.user_id=?1 AND r.media_id=x.id AND {in_progress}))
            ORDER BY x.at IS NULL,x.at DESC,a.at DESC,x.show_id")
}

/// The episode Up next offers.
pub(crate) struct UpNext {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) show: String,
    pub(crate) season: i64,
    pub(crate) number: i64,
}

/// Up next: the available episode that follows `media` in its show, past every episode its
/// played file also holds (any of its files when none is named).
pub(crate) async fn up_next(
    db: &Database,
    media: String,
    file: Option<String>,
) -> anyhow::Result<Option<UpNext>> {
    db.read("watching.up_next", move |db| {
        Ok(db
            .query_row(&up_next_sql(), params![media, file], |r| {
                Ok(UpNext {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    show: r.get(2)?,
                    season: r.get(3)?,
                    number: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                })
            })
            .optional()?)
    })
    .await
}

/// It ranks only the show of episode ?1.
fn up_next_sql() -> String {
    let ordered = show_order(
        "SELECT s.parent_id FROM media e JOIN media s ON s.id=e.parent_id WHERE e.id=?1",
    );
    format!("WITH ordered AS ({ordered}),
            target AS (SELECT show_id FROM ordered WHERE id=?1),
            covered AS (SELECT MAX(o.ord) AS ord FROM ordered o JOIN target t ON t.show_id=o.show_id
                WHERE o.id=?1 OR o.id IN (SELECT other.media_id FROM media_sources mine JOIN media_sources other ON other.file_id=mine.file_id
                    WHERE mine.media_id=?1 AND mine.file_id=COALESCE(?2,mine.file_id)))
            SELECT c.id,c.title,sh.title,s.sort_number,e.sort_number FROM ordered o JOIN target t ON t.show_id=o.show_id JOIN covered a ON o.ord>a.ord
                JOIN media_cards c ON c.id=o.id JOIN media e ON e.id=o.id JOIN media s ON s.id=e.parent_id JOIN media_cards sh ON sh.id=o.show_id
            WHERE c.available=1 ORDER BY o.ord LIMIT 1")
}

#[cfg(test)]
#[path = "watching_tests.rs"]
mod tests;
