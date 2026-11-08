//! Database operations for user_media.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn get_for(
    p: Principal,
    media: String,
    db: &Database,
) -> anyhow::Result<Option<Value>> {
    db.read("user_media.get_for", move |db| {
        Ok(db.query_row("SELECT COALESCE(s.favorite,0),COALESCE(s.watch_later,0),COALESCE(s.watched,0) FROM media m LEFT JOIN media_state s ON s.media_id=m.id AND s.user_id=?1 WHERE m.id=?2",params![p.user.id,media],|r|Ok(json!({"favorite":r.get::<_,bool>(0)?,"watch_later":r.get::<_,bool>(1)?,"watched":r.get::<_,bool>(2)?}))).optional()?)
    }).await
}

pub(super) async fn set_for(
    user: String,
    mid: String,
    db: &Database,
    input: Change,
) -> anyhow::Result<i32> {
    db.write("user_media.set_for", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind:Option<String> = tx.query_row("SELECT kind FROM media WHERE id=?1",[&mid],|r|r.get(0)).optional()?;
        let Some(kind) = kind else { return Ok(404); };
        if input.watch_later.is_some() && !["movie","show","season","episode"].contains(&kind.as_str()) { return Ok(400); }
        if input.watched.is_some() && !["movie","episode","track"].contains(&kind.as_str()) { return Ok(400); }
        tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite,watch_later) VALUES (?1,?2,COALESCE(?3,0),?4,COALESCE(?5,0),COALESCE(?6,0)) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=COALESCE(?3,watched),favorite=COALESCE(?5,favorite),watch_later=COALESCE(?6,watch_later),updated_at=?4",params![user,mid,input.watched,now(),input.favorite,input.watch_later])?;
        if let Some(watched) = input.watched {
            tx.execute("UPDATE edition_progress SET position=CASE WHEN ?1 THEN duration ELSE 0 END,updated_at=?2 WHERE user_id=?3 AND media_id=?4",params![watched,now(),user,mid])?;
        }
        tx.commit()?;
        Ok(200)
    }).await
}

pub(super) enum Marked {
    Missing,
    Unsupported,
    Episodes(usize),
}

/// A season's episodes, or an episode with every one before it in its series: earlier
/// seasons and episodes, Specials only among Specials.
const THROUGH: &str = "WITH target AS (
    SELECT e.id,COALESCE(e.sort_number,0) AS episode,COALESCE(s.sort_number,0) AS season,s.parent_id AS show
    FROM media e JOIN media s ON s.id=e.parent_id WHERE e.id=?1 AND e.kind='episode' AND s.kind='season')
SELECT e.id FROM target t JOIN media s ON s.parent_id=t.show AND s.kind='season'
    JOIN media e ON e.parent_id=s.id AND e.kind='episode'
WHERE e.id=t.id OR (CASE WHEN t.season=0
    THEN COALESCE(s.sort_number,0)=0 AND COALESCE(e.sort_number,0)<=t.episode
    ELSE COALESCE(s.sort_number,0)>0 AND (s.sort_number<t.season
        OR (s.sort_number=t.season AND COALESCE(e.sort_number,0)<=t.episode)) END)";

pub(super) async fn set_watched(
    user: String,
    media: String,
    watched: bool,
    through: bool,
    db: &Database,
) -> anyhow::Result<Marked> {
    db.write("user_media.set_watched", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind: Option<String> = tx
            .query_row("SELECT kind FROM media WHERE id=?1", [&media], |r| r.get(0))
            .optional()?;
        let Some(kind) = kind else {
            return Ok(Marked::Missing);
        };
        let episodes = match (kind.as_str(), through) {
            ("season", false) => tx
                .prepare("SELECT id FROM media WHERE parent_id=?1 AND kind='episode'")?
                .query_map([&media], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            ("episode", true) => tx
                .prepare(THROUGH)?
                .query_map([&media], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            ("episode", false) => vec![media],
            _ => return Ok(Marked::Unsupported),
        };
        let at = now();
        for episode in &episodes {
            tx.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,media_id) DO UPDATE SET watched=?3,updated_at=?4",params![user,episode,watched,at])?;
            tx.execute("UPDATE edition_progress SET position=CASE WHEN ?1 THEN duration ELSE 0 END,updated_at=?2 WHERE user_id=?3 AND media_id=?4",params![watched,at,user,episode])?;
        }
        tx.commit()?;
        Ok(Marked::Episodes(episodes.len()))
    })
    .await
}

/// The episodes of the series `shows` selects, in viewing order. Specials stand outside it.
/// Only those series are read and ranked, not every episode of the library: the unary plus
/// keeps SQLite, which has no statistics, from reading every episode by its kind instead of
/// finding the series' seasons and their episodes through `media_parent`.
fn series_order(shows: &str) -> String {
    format!(
        "SELECT e.id,s.parent_id AS show_id,ROW_NUMBER() OVER(PARTITION BY s.parent_id ORDER BY s.sort_number,e.sort_number,e.id) AS ord FROM media e JOIN media s ON s.id=e.parent_id WHERE +e.kind='episode' AND +s.kind='season' AND s.sort_number>0 AND s.parent_id IN ({shows})"
    )
}

/// Each series' next episode is its `candidates` row with `rn=1`: the first available,
/// unwatched one after the last watched (`anchor`), or from the start. Follows `ordered`.
const NEXT_UP: &str = "anchor AS (SELECT o.show_id,MAX(o.ord) AS ord FROM ordered o JOIN media_state u ON u.media_id=o.id WHERE u.user_id=?1 AND u.watched=1 GROUP BY o.show_id), candidates AS (SELECT o.id,o.show_id,a.ord AS anchor,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY o.ord) AS rn FROM ordered o LEFT JOIN anchor a ON a.show_id=o.show_id LEFT JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 JOIN media_cards c ON c.id=o.id WHERE c.available=1 AND COALESCE(u.watched,0)=0 AND o.ord>COALESCE(a.ord,0))";

/// When each episode was last played: its last progress report or watched mark. Unmarking
/// rewinds the progress to 0, which clears it as Jellyfin clears LastPlayedDate.
const PLAYED: &str = "played AS (SELECT media_id,MAX(at) AS at FROM (SELECT media_id,updated_at AS at FROM edition_progress WHERE user_id=?1 AND position>0 UNION ALL SELECT media_id,updated_at FROM media_state WHERE user_id=?1 AND watched=1) GROUP BY media_id)";

/// With rewatching, Jellyfin also offers the watched episode that follows the one played last.
const REWATCH: &str = "UNION ALL SELECT id,show_id,1,at FROM (SELECT o.id,o.show_id,r.at,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY o.ord) AS rn FROM (SELECT o.show_id,o.ord,p.at,ROW_NUMBER() OVER(PARTITION BY o.show_id ORDER BY p.at DESC,o.ord DESC) AS rn FROM ordered o JOIN played p ON p.media_id=o.id JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 WHERE u.watched=1) r JOIN ordered o ON o.show_id=r.show_id AND o.ord>r.ord JOIN media_state u ON u.media_id=o.id AND u.user_id=?1 JOIN media_cards c ON c.id=o.id WHERE r.rn=1 AND u.watched=1 AND c.available=1) WHERE rn=1";

/// Jellyfin's Next up (`TVSeriesManager`). Without a series, it lists the series played
/// since the cutoff, ordered by when their last watched episode was played. An entry with
/// a resume position is dropped unless resumable ones are wanted; a rewatch always is.
pub(super) async fn next_up(
    user: String,
    query: NextUp,
    db: &Database,
) -> anyhow::Result<Vec<String>> {
    db.read("user_media.next_up", move |db| {
        let sql = next_up_sql(query.show.is_some(), query.rewatching);
        Ok(db
            .prepare(&sql)?
            .query_map(
                params![user, query.show, query.since, query.root, query.resumable],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// Next up's query. It ranks the episodes of the series asked for, or else of those played
/// since the cutoff: the only ones it can list.
fn next_up_sql(show: bool, rewatching: bool) -> String {
    let rewatch = if rewatching { REWATCH } else { "" };
    let ordered = series_order(if show {
        "?2"
    } else {
        "SELECT id FROM shows WHERE at>=COALESCE(?3,0)"
    });
    format!("WITH {PLAYED},
            shows AS (SELECT s.parent_id AS id,MAX(p.at) AS at FROM played p JOIN media e ON e.id=p.media_id JOIN media s ON s.id=e.parent_id WHERE +e.kind='episode' AND +s.kind='season' GROUP BY s.parent_id),
            ordered AS ({ordered}), {NEXT_UP},
            entries AS (SELECT n.id,n.show_id,0 AS again,p.at FROM candidates n LEFT JOIN ordered o ON o.show_id=n.show_id AND o.ord=n.anchor LEFT JOIN played p ON p.media_id=o.id WHERE n.rn=1 {rewatch})
            SELECT x.id FROM entries x JOIN media sh ON sh.id=x.show_id LEFT JOIN shows a ON a.id=x.show_id
            WHERE CASE WHEN ?2 IS NULL THEN a.at>=COALESCE(?3,0) AND (?4 IS NULL OR sh.root_id=?4) ELSE x.show_id=?2 END
                AND NOT ((x.again OR NOT ?5) AND EXISTS(SELECT 1 FROM edition_progress r WHERE r.user_id=?1 AND r.media_id=x.id AND r.position>0 AND r.position<r.duration*0.9))
            ORDER BY x.at IS NULL,x.at DESC,a.at DESC,x.show_id")
}

/// The available episode that follows `media` in its series, past every episode its
/// played file also holds (any of its files when none is named).
pub(super) async fn next_episode(
    media: String,
    file: Option<String>,
    db: &Database,
) -> anyhow::Result<Option<Value>> {
    db.read("user_media.next_episode", move |db| {
        Ok(db.query_row(&next_episode_sql(),
            params![media, file],
            |r| Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"show":r.get::<_,String>(2)?,"season":r.get::<_,i64>(3)?,"number":r.get::<_,Option<i64>>(4)?.unwrap_or(0)})),
        ).optional()?)
    })
    .await
}

/// The next episode's query, which ranks only the series of episode ?1.
fn next_episode_sql() -> String {
    let ordered = series_order(
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

pub(super) async fn home_for(p: Principal, db: &Database) -> anyhow::Result<Value> {
    db.read("user_media.home_for", move |db| {
        let favorites = db.prepare("SELECT c.id,c.kind,c.title,c.available,sh.title FROM media_cards c JOIN media_state s ON s.media_id=c.id LEFT JOIN media pm ON pm.id=c.parent_id LEFT JOIN media_cards sh ON sh.id=CASE c.kind WHEN 'season' THEN c.parent_id WHEN 'episode' THEN pm.parent_id END WHERE s.user_id=?1 AND s.favorite=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],shelf_card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let later = db.prepare("SELECT c.id,c.kind,c.title,c.available,sh.title FROM media_cards c JOIN media_state s ON s.media_id=c.id LEFT JOIN media pm ON pm.id=c.parent_id LEFT JOIN media_cards sh ON sh.id=CASE c.kind WHEN 'season' THEN c.parent_id WHEN 'episode' THEN pm.parent_id END WHERE s.user_id=?1 AND s.watch_later=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],shelf_card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let resume = db.prepare("WITH latest AS (SELECT e.*,ROW_NUMBER() OVER(PARTITION BY media_id ORDER BY updated_at DESC,edition) AS rn FROM edition_progress e WHERE user_id=?1) SELECT c.id,c.kind,c.title,c.available,e.position,e.duration,e.edition,(SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1 ORDER BY f.id LIMIT 1),e.updated_at FROM latest e JOIN media_cards c ON c.id=e.media_id WHERE e.rn=1 AND EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.edition=e.edition AND f.present=1) AND c.kind IN ('movie','episode') AND c.available=1 AND e.position>0 AND e.position<e.duration*0.9 ORDER BY e.updated_at DESC LIMIT 50")?.query_map([&p.user.id],|r|{let mut c=card(r)?;c["position"]=json!(r.get::<_,f64>(4)?);c["duration"]=json!(r.get::<_,f64>(5)?);c["edition"]=json!(r.get::<_,String>(6)?);c["fileId"]=json!(r.get::<_,String>(7)?);c["last_played"]=json!(r.get::<_,i64>(8)?);Ok(c)})?.collect::<std::result::Result<Vec<_>,_>>()?;
        let next = db.prepare(&home_next_up_sql())?.query_map([&p.user.id],|r|{let mut c=card(r)?;c["show_title"]=json!(r.get::<_,String>(4)?);if let Some(duration)=r.get::<_,Option<f64>>(5)?{c["duration"]=json!(duration);}Ok(c)})?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"favorites":favorites,"watch_later":later,"continue_watching":resume,"next_up":next}))
    }).await
}

/// Home's Next up shelf. It ranks only the series it can show: those the user watched an
/// episode of, or keeps as a favorite or for later.
fn home_next_up_sql() -> String {
    let ordered = series_order(
        "SELECT s.parent_id FROM media_state u JOIN media e ON e.id=u.media_id JOIN media s ON s.id=e.parent_id WHERE u.user_id=?1 AND u.watched=1 AND e.kind='episode' UNION SELECT media_id FROM media_state WHERE user_id=?1 AND (favorite=1 OR watch_later=1)",
    );
    format!(
        "WITH ordered AS ({ordered}), {NEXT_UP} SELECT c.id,c.kind,c.title,c.available,sh.title,(SELECT CAST(json_extract(f.probe,'$.format.duration') AS REAL) FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=c.id AND f.present=1 ORDER BY f.id LIMIT 1) FROM candidates n JOIN media_cards c ON c.id=n.id JOIN media_cards sh ON sh.id=n.show_id LEFT JOIN media_state show_state ON show_state.media_id=n.show_id AND show_state.user_id=?1 WHERE n.rn=1 AND (n.anchor IS NOT NULL OR COALESCE(show_state.favorite,0)=1 OR COALESCE(show_state.watch_later,0)=1) ORDER BY sh.title LIMIT 50"
    )
}

/// A shelf card; seasons and episodes also name their series.
fn shelf_card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut value = card(row)?;
    if let Some(show) = row.get::<_, Option<String>>(4)? {
        value["show_title"] = json!(show);
    }
    Ok(value)
}
fn card(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"title":row.get::<_,String>(2)?,"available":row.get::<_,bool>(3)?}),
    )
}

pub(crate) fn validate_tracks(db: &rusqlite::Connection, items: &[String]) -> anyhow::Result<bool> {
    let mut query =
        db.prepare("SELECT EXISTS(SELECT 1 FROM media WHERE id=?1 AND kind='track')")?;
    for item in items {
        if !query.query_row([item], |r| r.get::<_, bool>(0))? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every series' episodes in viewing order, as all three queries ranked them before.
    const EVERY_SERIES: &str = "SELECT e.id,s.parent_id AS show_id,ROW_NUMBER() OVER(PARTITION BY s.parent_id ORDER BY s.sort_number,e.sort_number,e.id) AS ord FROM media e JOIN media s ON s.id=e.parent_id WHERE e.kind='episode' AND s.kind='season' AND s.sort_number>0";

    /// A query as it read before: its `ordered` ranked every series.
    fn before(sql: &str) -> String {
        let (head, tail) = sql.split_once("ordered AS (").unwrap();
        let mut depth = 1;
        let end = tail
            .char_indices()
            .find(|(_, c)| {
                depth += match c {
                    '(' => 1,
                    ')' => -1,
                    _ => 0,
                };
                depth == 0
            })
            .unwrap()
            .0;
        format!("{head}ordered AS ({EVERY_SERIES}{}", &tail[end..])
    }

    fn rows(
        db: &rusqlite::Connection,
        sql: &str,
        values: impl rusqlite::Params,
    ) -> rusqlite::Result<Vec<Vec<rusqlite::types::Value>>> {
        let mut statement = db.prepare(sql)?;
        let columns = statement.column_count();
        statement
            .query_map(values, |r| {
                (0..columns)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<_>>()
            })?
            .collect()
    }

    /// Two libraries of series whose seasons, specials, shared files, missing files, watched
    /// marks, progress, favorites and for-later marks vary from series to series and user to
    /// user, so every Next up branch has something to find.
    fn library(db: &rusqlite::Connection) -> rusqlite::Result<Vec<(String, String)>> {
        let mut seed = 7_u64;
        let mut next = move |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) % n
        };
        for user in ["alice", "bob"] {
            db.execute(
                "INSERT INTO users(id,username,role,created_at) VALUES (?1,?1,'user',1)",
                [user],
            )?;
        }
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('shows','Shows','shows','/shows'),('anime','Anime','shows','/anime')", [])?;
        let mut episodes = Vec::new();
        for show in 0..12 {
            let root = if show % 3 == 0 { "anime" } else { "shows" };
            let show_id = format!("show{show}");
            db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES (?1,?2,'show',?1,?1,1)", params![show_id, root])?;
            for season in 0..=next(3) {
                let season_id = format!("{show_id}s{season}");
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES (?1,?2,'season',?3,?1,?1,?4,1)", params![season_id, root, show_id, season as i64])?;
                for episode in 1..=1 + next(4) {
                    let id = format!("{season_id}e{episode}");
                    // Some episodes have no number and sort by their id.
                    let number = (next(6) != 0).then_some(episode);
                    db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES (?1,?2,'episode',?3,?1,?1,?4,1)", params![id, root, season_id, number.map(|n| n as i64)])?;
                    let file = format!("file-{id}");
                    db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,present,scanned_at) VALUES (?1,?2,?1,'g',1,'m',?1,'{\"format\":{\"duration\":\"1200\"}}',?3,1)", params![file, root, next(5) != 0])?;
                    db.execute(
                        "INSERT INTO media_sources VALUES (?1,?2)",
                        params![id, file],
                    )?;
                    // A file that also holds the previous episode.
                    if next(6) == 0
                        && let Some((previous, _)) = episodes.last()
                    {
                        db.execute(
                            "INSERT OR IGNORE INTO media_sources VALUES (?1,?2)",
                            params![previous, file],
                        )?;
                    }
                    episodes.push((id, file));
                }
            }
            for user in ["alice", "bob"] {
                let mark = next(4);
                if mark != 0 {
                    db.execute("INSERT INTO media_state(user_id,media_id,favorite,watch_later,updated_at) VALUES (?1,?2,?3,?4,1)", params![user, show_id, mark == 1, mark == 2])?;
                }
            }
        }
        for (at, (episode, _)) in episodes.iter().enumerate() {
            for user in ["alice", "bob"] {
                match next(5) {
                    0 => {
                        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,1,?3)", params![user, episode, 100 + at as i64])?;
                    }
                    1 => {
                        db.execute(
                            "INSERT INTO edition_progress VALUES (?1,?2,'',?3,1200,?4)",
                            params![user, episode, next(1300) as f64, 100 + at as i64],
                        )?;
                    }
                    _ => {}
                }
            }
        }
        Ok(episodes)
    }

    /// Next up, the next episode and Home's shelf list what they did when they ranked every
    /// series, whichever series, cutoff, library, resume and rewatch choices are asked for.
    #[tokio::test]
    async fn next_episodes_are_the_ones_found_before() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::open(temp.path().join("db.sqlite3")).unwrap();
        let compared = db
            .write("test.compare", |db| {
                let episodes = library(db)?;
                let mut compared = [0; 3];
                for user in ["alice", "bob", "nobody"] {
                    let sql = home_next_up_sql();
                    let shelf = rows(db, &sql, [user])?;
                    assert_eq!(shelf, rows(db, &before(&sql), [user])?, "{user}");
                    compared[0] += shelf.len();
                    let shows = (0..12).map(|show| Some(format!("show{show}")));
                    for show in [None, Some("show1s1".to_owned()), Some("missing".to_owned())].into_iter().chain(shows) {
                        for since in [None, Some(0), Some(150)] {
                            for root in [None, Some("anime"), Some("shows")] {
                                for resumable in [false, true] {
                                    for rewatching in [false, true] {
                                        let sql = next_up_sql(show.is_some(), rewatching);
                                        let values = params![user, show, since, root, resumable];
                                        let listed = rows(db, &sql, values)?;
                                        assert_eq!(listed, rows(db, &before(&sql), values)?, "{user} {show:?} {since:?} {root:?} {resumable} {rewatching}");
                                        compared[1] += listed.len();
                                    }
                                }
                            }
                        }
                    }
                }
                let sql = next_episode_sql();
                for (episode, _) in &episodes {
                    let files = episodes.iter().map(|(_, file)| Some(file.as_str()));
                    for file in std::iter::once(None).chain(files) {
                        let values = params![episode, file];
                        let next = rows(db, &sql, values)?;
                        assert_eq!(next, rows(db, &before(&sql), values)?, "{episode} {file:?}");
                        compared[2] += next.len();
                    }
                }
                Ok(compared)
            })
            .await
            .unwrap();
        assert_eq!(compared, [7, 740, 714]);
    }

    /// No query reads every episode of the library to rank them.
    #[tokio::test]
    async fn only_the_series_asked_for_are_ranked() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::open(temp.path().join("db.sqlite3")).unwrap();
        let plans = db
            .read("test.plans", |db| {
                [
                    home_next_up_sql(),
                    next_up_sql(false, true),
                    next_up_sql(true, true),
                    next_episode_sql(),
                ]
                .iter()
                .map(|sql| crate::test_support::query_plan(db, sql))
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
            })
            .await
            .unwrap();
        for plan in plans {
            // Episodes are found through their series' seasons.
            assert!(
                plan.iter()
                    .any(|step| step == "SEARCH e USING INDEX media_parent (parent_id=?)"),
                "{plan:#?}"
            );
            // Media is never read by its kind or in full.
            assert!(
                !plan.iter().any(|step| step.contains("media_by_title")
                    || ["SCAN e", "SCAN s", "SCAN m", "SCAN sh", "SCAN c"]
                        .contains(&step.as_str())),
                "{plan:#?}"
            );
        }
    }
}
