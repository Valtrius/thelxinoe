use super::*;
use crate::online::oauth::tests::{call, fixture};

#[tokio::test]
async fn stale_cleanup_cannot_fail_a_new_generation_or_reopen_a_terminal_session() {
    let (_temp, state, cookie) = fixture().await;
    state.db.write("test.playback", |db| {
        db.execute("INSERT INTO youtube_media VALUES ('abcdefghijk')", [])?;
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id,streaming) SELECT 'session','alice',id,'new','public','playing','transcode','{}',100,?1,?1,'abcdefghijk',1 FROM sessions WHERE user_id='alice' LIMIT 1", [now()])?;
        Ok(())
    }).await.unwrap();
    storage::fail_generation("session".into(), "old".into(), &state.db)
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/playback/session/progress",
            "POST",
            json!({"sequence":1,"position":1,"state":"playing"}),
            &cookie
        )
        .await
        .0,
        axum::http::StatusCode::OK
    );
    call(
        &state,
        "/api/v1/playback/session/progress",
        "POST",
        json!({"sequence":2,"position":1,"state":"stopped"}),
        &cookie,
    )
    .await;
    storage::fail_generation("session".into(), "new".into(), &state.db)
        .await
        .unwrap();
    let status: String = state
        .db
        .read("test.status", |db| {
            Ok(db.query_row(
                "SELECT state FROM playback_sessions WHERE id='session'",
                [],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(status, "stopped");
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &cookie)
            .await
            .0,
        axum::http::StatusCode::OK
    );
}
