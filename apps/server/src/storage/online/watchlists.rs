//! Database operations for online.watchlists.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn authorize(user: String, db: &Database, id: i64) -> anyhow::Result<bool> {
    db.read("online.watchlists.authorize", move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM youtube_watchlists WHERE user_id=?1 AND id=?2)",
            params![user, id],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

pub(super) async fn list(db: &Database, p: Principal, grant: String) -> anyhow::Result<Vec<Value>> {
    db.write("online.watchlists.list", move|db|{
        default_list(db,&p.user.id)?;
        let mut lists=db.prepare("SELECT id,name,is_default,auto_download,auto_remove_watched,sort_mode,sort_direction,created_at,updated_at FROM youtube_watchlists WHERE user_id=?1 ORDER BY is_default DESC,id")?.query_map([&p.user.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"isDefault":r.get::<_,bool>(2)?,"autoDownload":r.get::<_,bool>(3)?,"autoRemoveWatched":r.get::<_,bool>(4)?,"sortMode":r.get::<_,String>(5)?,"sortDirection":r.get::<_,String>(6)?,"createdAt":browse::date(r.get(7)?),"updatedAt":browse::date(r.get(8)?)})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for list in &mut lists {
            let entries=db.prepare("SELECT video_id,added_at,manual_position FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2 ORDER BY manual_position,video_id")?.query_map(params![p.user.id,list["id"].as_i64()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            list["items"]=json!(entries.into_iter().map(|(id,added,position)|{let video=browse::one(db,&p.user.id,&id,&grant)?;Ok(json!({"metadataPending":video["metadataPending"],"video":video,"addedAt":browse::date(added),"manualPosition":position}))}).collect::<anyhow::Result<Vec<Value>>>()?);
        }
        Ok(lists)
    }).await
}

pub(super) async fn create_for(
    user: String,
    db: &Database,
    input: Create,
) -> anyhow::Result<Option<i64>> {
    db.write("online.watchlists.create_for", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        default_list(&tx,&user)?;
        if tx.query_row("SELECT COUNT(*) FROM youtube_watchlists WHERE user_id=?1",[&user],|r|r.get::<_,i64>(0))?>=50{return Ok(None);}
        tx.execute("INSERT INTO youtube_watchlists(user_id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)",params![user,input.name.trim(),now()])?;
        let id=tx.last_insert_rowid();changed(&tx, &user)?;tx.commit()?;Ok(Some(id))
    }).await
}

pub(super) async fn update(
    db: &Database,
    id: i64,
    input: Update,
    user: String,
) -> anyhow::Result<()> {
    db.write("online.watchlists.update", move|db|{let tx=db.transaction()?;tx.execute("UPDATE youtube_watchlists SET name=?1,auto_download=?2,auto_remove_watched=?3,sort_mode=?4,sort_direction=?5,updated_at=?6 WHERE user_id=?7 AND id=?8",params![input.name.trim(),input.auto_download,input.auto_remove_watched,input.sort_mode,input.sort_direction,now(),user,id])?;changed(&tx,&user)?;tx.commit()?;Ok(())}).await
}

pub(super) async fn delete(db: &Database, id: i64, user: String) -> anyhow::Result<bool> {
    db.write("online.watchlists.delete", move |db| {
        let tx = db.transaction()?;
        let removed = tx.execute(
            "DELETE FROM youtube_watchlists WHERE user_id=?1 AND id=?2 AND is_default=0",
            params![user, id],
        )? > 0;
        if removed {
            changed(&tx, &user)?;
        }
        tx.commit()?;
        Ok(removed)
    })
    .await
}

pub(super) async fn add_for_write_youtube_watchlists(
    video: String,
    grant: String,
    user: String,
    db: &Database,
    id: i64,
    input: Add,
) -> anyhow::Result<Option<(bool, Value)>> {
    db.write("online.watchlists.add_for_write_youtube_watchlists", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Check again within the write transaction in case the list was deleted concurrently.
        let exists=tx.query_row("SELECT id FROM youtube_watchlists WHERE user_id=?1 AND id=?2",params![user,id],|r|r.get::<_,i64>(0)).optional()?;
        if exists.is_none(){return Ok(None);}
        let retained=tx.query_row("SELECT COUNT(*) FROM youtube_video_state WHERE user_id=?1 AND video_id<>?2 AND (watchlist=1 OR pinned=1)",params![user,video],|r|r.get::<_,i64>(0))?;
        if retained>=1000{return Ok(None);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,?2,?2) ON CONFLICT DO NOTHING",params![user,video])?;
        tx.execute("INSERT INTO youtube_state(user_id,video_id,updated_at) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",params![user,video,now()])?;
        let added=tx.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![user,id,video,input.manual_position,now()])?>0;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),user])?;
        let output=browse::one(&tx,&user,&video,&grant)?;changed(&tx, &user)?;tx.commit()?;Ok(Some((added,output)))
    }).await
}

pub(super) async fn add_for_read_youtube_watchlists(
    db: &Database,
    id: i64,
) -> anyhow::Result<bool> {
    db.read(
        "online.watchlists.add_for_read_youtube_watchlists",
        move |db| {
            Ok(db
                .query_row(
                    "SELECT auto_download FROM youtube_watchlists WHERE id=?1",
                    [id],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false))
        },
    )
    .await
}

pub(super) async fn remove_for(
    user: String,
    db: &Database,
    id: i64,
    video: String,
) -> anyhow::Result<()> {
    db.write("online.watchlists.remove_for", move|db|{let tx=db.transaction()?;tx.execute("DELETE FROM youtube_watchlist_items WHERE user_id=?1 AND watchlist_id=?2 AND video_id=?3",params![user,id,video])?;changed(&tx,&user)?;tx.commit()?;Ok(())}).await
}

pub(super) async fn reorder(
    db: &Database,
    id: i64,
    input: Reorder,
    user: String,
) -> anyhow::Result<()> {
    db.write("online.watchlists.reorder", move|db|{let tx=db.transaction()?;for (index,video) in input.video_ids.iter().enumerate(){tx.execute("UPDATE youtube_watchlist_items SET manual_position=?1 WHERE user_id=?2 AND watchlist_id=?3 AND video_id=?4",params![index as i64,user,id,video])?;}changed(&tx,&user)?;tx.commit()?;Ok(())}).await
}

pub(crate) fn default_list(db: &Connection, user: &str) -> anyhow::Result<i64> {
    db.execute("INSERT INTO youtube_watchlists(user_id,name,is_default,created_at,updated_at) SELECT ?1,'Watch Later',1,?2,?2 WHERE NOT EXISTS(SELECT 1 FROM youtube_watchlists WHERE user_id=?1 AND is_default=1)",params![user,now()])?;
    Ok(db.query_row(
        "SELECT id FROM youtube_watchlists WHERE user_id=?1 AND is_default=1",
        [user],
        |r| r.get(0),
    )?)
}

fn changed(tx: &Connection, user: &str) -> anyhow::Result<()> {
    crate::storage::record_event(tx, Some(user), "youtube.changed", &json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_watchlist_and_its_event_commit_together() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let db = Database::open(temp.path().join("db"))?;
        db.write("test.seed", |c| {
            c.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('alice','alice','unused','user',1)", [])?;
            Ok(())
        }).await?;
        create_for(
            "alice".into(),
            &db,
            Create {
                name: "Saved".into(),
            },
        )
        .await?;
        db.write("test.fail_event", |c| {
            c.execute_batch("CREATE TEMP TRIGGER reject_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'test event failure'); END;")?;
            Ok(())
        }).await?;
        assert!(
            create_for(
                "alice".into(),
                &db,
                Create {
                    name: "Rolled back".into()
                }
            )
            .await
            .is_err()
        );
        let counts = db.read("test.counts", |c| Ok(c.query_row(
            "SELECT (SELECT COUNT(*) FROM youtube_watchlists WHERE is_default=0),(SELECT COUNT(*) FROM events WHERE kind='youtube.changed')",
            [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?)).await?;
        assert_eq!(counts, (1, 1));
        db.shutdown().await?;
        Ok(())
    }
}
