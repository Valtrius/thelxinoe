//! Database operations for online.oauth.
use super::*;
use thelxinoe_database::Database;

pub(super) struct AuthorizationAttempt {
    pub(super) p: thelxinoe_core::Principal,
    pub(super) redirect: String,
    pub(super) client_hash: String,
    pub(super) hash: String,
    pub(super) encrypted: Vec<u8>,
    pub(super) browser_hash: String,
    pub(super) generation: String,
}

pub(super) async fn start(db: &Database, input: AuthorizationAttempt) -> anyhow::Result<()> {
    let AuthorizationAttempt {
        p,
        redirect,
        client_hash,
        hash,
        encrypted,
        browser_hash,
        generation,
    } = input;

    db.write("online.oauth.start", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM oauth_attempts WHERE expires_at<=?1 OR (user_id=?2 AND provider='youtube')",params![now(),p.user.id])?;
        tx.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES (?1,'youtube',?2,?3) ON CONFLICT(user_id,provider) DO UPDATE SET generation=excluded.generation",params![p.user.id,generation,now()])?;
        tx.execute("UPDATE youtube_sync SET generation=?1 WHERE user_id=?2",params![generation,p.user.id])?;
        tx.execute("INSERT INTO oauth_attempts VALUES (?1,?2,?3,'youtube',?4,?5,?6,?7,?8,?9)",params![hash,p.user.id,p.session_id,generation,browser_hash,encrypted,redirect,client_hash,now()+600])?;
        tx.commit()?;Ok(())
    }).await
}

pub(super) async fn consume(
    browser_hash: String,
    hash: String,
    db: &Database,
) -> anyhow::Result<Option<Attempt>> {
    db.write("online.oauth.consume", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result=tx.query_row("SELECT a.user_id,a.session_id,a.generation,a.verifier,a.redirect_uri,a.client_hash FROM oauth_attempts a JOIN sessions s ON s.id=a.session_id JOIN online_accounts o ON o.user_id=a.user_id AND o.provider=a.provider AND o.generation=a.generation WHERE a.state_hash=?1 AND a.browser_hash=?2 AND a.expires_at>?3 AND s.expires_at>?3 AND s.user_id=a.user_id AND s.transport='web'",params![hash,browser_hash,now()],|r|Ok(Attempt{user:r.get(0)?,session:r.get(1)?,generation:r.get(2)?,verifier:r.get(3)?,redirect:r.get(4)?,client_hash:r.get(5)?})).optional()?;
        if result.is_some(){tx.execute("DELETE FROM oauth_attempts WHERE state_hash=?1",[hash])?;}
        tx.commit()?;Ok(result)
    }).await
}

pub(super) async fn complete(
    attempt: Attempt,
    expires: i64,
    name: String,
    avatar: Option<String>,
    external: String,
    encrypted: Vec<u8>,
    db: &Database,
) -> anyhow::Result<bool> {
    db.write("online.oauth.complete", move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let saved=tx.execute("UPDATE online_accounts SET status='connected',credential=?1,expires_at=?2,display_name=?3,external_id=?4,updated_at=?5,profile_checked_at=?5,avatar_url=?9 WHERE user_id=?6 AND provider='youtube' AND generation=?7 AND EXISTS(SELECT 1 FROM sessions WHERE id=?8 AND user_id=?6 AND expires_at>?5)",params![encrypted,now()+expires,name,external,now(),attempt.user,attempt.generation,attempt.session,avatar])?==1;
        if saved {tx.execute("INSERT INTO youtube_sync(user_id,generation) VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET generation=excluded.generation,cursor='{}',next_run=0,failures=0,error=NULL",params![attempt.user,attempt.generation])?;}
        tx.commit()?;Ok(saved)
    }).await
}
