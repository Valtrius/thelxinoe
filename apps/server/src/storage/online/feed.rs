//! Database operations for online.feed.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn list(
    db: &Database,
    filter: Filter,
    owner: String,
) -> anyhow::Result<(Vec<Value>, u32, Option<Value>)> {
    db.read("online.feed.list", move|db|{
        let from="FROM youtube_videos v LEFT JOIN youtube_video_state s USING(user_id,video_id) WHERE v.user_id=?1 AND (?2=0 OR s.watchlist=1) AND (?3=0 OR s.pinned=1) AND (?4=0 OR (v.is_short=0 OR v.broadcast<>'none')) AND (?5=0 OR COALESCE(s.watched,0)=0) AND (?6='' OR instr(lower(v.title),lower(?6))>0 OR instr(lower(v.channel_title),lower(?6))>0) AND (?7='' OR v.channel_id=?7) AND (?2=1 OR ?3=1 OR EXISTS(SELECT 1 FROM youtube_subscriptions c WHERE c.user_id=v.user_id AND c.channel_id=v.channel_id AND c.active=1))";
        let query=params![owner,filter.watchlist,filter.pinned,filter.hide_shorts,filter.unwatched,filter.search,filter.channel];
        let total=db.query_row(&format!("SELECT COUNT(*) {from}"),query,|r|r.get::<_,u32>(0))?;
        let sql=format!("SELECT v.video_id,v.title,v.channel_title,v.published_at,v.duration,v.broadcast,v.available,v.is_short,v.metadata_at,COALESCE(s.watchlist,0),COALESCE(s.pinned,0),COALESCE(s.watched,0),COALESCE(s.position,0),v.privacy {from} ORDER BY CASE WHEN ?2=1 THEN s.added_at ELSE v.published_at END DESC,v.video_id LIMIT 50 OFFSET ?8");
        let rows=db.prepare(&sql)?.query_map(params![owner,filter.watchlist,filter.pinned,filter.hide_shorts,filter.unwatched,filter.search,filter.channel,filter.offset],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"channel":r.get::<_,String>(2)?,"published_at":r.get::<_,i64>(3)?,"duration":r.get::<_,Option<i64>>(4)?,"broadcast":r.get::<_,String>(5)?,"available":r.get::<_,bool>(6)?,"is_short":r.get::<_,Option<bool>>(7)?,"pending":r.get::<_,i64>(8)?==0,"watchlist":r.get::<_,bool>(9)?,"pinned":r.get::<_,bool>(10)?,"watched":r.get::<_,bool>(11)?,"position":r.get::<_,f64>(12)?,"privacy":r.get::<_,String>(13)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let sync=db.query_row("SELECT next_run,last_complete,error,cursor FROM youtube_sync WHERE user_id=?1",[owner],|r|Ok(json!({"next_run":r.get::<_,i64>(0)?,"last_complete":r.get::<_,Option<i64>>(1)?,"error":r.get::<_,Option<String>>(2)?,"in_progress":r.get::<_,String>(3)?!="{}"}))).optional()?;
        let rows=rows.into_iter().map(|mut row| {
            let status=db.query_row("SELECT state FROM youtube_downloads WHERE video_id=?1",[row["id"].as_str().unwrap()],|r|r.get::<_,String>(0)).optional()?;
            row["download"]=json!(status);Ok(row)
        }).collect::<anyhow::Result<Vec<_>>>()?;
        Ok((rows,total,sync))
    }).await
}

pub(super) async fn add(db: &Database, video: String, user: String) -> anyhow::Result<bool> {
    db.write("online.feed.add", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists=tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_video_state WHERE user_id=?1 AND video_id=?2 AND watchlist=1)",params![user,video],|r|r.get::<_,bool>(0))?;
        let count=tx.query_row("SELECT COUNT(*) FROM youtube_video_state WHERE user_id=?1 AND (watchlist=1 OR pinned=1)",[&user],|r|r.get::<_,u32>(0))?;
        if !exists&&count>=1000{return Ok(false);}
        tx.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES (?1,?2,?2) ON CONFLICT DO NOTHING",params![user,video])?;
        tx.execute("INSERT INTO youtube_state(user_id,video_id,updated_at) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![user,video,now()])?;
        let list=super::super::watchlists::default_list(&tx,&user)?;
        tx.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![user,list,video,-now(),now()])?;
        tx.execute("UPDATE youtube_sync SET next_run=MIN(next_run,?1) WHERE user_id=?2 AND failures=0",params![now(),user])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn edit_for(
    user: String,
    db: &Database,
    video: String,
    input: Edit,
) -> anyhow::Result<i32> {
    db.write("online.feed.edit_for", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",params![user,video],|r|r.get::<_,bool>(0))? {return Ok(0);}
        if input.watchlist==Some(true)||input.pinned==Some(true) {
            let count=tx.query_row("SELECT COUNT(*) FROM youtube_video_state WHERE user_id=?1 AND video_id<>?2 AND (watchlist=1 OR pinned=1)",params![user,video],|r|r.get::<_,u32>(0))?;
            if count>=1000 {return Ok(2);}
        }
        tx.execute("INSERT INTO youtube_state(user_id,video_id,updated_at) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![user,video,now()])?;
        tx.execute("UPDATE youtube_state SET pinned=COALESCE(?1,pinned),watched=COALESCE(?2,watched),updated_at=?3 WHERE user_id=?4 AND video_id=?5",params![input.pinned,input.watched,now(),user,video])?;
        if let Some(watchlist)=input.watchlist {
            if watchlist {
                let list=super::super::watchlists::default_list(&tx,&user)?;
                tx.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![user,list,video,-now(),now()])?;
            } else { tx.execute("DELETE FROM youtube_watchlist_items WHERE user_id=?1 AND video_id=?2",params![user,video])?; }
        }
        tx.commit()?;Ok(1)
    }).await
}

pub(super) async fn delete_data(db: &Database, user: String) -> anyhow::Result<()> {
    db.write("online.feed.delete_data", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        crate::statistics::delete_provider(&tx,&user,"youtube")?;
        crate::history::delete_provider(&tx,&user,"youtube")?;
        for table in ["oauth_attempts","online_accounts"]{tx.execute(&format!("DELETE FROM {table} WHERE user_id=?1 AND provider='youtube'"),[&user])?;}
        tx.execute("UPDATE playback_sessions SET state='stopped',updated_at=?2 WHERE user_id=?1 AND youtube_video_id IS NOT NULL",params![user,now()])?;
        tx.execute("DELETE FROM playback_grants WHERE resource IN (SELECT 'playback:'||id FROM playback_sessions WHERE user_id=?1 AND youtube_video_id IS NOT NULL)",[&user])?;
        for table in ["youtube_watchlists","youtube_sync","youtube_subscriptions","youtube_videos"]{tx.execute(&format!("DELETE FROM {table} WHERE user_id=?1"),[&user])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.delete-data','youtube',?2)",params![user,now()])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn artwork(
    db: &Database,
    p: thelxinoe_core::Principal,
    id: String,
) -> anyhow::Result<bool> {
    db.read("online.feed.artwork", move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2 AND privacy='public')",params![p.user.id,id],|r|r.get::<_,bool>(0))?)).await
}
