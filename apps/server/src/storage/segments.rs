//! Database operations for segments.
use super::*;
use std::collections::HashMap;
use thelxinoe_database::Database;

/// What applies to one file: a manual correction replaces everything, otherwise the best source
/// of each kind wins. `rows` must already be in source priority order.
fn applicable(rows: Vec<Segment>, manual: bool) -> Vec<Segment> {
    let mut selected = Vec::<Segment>::new();
    for s in rows {
        if manual && s.source != "manual" {
            continue;
        }
        if selected
            .iter()
            .any(|other| other.source != s.source && other.kind == s.kind)
        {
            continue;
        }
        selected.push(s);
    }
    selected.sort_by(|a, b| a.start.total_cmp(&b.start));
    selected
}

pub(super) async fn resolved(
    media: String,
    file: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<Vec<Segment>> {
    db.read("segments.resolved", move|db|{
        let manual=db.query_row("SELECT EXISTS(SELECT 1 FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3)",params![media,file,generation],|r|r.get::<_,bool>(0))?;
        let rows=db.prepare("SELECT id,kind,start,end,source,confidence FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 ORDER BY CASE source WHEN 'manual' THEN 0 WHEN 'theintrodb' THEN 1 ELSE 2 END,start")?.query_map(params![media,file,generation],|r|Ok(Segment{id:r.get(0)?,kind:r.get(1)?,start:r.get(2)?,end:r.get(3)?,source:r.get(4)?,confidence:r.get(5)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(applicable(rows,manual))
    }).await
}

pub(super) async fn for_jellyfin(
    user: String,
    auth: String,
    mid: String,
    db: &Database,
) -> anyhow::Result<(Option<String>, i64)> {
    db.read("segments.for_jellyfin", move|db|{
        let file=db.query_row("SELECT s.file_id FROM playback_sessions s JOIN media_files f ON f.id=s.file_id AND f.generation=s.generation AND f.present=1 WHERE s.media_id=?1 AND s.user_id=?2 AND s.auth_session_id=?3 AND s.state IN ('ready','playing','paused') ORDER BY s.created_at DESC,s.rowid DESC LIMIT 1",params![mid,user,auth],|r|r.get::<_,String>(0)).optional()?;
        let count=db.query_row("SELECT COUNT(*) FROM media_sources s JOIN media_files f ON f.id=s.file_id AND f.present=1 WHERE s.media_id=?1",[mid],|r|r.get::<_,i64>(0))?;
        Ok((file,count))
    }).await
}

pub(super) async fn preferences_for(user: String, db: &Database) -> anyhow::Result<Option<String>> {
    db.read("segments.preferences_for", move |db| {
        Ok(db
            .query_row(
                "SELECT value FROM segment_preferences WHERE user_id=?1",
                [user],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn save_preferences(
    db: &Database,
    input: Value,
    p: Principal,
) -> anyhow::Result<()> {
    db.write("segments.save_preferences", move|db|{db.execute("INSERT INTO segment_preferences VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![p.user.id,input.to_string()])?;Ok(())}).await
}

pub(super) async fn save(
    db: &Database,
    media: String,
    input: Edit,
    p: Principal,
    src: thelxinoe_playback::Source,
) -> anyhow::Result<()> {
    db.write("segments.save", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM media_segments WHERE media_id=?1 AND file_id=?2 AND generation=?3 AND source='manual'",params![media,src.id,src.generation])?;
        tx.execute("DELETE FROM segment_overrides WHERE media_id=?1 AND file_id=?2 AND generation=?3",params![media,src.id,src.generation])?;
        if !input.reset {
            tx.execute("INSERT INTO segment_overrides VALUES (?1,?2,?3)",params![media,src.id,src.generation])?;
            for s in input.items {insert(&tx,&src,&s,"manual")?;}
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'segments.edit',?2,?3)",params![p.user.id,media,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn reanalyze_read_media(db: &Database, mid: String) -> anyhow::Result<bool> {
    db.read("segments.reanalyze_read_media", move |db| {
        Ok(
            db.query_row("SELECT kind='episode' FROM media WHERE id=?1", [mid], |r| {
                r.get::<_, bool>(0)
            })?,
        )
    })
    .await
}

pub(super) async fn reanalyze_write_segment_analysis(
    db: &Database,
    media: String,
    src: thelxinoe_playback::Source,
) -> anyhow::Result<()> {
    db.write("segments.reanalyze_write_segment_analysis", move|db|{db.execute("INSERT INTO segment_analysis(media_id,file_id,generation,state,requested_at) VALUES (?1,?2,?3,'queued',?4) ON CONFLICT(media_id,file_id,generation) DO UPDATE SET state=CASE WHEN state='running' THEN state ELSE 'queued' END,requested_at=excluded.requested_at,error=NULL",params![media,src.id,src.generation,now()])?;Ok(())}).await
}

pub(super) async fn config(db: &Database) -> anyhow::Result<Config> {
    db.read("segments.config", |db| {
        Ok(serde_json::from_str(&db.query_row(
            "SELECT value FROM settings WHERE key='segments.config'",
            [],
            |r| r.get::<_, String>(0),
        )?)?)
    })
    .await
}

pub(super) async fn configure(db: &Database, input: Config) -> anyhow::Result<()> {
    db.write("segments.configure", move |db| {
        db.execute(
            "UPDATE settings SET value=?1 WHERE key='segments.config'",
            [serde_json::to_string(&input)?],
        )?;
        Ok(())
    })
    .await
}

/// Every current episode file, whether or not the worker has reached it yet. A file shared by
/// several episodes is never analyzed, so it only appears if a request for it left a row.
const ANALYZED_EPISODES: &str = "SELECT m.id,f.id,
        CASE WHEN season.sort_number IS NULL OR m.sort_number IS NULL THEN m.title
          ELSE card.title||' · '||printf('S%02dE%02d',season.sort_number,m.sort_number) END,
        series.id,card.title,a.state,a.error,
        iif(a.state IN ('complete','failed'),a.completed_at,NULL),
        EXISTS(SELECT 1 FROM segment_overrides o WHERE o.media_id=m.id AND o.file_id=f.id AND o.generation=f.generation)
    FROM media m
    JOIN media_sources s ON s.media_id=m.id
    JOIN media_files f ON f.id=s.file_id AND f.present=1
    JOIN media season ON season.id=m.parent_id
    JOIN media series ON series.id=season.parent_id
    JOIN media_cards card ON card.id=series.id
    LEFT JOIN segment_analysis a ON a.media_id=m.id AND a.file_id=f.id AND a.generation=f.generation
    WHERE m.kind='episode' AND (a.media_id IS NOT NULL OR NOT EXISTS(
      SELECT 1 FROM media_sources other WHERE other.file_id=f.id AND other.media_id<>m.id
    ))";
/// Work in progress, then what finished most recently.
const LATEST_FIRST: &str = "ORDER BY CASE WHEN a.state='running' THEN 0 WHEN a.state IN ('complete','failed') THEN 2 ELSE 1 END,
    iif(a.state IN ('complete','failed'),a.completed_at,NULL) DESC,
    COALESCE(a.requested_at,m.created_at) DESC,m.rowid DESC,f.id";
const IN_EPISODE_ORDER: &str = "ORDER BY season.sort_number,m.sort_number,m.title,f.id";
const ANALYSIS_PAGE: usize = 100;

#[derive(Default, Serialize)]
struct Coverage {
    episodes: i64,
    intro: i64,
    credits: i64,
    nothing: i64,
    failed: i64,
    waiting: i64,
}

impl Coverage {
    fn count(&mut self, status: &str, segments: &[Segment]) {
        let has = |kind: &str| i64::from(segments.iter().any(|s| s.kind == kind));
        self.episodes += 1;
        self.intro += has("Intro");
        self.credits += has("Credits");
        match status {
            "nothing" => self.nothing += 1,
            "failed" => self.failed += 1,
            "waiting" | "running" => self.waiting += 1,
            _ => {}
        }
    }
}

#[derive(Serialize)]
struct ShowCoverage {
    id: String,
    title: String,
    #[serde(flatten)]
    coverage: Coverage,
}

/// The chosen episodes with what applies to each, and the coverage of the whole library and of
/// each show. Counting needs every episode, so the filters apply here rather than in the query.
pub(super) async fn analysis_results(
    db: &Database,
    filter: AnalysisFilter,
) -> anyhow::Result<Value> {
    db.read("segments.analysis_results", move |db| {
        let mut found = HashMap::<(String, String), Vec<Segment>>::new();
        for row in db
            .prepare("SELECT x.media_id,x.file_id,x.id,x.kind,x.start,x.end,x.source,x.confidence FROM media_segments x JOIN media_files f ON f.id=x.file_id AND f.generation=x.generation AND f.present=1 ORDER BY CASE x.source WHEN 'manual' THEN 0 WHEN 'theintrodb' THEN 1 ELSE 2 END,x.start")?
            .query_map([], |r| {
                Ok((
                    (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                    Segment {
                        id: r.get(2)?,
                        kind: r.get(3)?,
                        start: r.get(4)?,
                        end: r.get(5)?,
                        source: r.get(6)?,
                        confidence: r.get(7)?,
                    },
                ))
            })?
        {
            let (file, segment) = row?;
            found.entry(file).or_default().push(segment);
        }

        let order = if filter.show.is_some() { IN_EPISODE_ORDER } else { LATEST_FIRST };
        let limit = filter.limit.map_or(ANALYSIS_PAGE, |limit| limit as usize);
        let mut items = Vec::new();
        let mut totals = Coverage::default();
        let mut shows = HashMap::<String, ShowCoverage>::new();
        for row in db
            .prepare(&format!("{ANALYZED_EPISODES} {order}"))?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, bool>(8)?,
                ))
            })?
        {
            let (media, file, title, show, show_title, state, error, analyzed_at, manual) = row?;
            let segments = applicable(
                found.remove(&(media.clone(), file.clone())).unwrap_or_default(),
                manual,
            );
            let status = match state.as_deref() {
                Some("running") => "running",
                Some("failed") => "failed",
                Some("complete") if segments.is_empty() => "nothing",
                Some("complete") => "found",
                _ => "waiting",
            };
            totals.count(status, &segments);
            shows
                .entry(show.clone())
                .or_insert_with(|| ShowCoverage {
                    id: show.clone(),
                    title: show_title,
                    coverage: Coverage::default(),
                })
                .coverage
                .count(status, &segments);
            let chosen = filter.show.as_deref().is_none_or(|chosen| chosen == show)
                && match filter.status.as_deref() {
                    None => true,
                    Some("waiting") => matches!(status, "waiting" | "running"),
                    Some(chosen) => chosen == status,
                };
            if chosen && items.len() < limit {
                items.push(json!({
                    "media_id":media,"file_id":file,"title":title,"status":status,"error":error,
                    "analyzed_at":analyzed_at,"segments":segments,
                }));
            }
        }
        let mut shows = shows.into_values().collect::<Vec<_>>();
        shows.sort_by(|a, b| {
            b.coverage.episodes.cmp(&a.coverage.episodes).then_with(|| a.title.cmp(&b.title))
        });
        Ok(json!({"items":items,"shows":shows,"totals":totals}))
    })
    .await
}

pub(super) fn insert(
    db: &rusqlite::Connection,
    src: &thelxinoe_playback::Source,
    s: &Segment,
    origin: &'static str,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO media_segments VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id(),
            src.media_id,
            src.id,
            src.generation,
            s.kind,
            s.start,
            s.end,
            origin,
            if origin == "manual" {
                1.0
            } else {
                s.confidence
            },
            now()
        ],
    )?;
    Ok(())
}
