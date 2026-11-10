use super::*;
use crate::test_support::{call, fixture};

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

#[tokio::test]
async fn youtube_videos_become_watched_only_once_playback_ends_past_the_threshold() {
    let (_temp, state, cookie) = fixture().await;
    state.db.write("test.playback", |db| {
        db.execute("INSERT INTO youtube_media VALUES ('abcdefghijk')", [])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice','abcdefghijk','Video')", [])?;
        for id in ["closed", "abandoned"] {
            db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id,streaming) SELECT ?1,'alice',id,'g','public','ready','transcode','{}',100,?2,?2,'abcdefghijk',1 FROM sessions WHERE user_id='alice' LIMIT 1", params![id, now()])?;
        }
        Ok(())
    }).await.unwrap();
    let watched = || async {
        state
            .db
            .read("test.watched", |db| {
                Ok(db.query_row(
                    "SELECT watched FROM youtube_state WHERE user_id='alice' AND video_id='abcdefghijk'",
                    [],
                    |row| row.get::<_, bool>(0),
                )?)
            })
            .await
            .unwrap()
    };
    for (sequence, state_name, expected) in [
        (1, "playing", false),
        (2, "paused", false),
        (3, "stopped", true),
    ] {
        let report = call(
            &state,
            "/api/v1/playback/closed/progress",
            "POST",
            json!({"sequence":sequence,"position":95,"state":state_name}),
            &cookie,
        )
        .await;
        assert_eq!(report.0, axum::http::StatusCode::OK, "{}", report.2);
        assert_eq!(watched().await, expected, "after a {state_name} report");
    }

    // A tab closed mid-video sends no final report; its session expires instead.
    state
        .db
        .write("test.unwatch", |db| {
            db.execute("UPDATE youtube_state SET watched=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    call(
        &state,
        "/api/v1/playback/abandoned/progress",
        "POST",
        json!({"sequence":1,"position":95,"state":"playing"}),
        &cookie,
    )
    .await;
    assert!(!storage::expire_sessions(&state.db).await.unwrap().1);
    assert!(!watched().await, "a session still reporting keeps playing");
    state
        .db
        .write("test.abandon", |db| {
            db.execute(
                "UPDATE playback_sessions SET updated_at=?1 WHERE id='abandoned'",
                [now() - 121],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(storage::expire_sessions(&state.db).await.unwrap().1);
    assert!(watched().await);
    let change: Value = state
        .db
        .read("test.event", |db| {
            Ok(serde_json::from_str(&db.query_row(
                "SELECT payload FROM events WHERE kind='playback.changed' AND user_id='alice' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )?)?)
        })
        .await
        .unwrap();
    assert_eq!(
        change,
        json!({"id":"abandoned","state":"stopped","media_id":"youtube:abcdefghijk","position":95.0,"duration":100.0,"watched":true})
    );
}

#[tokio::test]
async fn abandoned_playback_that_finishes_a_video_removes_it_from_auto_remove_watchlists() {
    let (_temp, state, cookie) = fixture().await;
    state.db.write("test.playback", |db| {
        db.execute("INSERT INTO youtube_media VALUES ('abcdefghijk')", [])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice','abcdefghijk','Video')", [])?;
        // Added after it was watched, so only finishing it again removes it.
        db.execute("INSERT INTO youtube_state(user_id,video_id,watched,updated_at) VALUES ('alice','abcdefghijk',1,1)", [])?;
        db.execute("INSERT INTO youtube_watchlists(id,user_id,name,auto_remove_watched,created_at,updated_at) VALUES (1,'alice','Later',1,1,1)", [])?;
        db.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES ('alice',1,'abcdefghijk',0,1)", [])?;
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id,streaming) SELECT 'abandoned','alice',id,'g','public','ready','transcode','{}',100,?1,?1,'abcdefghijk',1 FROM sessions WHERE user_id='alice' LIMIT 1", [now()])?;
        Ok(())
    }).await.unwrap();
    let listed = || async {
        state
            .db
            .read("test.listed", |db| {
                Ok(
                    db.query_row("SELECT COUNT(*) FROM youtube_watchlist_items", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap()
    };
    call(
        &state,
        "/api/v1/playback/abandoned/progress",
        "POST",
        json!({"sequence":1,"position":95,"state":"playing"}),
        &cookie,
    )
    .await;
    storage::expire_sessions(&state.db).await.unwrap();
    assert_eq!(listed().await, 1, "a session still reporting keeps playing");
    state
        .db
        .write("test.abandon", |db| {
            db.execute(
                "UPDATE playback_sessions SET updated_at=?1 WHERE id='abandoned'",
                [now() - 121],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(storage::expire_sessions(&state.db).await.unwrap().1);
    assert_eq!(listed().await, 0);
}
