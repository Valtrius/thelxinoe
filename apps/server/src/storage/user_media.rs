//! Database operations for user_media.
use super::*;
use crate::watching;
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
) -> anyhow::Result<watching::Changed> {
    db.write("user_media.set_for", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let change = watching::Change {
            watched: input.watched,
            favorite: input.favorite,
            watch_later: input.watch_later,
            resume: None,
        };
        let changed = watching::change(&tx, &user, &mid, change, now())?;
        if matches!(changed, watching::Changed::Done) {
            tx.commit()?;
        }
        Ok(changed)
    })
    .await
}

pub(super) async fn set_watched(
    user: String,
    media: String,
    watched: bool,
    through: bool,
    db: &Database,
) -> anyhow::Result<watching::Marked> {
    db.write("user_media.set_watched", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let scope = if through {
            watching::Scope::Through
        } else {
            watching::Scope::Episodes
        };
        let marked = watching::mark(&tx, &user, &media, watched, scope, now())?;
        tx.commit()?;
        Ok(marked)
    })
    .await
}

pub(super) async fn home_for(p: Principal, db: &Database) -> anyhow::Result<Value> {
    db.read("user_media.home_for", move |db| {
        let favorites = db.prepare("SELECT c.id,c.kind,c.title,c.available,sh.title FROM media_cards c JOIN media_state s ON s.media_id=c.id LEFT JOIN media pm ON pm.id=c.parent_id LEFT JOIN media_cards sh ON sh.id=CASE c.kind WHEN 'season' THEN c.parent_id WHEN 'episode' THEN pm.parent_id END WHERE s.user_id=?1 AND s.favorite=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],shelf_card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let later = db.prepare("SELECT c.id,c.kind,c.title,c.available,sh.title FROM media_cards c JOIN media_state s ON s.media_id=c.id LEFT JOIN media pm ON pm.id=c.parent_id LEFT JOIN media_cards sh ON sh.id=CASE c.kind WHEN 'season' THEN c.parent_id WHEN 'episode' THEN pm.parent_id END WHERE s.user_id=?1 AND s.watch_later=1 ORDER BY s.updated_at DESC,c.title LIMIT 100")?.query_map([&p.user.id],shelf_card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        let resume = watching::continue_watching(db, &p.user.id)?
            .into_iter()
            .map(|e| json!({"id":e.id,"kind":e.kind,"title":e.title,"available":e.available,"position":e.position,"duration":e.duration,"edition":e.edition,"fileId":e.file,"last_played":e.last_played}))
            .collect::<Vec<_>>();
        let next = watching::home_next_up(db, &p.user.id)?
            .into_iter()
            .map(|e| {
                let mut card = json!({"id":e.id,"kind":e.kind,"title":e.title,"available":e.available,"show_title":e.show_title});
                if let Some(duration) = e.duration {
                    card["duration"] = json!(duration);
                }
                card
            })
            .collect::<Vec<_>>();
        Ok(json!({"favorites":favorites,"watch_later":later,"continue_watching":resume,"next_up":next}))
    }).await
}

/// A shelf card; seasons and episodes also name their show.
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
