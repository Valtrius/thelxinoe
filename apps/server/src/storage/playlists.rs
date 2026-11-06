//! Database operations for playlists.
use super::*;
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::collections::{HashMap, HashSet};
use thelxinoe_database::Database;

/// A refusal is decided inside the transaction that would have made the change.
type Decided<T> = std::result::Result<T, Refusal>;

/// Who else may do what is the owner's choice; to everyone else a private playlist is missing.
struct Rights {
    owner: bool,
    add: bool,
    edit: bool,
}
fn rights_of(owner: bool, access: &str) -> Option<Rights> {
    if !owner && access == "private" {
        return None;
    }
    Some(Rights {
        owner,
        add: owner || matches!(access, "add" | "edit"),
        edit: owner || access == "edit",
    })
}
fn rights(db: &Connection, key: &str, user: &str) -> rusqlite::Result<Option<Rights>> {
    Ok(db
        .query_row(
            "SELECT owner_id,access FROM playlists WHERE id=?1",
            [key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
        .and_then(|(owner, access)| rights_of(owner == user, &access)))
}

/// Records an edit, and returns the playlist's new revision.
fn touch(tx: &Transaction<'_>, key: &str) -> rusqlite::Result<i64> {
    tx.query_row(
        "UPDATE playlists SET revision=revision+1,updated_at=?2 WHERE id=?1 RETURNING revision",
        params![key, now()],
        |r| r.get(0),
    )
}

/// Names are unique per owner, ignoring case. Duplicates older than this rule stay until
/// someone renames them.
fn name_taken(
    tx: &Transaction<'_>,
    owner: &str,
    name: &str,
    except: Option<&str>,
) -> rusqlite::Result<bool> {
    let name = name.trim().to_lowercase();
    Ok(tx
        .prepare("SELECT id,name FROM playlists WHERE owner_id=?1")?
        .query_map([owner], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|(id, other)| Some(id.as_str()) != except && other.trim().to_lowercase() == name))
}

fn playlist_row(user: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let owner_id = r.get::<_, String>(3)?;
    let access = r.get::<_, String>(8)?;
    let rights = rights_of(owner_id == user, &access);
    let (add, edit) = rights.map_or((false, false), |rights| (rights.add, rights.edit));
    Ok(json!({
        "id":r.get::<_,String>(0)?,
        "name":r.get::<_,String>(1)?,
        "description":r.get::<_,String>(2)?,
        "owner_id":owner_id,
        "owner":r.get::<_,String>(4)?,
        "revision":r.get::<_,i64>(5)?,
        "favorite":r.get::<_,bool>(6)?,
        "count":r.get::<_,i64>(7)?,
        "access":access,
        "sort_mode":r.get::<_,String>(9)?,
        "sort_direction":r.get::<_,String>(10)?,
        "is_owner":owner_id == user,
        "can_add":add,
        "can_edit":edit,
    }))
}
const PLAYLIST: &str = "SELECT p.id,p.name,p.description,p.owner_id,u.username,p.revision,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.user_id=?1 AND f.playlist_id=p.id),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id),p.access,p.sort_mode,p.sort_direction,EXISTS(SELECT 1 FROM playlist_items i WHERE i.playlist_id=p.id AND i.media_id=?2) FROM playlists p JOIN users u ON u.id=p.owner_id WHERE (p.access<>'private' OR p.owner_id=?1)";

pub(super) async fn list(
    db: &Database,
    p: Principal,
    track: Option<String>,
) -> anyhow::Result<Vec<Value>> {
    db.read("playlists.list", move |db| {
        let user = p.user.id;
        Ok(db
            .prepare(&format!("{PLAYLIST} ORDER BY p.name COLLATE NOCASE,p.id"))?
            .query_map(params![user, track], |r| {
                let mut row = playlist_row(&user, r)?;
                if track.is_some() {
                    row["contains"] = json!(r.get::<_, bool>(11)?);
                }
                Ok(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    })
    .await
}

pub(super) async fn detail(
    db: &Database,
    id: String,
    user: String,
) -> anyhow::Result<Option<Value>> {
    db.read("playlists.detail", move |db| {
        Ok(db
            .query_row(
                &format!("{PLAYLIST} AND p.id=?3"),
                params![user, Option::<String>::None, id],
                |r| playlist_row(&user, r),
            )
            .optional()?)
    })
    .await
}

/// Adds the tracks not there yet, after the others. The caller checked they are tracks.
fn insert_items(tx: &Transaction<'_>, key: &str, items: &[String]) -> rusqlite::Result<i64> {
    let added_at = now();
    let mut position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position),-1)+1 FROM playlist_items WHERE playlist_id=?1",
        [key],
        |r| r.get(0),
    )?;
    let mut insert = tx.prepare(
        "INSERT OR IGNORE INTO playlist_items(playlist_id,media_id,position,added_at) VALUES (?1,?2,?3,?4)",
    )?;
    let mut added = 0;
    for media in items {
        if insert.execute(params![key, media, position, added_at])? > 0 {
            position += 1;
            added += 1;
        }
    }
    Ok(added)
}

pub(super) async fn save(
    creating: bool,
    key: String,
    db: &Database,
    p: Principal,
    input: Save,
) -> anyhow::Result<Decided<()>> {
    db.write("playlists.save", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let name = input.name.trim();
        if creating {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM playlists WHERE owner_id=?1",
                [&p.user.id],
                |r| r.get(0),
            )?;
            if count >= 100 {
                return Ok(Err(Refusal::Invalid));
            }
            if name_taken(&tx, &p.user.id, name, None)? {
                return Ok(Err(Refusal::NameTaken));
            }
            tx.execute(
                "INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
                params![key, p.user.id, name, now()],
            )?;
        } else {
            let current: Option<(String, i64, String)> = tx
                .query_row(
                    "SELECT owner_id,revision,name FROM playlists WHERE id=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((owner, revision, previous)) = current else {
                return Ok(Err(Refusal::Missing));
            };
            if owner != p.user.id {
                return Ok(Err(if rights(&tx, &key, &p.user.id)?.is_some() {
                    Refusal::Forbidden
                } else {
                    Refusal::Missing
                }));
            }
            if revision != input.revision {
                return Ok(Err(Refusal::Stale));
            }
            if previous != name && name_taken(&tx, &p.user.id, name, Some(&key))? {
                return Ok(Err(Refusal::NameTaken));
            }
        }
        if !validate_tracks(&tx, &input.items)? {
            return Ok(Err(Refusal::Invalid));
        }
        tx.execute(
            "UPDATE playlists SET name=?1,description=?2,revision=revision+?3,updated_at=?4 WHERE id=?5",
            params![name, input.description, if creating { 0 } else { 1 }, now(), key],
        )?;
        // Tracks that stay keep when they were added; the position comes from this order.
        let added: HashMap<String, i64> = tx
            .prepare("SELECT media_id,added_at FROM playlist_items WHERE playlist_id=?1")?
            .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id=?1", [&key])?;
        let mut seen = HashSet::new();
        let items = input.items.iter().filter(|media| seen.insert(*media));
        for (position, media) in items.enumerate() {
            tx.execute(
                "INSERT INTO playlist_items(playlist_id,media_id,position,added_at) VALUES (?1,?2,?3,?4)",
                params![
                    key,
                    media,
                    position as i64,
                    added.get(media).copied().unwrap_or_else(now)
                ],
            )?;
        }
        tx.commit()?;
        Ok(Ok(()))
    })
    .await
}

pub(super) async fn change(
    db: &Database,
    key: String,
    user: String,
    input: Change,
) -> anyhow::Result<Decided<i64>> {
    db.write("playlists.change", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(rights) = rights(&tx, &key, &user)? else {
            return Ok(Err(Refusal::Missing));
        };
        let settings = input.name.is_some() || input.access.is_some();
        let sort = input.sort_mode.is_some() || input.sort_direction.is_some();
        if (settings && !rights.owner) || (sort && !rights.edit) {
            return Ok(Err(Refusal::Forbidden));
        }
        if let Some(name) = &input.name {
            let previous: String =
                tx.query_row("SELECT name FROM playlists WHERE id=?1", [&key], |r| {
                    r.get(0)
                })?;
            let name = name.trim();
            if previous != name && name_taken(&tx, &user, name, Some(&key))? {
                return Ok(Err(Refusal::NameTaken));
            }
        }
        tx.execute(
            "UPDATE playlists SET name=COALESCE(?2,name),access=COALESCE(?3,access),sort_mode=COALESCE(?4,sort_mode),sort_direction=COALESCE(?5,sort_direction) WHERE id=?1",
            params![
                key,
                input.name.as_deref().map(str::trim),
                input.access,
                input.sort_mode,
                input.sort_direction
            ],
        )?;
        let revision = touch(&tx, &key)?;
        tx.commit()?;
        Ok(Ok(revision))
    })
    .await
}

pub(super) async fn add_items(
    db: &Database,
    key: String,
    user: String,
    items: Vec<String>,
) -> anyhow::Result<Decided<(i64, i64)>> {
    db.write("playlists.add_items", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(rights) = rights(&tx, &key, &user)? else {
            return Ok(Err(Refusal::Missing));
        };
        if !rights.add {
            return Ok(Err(Refusal::Forbidden));
        }
        if !validate_tracks(&tx, &items)? {
            return Ok(Err(Refusal::Invalid));
        }
        let added = insert_items(&tx, &key, &items)?;
        let revision = if added > 0 {
            touch(&tx, &key)?
        } else {
            tx.query_row("SELECT revision FROM playlists WHERE id=?1", [&key], |r| {
                r.get(0)
            })?
        };
        tx.commit()?;
        Ok(Ok((added, revision)))
    })
    .await
}

pub(super) async fn remove_item(
    db: &Database,
    key: String,
    user: String,
    media: String,
) -> anyhow::Result<Decided<i64>> {
    db.write("playlists.remove_item", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(rights) = rights(&tx, &key, &user)? else {
            return Ok(Err(Refusal::Missing));
        };
        if !rights.edit {
            return Ok(Err(Refusal::Forbidden));
        }
        tx.execute(
            "DELETE FROM playlist_items WHERE playlist_id=?1 AND media_id=?2",
            params![key, media],
        )?;
        let revision = touch(&tx, &key)?;
        tx.commit()?;
        Ok(Ok(revision))
    })
    .await
}

pub(super) async fn move_item(
    db: &Database,
    key: String,
    user: String,
    media: String,
    before: Option<String>,
) -> anyhow::Result<Decided<i64>> {
    db.write("playlists.move_item", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(rights) = rights(&tx, &key, &user)? else {
            return Ok(Err(Refusal::Missing));
        };
        if !rights.edit {
            return Ok(Err(Refusal::Forbidden));
        }
        let position = |media: &str| {
            tx.query_row(
                "SELECT position FROM playlist_items WHERE playlist_id=?1 AND media_id=?2",
                params![key, media],
                |r| r.get::<_, i64>(0),
            )
            .optional()
        };
        let Some(from) = position(&media)? else {
            return Ok(Err(Refusal::Missing));
        };
        // A track removed by someone else meanwhile leaves the end as the target.
        let target = match before.as_deref().map(position).transpose()?.flatten() {
            Some(target) => target,
            None => tx.query_row(
                "SELECT MAX(position)+1 FROM playlist_items WHERE playlist_id=?1",
                [&key],
                |r| r.get(0),
            )?,
        };
        let to = if target > from { target - 1 } else { target };
        if to < from {
            tx.execute(
                "UPDATE playlist_items SET position=position+1 WHERE playlist_id=?1 AND position>=?2 AND position<?3",
                params![key, to, from],
            )?;
        } else {
            tx.execute(
                "UPDATE playlist_items SET position=position-1 WHERE playlist_id=?1 AND position>?2 AND position<=?3",
                params![key, from, to],
            )?;
        }
        tx.execute(
            "UPDATE playlist_items SET position=?3 WHERE playlist_id=?1 AND media_id=?2",
            params![key, media, to],
        )?;
        let revision = touch(&tx, &key)?;
        tx.commit()?;
        Ok(Ok(revision))
    })
    .await
}

pub(super) async fn remove(
    db: &Database,
    p: Principal,
    key: String,
) -> anyhow::Result<Decided<()>> {
    db.write("playlists.remove", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(rights) = rights(&tx, &key, &p.user.id)? else {
            return Ok(Err(Refusal::Missing));
        };
        if !rights.owner {
            return Ok(Err(Refusal::Forbidden));
        }
        tx.execute("DELETE FROM playlists WHERE id=?1", [&key])?;
        tx.execute(
            "INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'playlist.delete',?2,?3)",
            params![p.user.id, key, now()],
        )?;
        tx.commit()?;
        Ok(Ok(()))
    })
    .await
}

pub(super) async fn favorite_for(
    user: String,
    key: String,
    db: &Database,
    favorite: bool,
) -> anyhow::Result<bool> {
    db.write("playlists.favorite_for", move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if rights(&tx, &key, &user)?.is_none() {
            return Ok(false);
        }
        if favorite {
            tx.execute(
                "INSERT OR IGNORE INTO playlist_favorites VALUES (?1,?2)",
                params![user, key],
            )?;
        } else {
            tx.execute(
                "DELETE FROM playlist_favorites WHERE user_id=?1 AND playlist_id=?2",
                params![user, key],
            )?;
        }
        tx.commit()?;
        Ok(true)
    })
    .await
}
