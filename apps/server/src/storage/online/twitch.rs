//! Database operations for online.twitch.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn configure(
    db: &Database,
    p: thelxinoe_core::Principal,
    changed: bool,
    encrypted: Vec<u8>,
) -> anyhow::Result<()> {
    db.write("online.twitch.configure", move|db|{let tx=db.transaction()?;
        tx.execute("INSERT INTO secrets VALUES ('provider.twitch_client_id',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;
        if changed {
            tx.execute("UPDATE online_accounts SET credential=NULL,status=CASE WHEN status='disconnected' THEN status ELSE 'reconnect_required' END,generation=?1,expires_at=0 WHERE provider='twitch'",[id()])?;
            tx.execute("DELETE FROM twitch_attempts",[])?;tx.execute("DELETE FROM twitch_sync",[])?;
        }
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','twitch',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(())
    }).await
}

pub(super) async fn account(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> std::prelude::v1::Result<
    (
        Value,
        Option<(String, Vec<u8>, i64, Option<String>)>,
        Option<Value>,
    ),
    anyhow::Error,
> {
    db.read("online.twitch.account", move|db|{
        let account=db.query_row("SELECT status,display_name,avatar_url FROM online_accounts WHERE user_id=?1 AND provider='twitch'",[&p.user.id],|r|Ok(json!({"status":r.get::<_,String>(0)?,"display_name":r.get::<_,String>(1)?,"avatar_url":r.get::<_,Option<String>>(2)?}))).optional()?.unwrap_or(json!({"status":"disconnected","display_name":"","avatar_url":null}));
        let attempt=db.query_row("SELECT generation,device,expires_at,error FROM twitch_attempts WHERE user_id=?1 AND session_id=?2 AND expires_at>?3",params![p.user.id,p.session_id,now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<String>>(3)?))).optional()?;
        let sync=db.query_row("SELECT last_complete,next_run,error FROM twitch_sync WHERE user_id=?1",[p.user.id],|r|Ok(json!({"last_complete":r.get::<_,Option<i64>>(0)?,"next_run":r.get::<_,i64>(1)?,"error":r.get::<_,Option<String>>(2)?}))).optional()?;
        Ok((account,attempt,sync))
    }).await
}

pub(super) async fn start_write_online_accounts(
    db: &Database,
    user: String,
    gen_copy: String,
) -> anyhow::Result<bool> {
    db.write("online.twitch.start_write_online_accounts", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let recent=tx.query_row("SELECT updated_at>?1-30 FROM online_accounts WHERE user_id=?2 AND provider='twitch'",params![now(),user],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        if recent{return Ok(false)}
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES (?1,'twitch',?2,?3) ON CONFLICT(user_id,provider) DO UPDATE SET generation=excluded.generation,updated_at=excluded.updated_at",params![user,gen_copy,now()])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1",[&user])?;
        tx.execute("UPDATE twitch_sync SET generation=?1 WHERE user_id=?2",params![gen_copy,user])?;
        tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn start_write_twitch_attempts(
    db: &Database,
    p: thelxinoe_core::Principal,
    generation: String,
    expires: i64,
    interval: i64,
    encrypted: Vec<u8>,
    hash: String,
) -> anyhow::Result<bool> {
    db.write("online.twitch.start_write_twitch_attempts", move|db|Ok(db.execute("INSERT INTO twitch_attempts(user_id,session_id,generation,client_hash,device,expires_at,next_poll,interval) SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='twitch' AND generation=?3) AND EXISTS(SELECT 1 FROM sessions WHERE id=?2 AND user_id=?1 AND expires_at>?9)",params![p.user.id,p.session_id,generation,hash,encrypted,expires,now()+interval,interval,now()])?==1)).await
}

pub(super) async fn clear(
    p: thelxinoe_core::Principal,
    db: &Database,
    delete: bool,
) -> anyhow::Result<Vec<String>> {
    db.write("online.twitch.clear", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE online_accounts SET credential=NULL,status='disconnected',expires_at=0,generation=?1,updated_at=?2,avatar_url=NULL,profile_checked_at=0 WHERE user_id=?3 AND provider='twitch'",params![id(),now(),p.user.id])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1",[&p.user.id])?;tx.execute("DELETE FROM twitch_sync WHERE user_id=?1",[&p.user.id])?;
        let stopped=if delete {
            let ids=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1 AND live_media_id LIKE 'twitch:%' AND state IN ('ready','playing','paused')")?.query_map([&p.user.id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for key in &ids {tx.execute("UPDATE playback_sessions SET state='stopped' WHERE id=?1",[key])?;tx.execute("DELETE FROM playback_grants WHERE resource=?1",[format!("playback:{key}")])?;}
            crate::statistics::delete_provider(&tx,&p.user.id,"twitch")?;
            crate::history::delete_provider(&tx,&p.user.id,"twitch")?;
            ids
        }else{vec![]};
        if delete {tx.execute("DELETE FROM twitch_streams WHERE user_id=?1",[&p.user.id])?;tx.execute("DELETE FROM online_accounts WHERE user_id=?1 AND provider='twitch'",[&p.user.id])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,'twitch',?3)",params![p.user.id,if delete{"online.delete_data"}else{"online.disconnect"},now()])?;
        tx.commit()?;Ok(stopped)
    }).await
}

pub(super) async fn feed(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Vec<Value>> {
    db.read("online.twitch.feed", move|db|Ok(db.prepare("SELECT channel_id,login,display_name,title,category,viewers,started_at,thumbnail_url,profile_image_url FROM twitch_streams WHERE user_id=?1 AND active=1 ORDER BY viewers DESC,login LIMIT 1000")?.query_map([p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"login":r.get::<_,String>(1)?,"display_name":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"category":r.get::<_,String>(4)?,"viewers":r.get::<_,i64>(5)?,"started_at":r.get::<_,String>(6)?,"thumbnail_url":r.get::<_,Option<String>>(7)?,"profile_image_url":r.get::<_,Option<String>>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await
}

pub(super) async fn claim_attempt(db: &Database) -> anyhow::Result<Option<Attempt>> {
    db.write("online.twitch.claim_attempt", |db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM twitch_attempts WHERE expires_at<=?1 OR NOT EXISTS(SELECT 1 FROM sessions s WHERE s.id=twitch_attempts.session_id AND s.expires_at>?1)",[now()])?;
        let a=tx.query_row("SELECT user_id,session_id,generation,client_hash,device,interval FROM twitch_attempts WHERE next_poll<=?1 ORDER BY next_poll LIMIT 1",[now()],|r|Ok(Attempt{user:r.get(0)?,session:r.get(1)?,generation:r.get(2)?,hash:r.get(3)?,device:r.get(4)?,interval:r.get(5)?})).optional()?;
        if let Some(a)=&a {tx.execute("UPDATE twitch_attempts SET next_poll=?1 WHERE user_id=?2 AND generation=?3",params![now()+90,a.user,a.generation])?;}
        tx.commit()?;Ok(a)
    }).await
}

pub(super) async fn poll_write_twitch_attempts(
    delay: i64,
    terminal: bool,
    user: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.twitch.poll_write_twitch_attempts", move|db|{if terminal{db.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND generation=?2",params![user,generation])?;}else{db.execute("UPDATE twitch_attempts SET next_poll=?1,interval=?2 WHERE user_id=?3 AND generation=?4",params![now()+delay,delay.min(300),user,generation])?;}Ok(())}).await
}

pub(super) async fn complete_authorization(
    v: Validation,
    encrypted: Vec<u8>,
    user: String,
    session: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.write("online.twitch.complete_authorization", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT EXISTS(SELECT 1 FROM twitch_attempts t JOIN sessions s ON s.id=t.session_id JOIN online_accounts a ON a.user_id=t.user_id AND a.provider='twitch' AND a.generation=t.generation WHERE t.user_id=?1 AND t.generation=?2 AND t.session_id=?3 AND s.expires_at>?4 AND t.expires_at>?4)",params![user,generation,session,now()],|r|r.get::<_,bool>(0))?;
        if !valid{return Ok(false)}
        tx.execute("UPDATE online_accounts SET credential=?1,status='connected',external_id=?2,display_name=?3,expires_at=?4,updated_at=?5,avatar_url=NULL,profile_checked_at=0 WHERE user_id=?6 AND provider='twitch' AND generation=?7",params![encrypted,v.user_id,v.login,now()+v.expires_in.min(86400*60),now(),user,generation])?;
        tx.execute("INSERT INTO twitch_sync(user_id,generation,validated_at) VALUES (?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET generation=excluded.generation,validated_at=excluded.validated_at,next_run=0,cursor='',snapshot='',pages=0,error=NULL,failures=0",params![user,generation,now()])?;
        tx.execute("DELETE FROM twitch_attempts WHERE user_id=?1 AND generation=?2",params![user,generation])?;tx.commit()?;Ok(true)
    }).await
}

pub(super) async fn run_write_twitch_sync(db: &Database) -> anyhow::Result<()> {
    db.write("online.twitch.run_write_twitch_sync", |db| {
        db.execute("UPDATE twitch_sync SET validated_at=0", [])?;
        Ok(())
    })
    .await
}

pub(super) async fn run_write_twitch_attempts(
    a: Attempt,
    error: ApiError,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.twitch.run_write_twitch_attempts", move |db| {
        db.execute(
            "UPDATE twitch_attempts SET error=?1,next_poll=?2 WHERE user_id=?3 AND generation=?4",
            params![error.2, now() + 30, a.user, a.generation],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn sync_tick_write_twitch_sync(db: &Database) -> anyhow::Result<Option<Turn>> {
    db.write("online.twitch.sync_tick_write_twitch_sync", |db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let turn=tx.query_row("SELECT t.user_id,t.generation,a.external_id,a.credential,a.expires_at,t.validated_at,t.cursor,t.snapshot,t.pages,t.failures FROM twitch_sync t JOIN online_accounts a ON a.user_id=t.user_id AND a.provider='twitch' AND a.generation=t.generation AND a.status='connected' WHERE t.next_run<=?1 ORDER BY t.last_turn,t.user_id LIMIT 1",[now()],|r|Ok(Turn{user:r.get(0)?,generation:r.get(1)?,external:r.get(2)?,credential:r.get(3)?,expires:r.get(4)?,validated:r.get(5)?,cursor:r.get(6)?,snapshot:r.get(7)?,pages:r.get(8)?,failures:r.get(9)?})).optional()?;
        if let Some(t)=&turn {tx.execute("UPDATE twitch_sync SET next_run=?1,last_turn=(SELECT COALESCE(MAX(last_turn),0)+1 FROM twitch_sync) WHERE user_id=?2",params![now()+90,t.user])?;}
        tx.commit()?;Ok(turn)
    }).await
}

pub(super) async fn sync_tick_write_online_accounts(
    t: Turn,
    error: ApiError,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.twitch.sync_tick_write_online_accounts", move|db|{let tx=db.transaction()?;
        if error.0==axum::http::StatusCode::UNAUTHORIZED {tx.execute("UPDATE online_accounts SET status='reconnect_required',credential=NULL,expires_at=0 WHERE user_id=?1 AND provider='twitch' AND generation=?2",params![t.user,t.generation])?;}
        let delay=30i64.saturating_mul(1i64<<t.failures.min(7)).min(3600);
        tx.execute("UPDATE twitch_sync SET error=?1,failures=failures+1,next_run=MAX(next_run,?2) WHERE user_id=?3 AND generation=?4",params![if error.0==axum::http::StatusCode::UNAUTHORIZED{"Reconnect Twitch to continue synchronization".to_owned()}else{error.2},now()+delay,t.user,t.generation])?;tx.commit()?;Ok(())
    }).await
}

pub(super) async fn sync_step_write_online_accounts(
    encrypted: Vec<u8>,
    user: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.write("online.twitch.sync_step_write_online_accounts", move|db|Ok(db.execute("UPDATE online_accounts SET credential=?1,expires_at=?2 WHERE user_id=?3 AND provider='twitch' AND generation=?4 AND status='connected'",params![encrypted,now()+300,user,generation])?==1)).await
}

pub(super) async fn record_validation(
    v: Validation,
    user: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.twitch.record_validation", move|db|{let tx=db.transaction()?;tx.execute("UPDATE online_accounts SET expires_at=?1 WHERE user_id=?2 AND provider='twitch' AND generation=?3 AND status='connected'",params![now()+v.expires_in.min(86400*60),user,generation])?;tx.execute("UPDATE twitch_sync SET validated_at=?1 WHERE user_id=?2 AND generation=?3",params![now(),user,generation])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn sync_step_write_twitch_sync(
    reset: i64,
    user: String,
    generation: String,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.twitch.sync_step_write_twitch_sync", move |db| {
        db.execute(
            "UPDATE twitch_sync SET next_run=?1 WHERE user_id=?2 AND generation=?3",
            params![reset, user, generation],
        )?;
        Ok(())
    })
    .await
}

pub(super) async fn sync_step_read_online_accounts(
    owner: String,
    db: &Database,
) -> anyhow::Result<bool> {
    db.read("online.twitch.sync_step_read_online_accounts", move |db| {
        Ok(db.query_row("SELECT profile_checked_at<?1 FROM online_accounts WHERE user_id=?2 AND provider='twitch'", params![now()-86400,owner], |r|r.get::<_,bool>(0)).optional()?.unwrap_or(false))
    }).await
}

pub(super) struct SyncPage {
    pub(super) reset: i64,
    pub(super) limited: bool,
    pub(super) rows: Vec<Value>,
    pub(super) next: String,
    pub(super) profiles: std::collections::HashMap<String, String>,
    pub(super) own_profile: Option<(Option<String>, Option<String>)>,
    pub(super) user: String,
    pub(super) generation: String,
    pub(super) snapshot: String,
}

pub(super) async fn save_sync_page(db: &Database, input: SyncPage) -> anyhow::Result<bool> {
    let SyncPage {
        reset,
        limited,
        rows,
        next,
        profiles,
        own_profile,
        user,
        generation,
        snapshot,
    } = input;

    db.write("online.twitch.save_sync_page", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT EXISTS(SELECT 1 FROM online_accounts WHERE user_id=?1 AND provider='twitch' AND generation=?2 AND status='connected')",params![user,generation],|r|r.get::<_,bool>(0))?;
        if !valid{return Ok(false)}
        let profile_changed = own_profile.is_some();
        if let Some((name, avatar)) = own_profile {
            tx.execute("UPDATE online_accounts SET display_name=COALESCE(?1,display_name),avatar_url=?2,profile_checked_at=?3 WHERE user_id=?4 AND provider='twitch'",params![name,avatar,now(),user])?;
        }
        for r in rows {tx.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,thumbnail_url,profile_image_url) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(user_id,channel_id) DO UPDATE SET login=excluded.login,display_name=excluded.display_name,title=excluded.title,category=excluded.category,viewers=excluded.viewers,started_at=excluded.started_at,snapshot=excluded.snapshot,thumbnail_url=excluded.thumbnail_url,profile_image_url=COALESCE(excluded.profile_image_url,twitch_streams.profile_image_url)",params![user,r["user_id"].as_str(),r["user_login"].as_str(),r["user_name"].as_str(),r["title"].as_str(),r["game_name"].as_str(),r["viewer_count"].as_i64(),r["started_at"].as_str(),snapshot,super::super::public_image(r["thumbnail_url"].as_str()).map(|url|url.replace("{width}","640").replace("{height}","360")),profiles.get(r["user_id"].as_str().unwrap_or(""))])?;}
        if next.is_empty(){
            tx.execute("DELETE FROM twitch_streams WHERE user_id=?1 AND snapshot<>?2",params![user,snapshot])?;
            tx.execute("UPDATE twitch_streams SET active=1 WHERE user_id=?1",[&user])?;
            tx.execute("UPDATE twitch_sync SET cursor='',snapshot='',pages=0,next_run=?1,last_complete=?2,failures=0,error=NULL WHERE user_id=?3 AND generation=?4",params![if limited{reset.max(now()+60)}else{now()+60},now(),user,generation])?;
        }else{tx.execute("UPDATE twitch_sync SET cursor=?1,snapshot=?2,pages=pages+1,next_run=?3,failures=0,error=NULL WHERE user_id=?4 AND generation=?5",params![next,snapshot,if limited{reset}else{now()+1},user,generation])?;}
        tx.commit()?;Ok(profile_changed)
    }).await
}
