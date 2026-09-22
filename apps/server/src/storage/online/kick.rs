//! Database operations for online.kick.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn configure(
    db: &Database,
    p: thelxinoe_core::Principal,
    encrypted: Vec<u8>,
) -> anyhow::Result<()> {
    db.write("online.kick.configure", move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO secrets VALUES ('provider.kick',?1) ON CONFLICT(scope) DO UPDATE SET ciphertext=excluded.ciphertext",[encrypted])?;
        tx.execute("UPDATE kick_channels SET generation=?1,next_run=0,failures=0,error=NULL",[id()])?;
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.configure','kick',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn add(
    db: &Database,
    p: thelxinoe_core::Principal,
    slug: String,
) -> anyhow::Result<()> {
    db.write("online.kick.add", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        anyhow::ensure!(tx.query_row("SELECT COUNT(*) FROM kick_channels WHERE user_id=?1",[&p.user.id],|r|r.get::<_,i64>(0))?<100,"Tracked Kick channel limit reached");
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,status,updated_at) VALUES (?1,'kick',?2,'connected',?3) ON CONFLICT(user_id,provider) DO UPDATE SET status='connected',updated_at=excluded.updated_at",params![p.user.id,id(),now()])?;
        tx.execute("INSERT INTO kick_channels(user_id,slug,generation) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![p.user.id,slug,id()])?;tx.commit()?;Ok(())}).await
}

pub(super) async fn feed(
    db: &Database,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<(bool, Vec<Value>)> {
    db.read("online.kick.feed", move|db|{let connected=db.query_row("SELECT status='connected' FROM online_accounts WHERE user_id=?1 AND provider='kick'",[&p.user.id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let items=db.prepare("SELECT slug,title,category,live,viewers,updated_at,next_run,error,thumbnail_url,started_at,display_name,profile_image_url,language,mature,tags FROM kick_channels WHERE user_id=?1 ORDER BY COALESCE(live,0) DESC,viewers DESC,slug")?.query_map([p.user.id],|r|Ok(json!({"slug":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"live":r.get::<_,Option<bool>>(3)?,"viewers":r.get::<_,i64>(4)?,"updated_at":r.get::<_,i64>(5)?,"next_run":r.get::<_,i64>(6)?,"error":r.get::<_,Option<String>>(7)?,"thumbnail_url":r.get::<_,Option<String>>(8)?,"started_at":r.get::<_,Option<String>>(9)?,"display_name":r.get::<_,Option<String>>(10)?,"profile_image_url":r.get::<_,Option<String>>(11)?,"language":r.get::<_,Option<String>>(12)?,"mature":r.get::<_,bool>(13)?,"tags":serde_json::from_str::<Value>(&r.get::<_,String>(14)?).unwrap_or(json!([]))})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok((connected,items))}).await
}

pub(super) async fn connect(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<()> {
    db.write("online.kick.connect", move|db|{db.execute("INSERT INTO online_accounts(user_id,provider,generation,status,updated_at) VALUES (?1,'kick',?2,'connected',?3) ON CONFLICT(user_id,provider) DO UPDATE SET status='connected',generation=excluded.generation,updated_at=excluded.updated_at",params![p.user.id,id(),now()])?;Ok(())}).await
}

pub(super) async fn disconnect(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<()> {
    db.write("online.kick.disconnect", move|db|{db.execute("UPDATE online_accounts SET status='disconnected',generation=?1,updated_at=?2 WHERE user_id=?3 AND provider='kick'",params![id(),now(),p.user.id])?;Ok(())}).await
}

pub(super) async fn erase(
    p: thelxinoe_core::Principal,
    db: &Database,
    channel: Option<String>,
) -> anyhow::Result<Vec<String>> {
    db.write("online.kick.erase", move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let media=channel.as_ref().map(|s|format!("kick:{s}"));
        let ids=tx.prepare("SELECT id FROM playback_sessions WHERE user_id=?1 AND live_media_id LIKE 'kick:%' AND (?2 IS NULL OR live_media_id=?2) AND state IN ('ready','playing','paused')")?.query_map(params![p.user.id,media],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for key in &ids {tx.execute("UPDATE playback_sessions SET state='stopped' WHERE id=?1",[key])?;tx.execute("DELETE FROM playback_grants WHERE resource=?1",[format!("playback:{key}")])?;}
        tx.execute("DELETE FROM kick_channels WHERE user_id=?1 AND (?2 IS NULL OR slug=?2)",params![p.user.id,channel])?;
        if channel.is_none(){crate::statistics::delete_provider(&tx,&p.user.id,"kick")?;crate::history::delete_provider(&tx,&p.user.id,"kick")?;tx.execute("DELETE FROM online_accounts WHERE user_id=?1 AND provider='kick'",[&p.user.id])?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'online.delete_data','kick',?2)",params![p.user.id,now()])?;tx.commit()?;Ok(ids)}).await
}

pub(super) async fn tick_write_settings(db: &Database) -> anyhow::Result<Option<Turn>> {
    db.write("online.kick.tick_write_settings", |db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let limited=tx.query_row("SELECT CAST(value AS INTEGER)>?1 FROM settings WHERE key='kick.rate_limited_until'",[now()],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);if limited{return Ok(None)}
        let t=tx.query_row("SELECT c.user_id,c.slug,c.generation,a.generation,c.failures FROM kick_channels c JOIN online_accounts a ON a.user_id=c.user_id AND a.provider='kick' AND a.status='connected' WHERE c.next_run<=?1 ORDER BY c.last_turn,c.user_id,c.slug LIMIT 1",[now()],|r|Ok(Turn{user:r.get(0)?,slug:r.get(1)?,generation:r.get(2)?,account:r.get(3)?,failures:r.get(4)?})).optional()?;
        if let Some(t)=&t{tx.execute("UPDATE kick_channels SET next_run=?1,last_turn=(SELECT COALESCE(MAX(last_turn),0)+1 FROM kick_channels) WHERE user_id=?2 AND slug=?3",params![now()+90,t.user,t.slug])?;}tx.commit()?;Ok(t)}).await
}

pub(super) async fn tick_write_kick_channels(
    t: Turn,
    delay: i64,
    error: ApiError,
    db: &Database,
) -> anyhow::Result<()> {
    db.write("online.kick.tick_write_kick_channels", move|db|{db.execute("UPDATE kick_channels SET error=?1,failures=failures+1,next_run=MAX(next_run,?2) WHERE user_id=?3 AND slug=?4 AND generation=?5",params![error.2,now()+delay,t.user,t.slug,t.generation])?;Ok(())}).await
}

pub(super) async fn step_write_settings(delay: i64, db: &Database) -> anyhow::Result<()> {
    db.write("online.kick.step_write_settings", move|db|{db.execute("INSERT INTO settings VALUES ('kick.rate_limited_until',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[(now()+delay).to_string()])?;Ok(())}).await
}

pub(super) struct ChannelUpdate {
    pub(super) live: bool,
    pub(super) viewers: i64,
    pub(super) title: String,
    pub(super) category: String,
    pub(super) thumbnail: Option<String>,
    pub(super) started_at: Option<String>,
    pub(super) display_name: Option<String>,
    pub(super) profile: Option<String>,
    pub(super) language: Option<String>,
    pub(super) mature: bool,
    pub(super) tags: Vec<String>,
    pub(super) user: String,
    pub(super) slug: String,
    pub(super) generation: String,
    pub(super) account: String,
}

pub(super) async fn step_write_kick_channels(
    db: &Database,
    input: ChannelUpdate,
) -> anyhow::Result<()> {
    let ChannelUpdate {
        live,
        viewers,
        title,
        category,
        thumbnail,
        started_at,
        display_name,
        profile,
        language,
        mature,
        tags,
        user,
        slug,
        generation,
        account,
    } = input;

    db.write("online.kick.step_write_kick_channels", move|db|{db.execute("UPDATE kick_channels SET title=?1,category=?2,live=?3,viewers=?4,updated_at=?5,next_run=?5+60,failures=0,error=NULL,thumbnail_url=?10,started_at=?11,display_name=COALESCE(?12,display_name),profile_image_url=COALESCE(?13,profile_image_url),language=?14,mature=?15,tags=?16 WHERE user_id=?6 AND slug=?7 AND generation=?8 AND EXISTS(SELECT 1 FROM online_accounts a WHERE a.user_id=?6 AND a.provider='kick' AND a.status='connected' AND a.generation=?9)",params![title,category,live,viewers,now(),user,slug,generation,account,thumbnail,started_at,display_name,profile,language,mature,serde_json::to_string(&tags)?])?;Ok(())}).await
}

pub(super) async fn presentation_metadata(delay: i64, db: &Database) -> anyhow::Result<()> {
    db.write("online.kick.presentation_metadata", move|db|{db.execute("INSERT INTO settings VALUES ('kick.rate_limited_until',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[(now()+delay).to_string()])?;Ok(())}).await
}
