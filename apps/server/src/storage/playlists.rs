//! Database operations for playlists.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn list(db: &Database, p: Principal) -> anyhow::Result<Vec<Value>> {
    db.read("playlists.list", move|db|{
        Ok(db.prepare("SELECT p.id,p.name,p.description,p.owner_id,u.username,p.revision,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.user_id=?1 AND f.playlist_id=p.id),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p JOIN users u ON u.id=p.owner_id ORDER BY p.updated_at DESC,p.id LIMIT 500")?.query_map([p.user.id],playlist_row)?.collect::<std::result::Result<Vec<_>,_>>()?)
    }).await
}

pub(super) async fn detail(
    db: &Database,
    id: String,
    p: Principal,
) -> anyhow::Result<Option<Value>> {
    db.read("playlists.detail", move|db|{
        let playlist=db.query_row("SELECT p.id,p.name,p.description,p.owner_id,u.username,p.revision,EXISTS(SELECT 1 FROM playlist_favorites f WHERE f.user_id=?1 AND f.playlist_id=p.id),(SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id=p.id) FROM playlists p JOIN users u ON u.id=p.owner_id WHERE p.id=?2",params![p.user.id,id],playlist_row).optional()?;
        let Some(mut playlist)=playlist else{return Ok(None);};
        let items=db.prepare("SELECT c.id,c.kind,c.title,c.available FROM playlist_items i JOIN media_cards c ON c.id=i.media_id WHERE i.playlist_id=?1 ORDER BY i.position")?.query_map([id],card)?.collect::<std::result::Result<Vec<_>,_>>()?;
        playlist["items"]=json!(items);Ok(Some(playlist))
    }).await
}

pub(super) async fn save(
    creating: bool,
    key: String,
    db: &Database,
    p: Principal,
    input: Save,
) -> anyhow::Result<i32> {
    db.write("playlists.save", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if creating {
            let count:i64=tx.query_row("SELECT COUNT(*) FROM playlists WHERE owner_id=?1",[&p.user.id],|r|r.get(0))?;
            if count>=100{return Ok(400);}
            tx.execute("INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",params![key,p.user.id,input.name.trim(),now()])?;
        } else {
            let owner:Option<(String,i64)>=tx.query_row("SELECT owner_id,revision FROM playlists WHERE id=?1",[&key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let Some((owner,revision))=owner else{return Ok(404);};
            if owner!=p.user.id{return Ok(403);}
            if revision!=input.revision{return Ok(409);}
        }
        if !validate_tracks(&tx,&input.items)?{return Ok(400);}
        tx.execute("UPDATE playlists SET name=?1,description=?2,revision=revision+?3,updated_at=?4 WHERE id=?5",params![input.name.trim(),input.description,if creating{0}else{1},now(),key])?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id=?1",[&key])?;
        for (position,media) in input.items.iter().enumerate(){tx.execute("INSERT INTO playlist_items VALUES (?1,?2,?3)",params![key,position as i64,media])?;}
        tx.commit()?;Ok(200)
    }).await
}

pub(super) async fn remove(db: &Database, p: Principal, key: String) -> anyhow::Result<i32> {
    db.write("playlists.remove", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let owner:Option<String>=tx.query_row("SELECT owner_id FROM playlists WHERE id=?1",[&key],|r|r.get(0)).optional()?;
        let Some(owner)=owner else{return Ok(404);};
        if owner!=p.user.id{return Ok(403);}
        tx.execute("DELETE FROM playlists WHERE id=?1",[&key])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'playlist.delete',?2,?3)",params![p.user.id,key,now()])?;
        tx.commit()?;Ok(200)
    }).await
}

pub(super) async fn favorite_for(
    user: String,
    key: String,
    db: &Database,
    favorite: bool,
) -> anyhow::Result<bool> {
    db.write("playlists.favorite_for", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
            [&key],
            |r| r.get::<_, bool>(0),
        )? {
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

pub(super) fn playlist_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,String>(2)?,"owner_id":r.get::<_,String>(3)?,"owner":r.get::<_,String>(4)?,"revision":r.get::<_,i64>(5)?,"favorite":r.get::<_,bool>(6)?,"count":r.get::<_,i64>(7)?}),
    )
}
