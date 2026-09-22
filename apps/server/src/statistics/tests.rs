use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;

#[tokio::test]
async fn playback_changes_roll_back_when_their_event_cannot_be_saved() {
    let (_temp, state, alice, _) = live_fixture().await;
    state.db.write("test.fail_event", |db| {
        db.execute_batch("CREATE TEMP TRIGGER reject_playback_event BEFORE INSERT ON events WHEN NEW.kind='playback.changed' BEGIN SELECT RAISE(ABORT,'test event failure'); END;")?;
        Ok(())
    }).await.unwrap();
    let path = "/api/v1/playback/alice/progress";
    let event = json!({"sequence":0,"position":12.0,"active_seconds":0.0,"state":"playing"});
    assert_eq!(
        call(&state, path, "POST", event.clone(), &alice).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    state
        .db
        .write("test.check_rollback", |db| {
            assert_eq!(
                db.query_row(
                    "SELECT state FROM playback_sessions WHERE id='alice'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "ready"
            );
            for table in [
                "playback_history",
                "playback_statistics",
                "playback_activity",
            ] {
                assert_eq!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
            }
            db.execute_batch("DROP TRIGGER reject_playback_event")?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(&state, path, "POST", event.clone(), &alice).await.2["accepted"],
        true
    );
    assert_eq!(
        call(&state, path, "POST", event, &alice).await.2["accepted"],
        false
    );
    let events = state
        .db
        .read("test.events", |db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='playback.changed'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(events, 1);
    state.db.shutdown().await.unwrap();
}

async fn live_fixture() -> (tempfile::TempDir, AppState, String, String) {
    let (temp, state, alice) = fixture().await;
    let token = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
        .await
        .unwrap();
    state.db.write("test.fixture", |db|{
        db.execute("INSERT INTO live_media VALUES ('twitch:42','Live fixture')",[])?;
        for user in ["alice","bob"] {
            db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES (?1,'42','fixture','Fixture','Live fixture','Science',10,'today','s',1)",[user])?;
            db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,live_media_id,streaming) SELECT ?1,user_id,id,'g','live','ready','transcode','{}',0,?2,?2,'twitch:42',1 FROM sessions WHERE user_id=?1 LIMIT 1",params![user,thelxinoe_core::now()-60])?;
        }
        Ok(())
    }).await.unwrap();
    (temp, state, alice, format!("thelxinoe_session={token}"))
}

async fn backdate(state: &AppState, id: &'static str, milliseconds: i64) {
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "UPDATE playback_sessions SET reported_at_ms=?1 WHERE id=?2",
                params![Utc::now().timestamp_millis() - milliseconds, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn statistics_progress_is_idempotent_private_and_survives_session_revocation() {
    let (_temp, state, alice, bob) = live_fixture().await;
    let path = "/api/v1/playback/alice/progress";
    let event = |seq, pos, active, status| json!({"sequence":seq,"position":pos,"active_seconds":active,"state":status});
    assert_eq!(
        call(&state, path, "POST", event(0, 0.0, 0.0, "playing"), &bob)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&state, path, "POST", event(0, 0.0, -1.0, "playing"), &alice)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&state, path, "POST", event(0, 0.0, 0.0, "playing"), &alice)
            .await
            .0,
        StatusCode::OK
    );
    backdate(&state, "alice", 10_000).await;
    // Ten wall seconds at 2x speed still count as ten seconds.
    call(
        &state,
        path,
        "POST",
        event(1, 20.0, 10.0, "playing"),
        &alice,
    )
    .await;
    let duplicate = call(
        &state,
        path,
        "POST",
        event(1, 20.0, 10.0, "playing"),
        &alice,
    )
    .await
    .2;
    assert_eq!(duplicate["accepted"], false);
    // A seek and paused time cannot increase a cumulative activity counter.
    call(
        &state,
        path,
        "POST",
        event(2, 2000.0, 10.0, "paused"),
        &alice,
    )
    .await;
    backdate(&state, "alice", 20_000).await;
    call(
        &state,
        path,
        "POST",
        event(3, 2000.0, 10.0, "paused"),
        &alice,
    )
    .await;
    let mine = call(
        &state,
        "/api/v1/me/statistics?range=all",
        "GET",
        Value::Null,
        &alice,
    )
    .await
    .2;
    assert_eq!(mine["totalActiveSeconds"], 10.0);
    assert_eq!(mine["twitchChannelsWatched"], 1);
    assert_eq!(mine["topChannels"][0]["name"], "Fixture");
    assert_eq!(
        call(
            &state,
            "/api/v1/me/history?platform=twitch&range=all",
            "GET",
            Value::Null,
            &alice
        )
        .await
        .2["items"][0]["played_seconds"],
        10.0
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/statistics?user=alice",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .2["totalActiveSeconds"],
        0.0
    );
    assert_eq!(
        call(&state, "/api/v1/admin/statistics", "GET", Value::Null, &bob)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let aggregate = call(
        &state,
        "/api/v1/admin/statistics?range=all",
        "GET",
        Value::Null,
        &bob,
    )
    .await
    .2;
    assert_eq!(aggregate["totalActiveSeconds"], 10.0);
    assert_eq!(aggregate["users"][0]["username"], "alice");
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/statistics?range=all&user=bob",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .2["totalActiveSeconds"],
        0.0
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/statistics?platform=bad",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // Restarting/opening the server and signing out do not remove activity.
    state
        .db
        .write("test.fixture", |db| {
            db.execute("DELETE FROM sessions WHERE user_id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let reopened = AppState::open(state.config.as_ref().clone()).await.unwrap();
    assert_eq!(
        call(
            &reopened,
            "/api/v1/admin/statistics?range=all&user=alice",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .2["totalActiveSeconds"],
        10.0
    );
    assert_eq!(
        call(
            &reopened,
            "/api/v1/playback/alice/progress",
            "POST",
            event(4, 2001.0, 11.0, "playing"),
            &alice
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn statistics_split_minutes_and_local_days_without_counting_prefetch_or_idle() {
    let (_temp, state, _, _) = live_fixture().await;
    state.db.write("test.fixture", |db|{
        let start="2026-07-11T03:59:55Z".parse::<DateTime<Utc>>()?.timestamp_millis();
        let tx=db.transaction()?;
        let input=|total,state: &str|Progress{sequence:0,position:0.0,state:state.into(),active_seconds:Some(total)};
        record_at(&tx,"alice",&input(0.0,"paused"),start)?;
        assert_eq!(tx.query_row("SELECT COUNT(*) FROM playback_statistics",[],|r|r.get::<_,u32>(0))?,0);
        record_at(&tx,"alice",&input(0.0,"playing"),start)?;
        assert_eq!(record_at(&tx,"alice",&input(10.0,"playing"),start+10_000)?,10.0);
        tx.commit()?;
        let zone="America/New_York".parse()?;
        let result=aggregate(db,Some("alice"),zone,"7d","all",false,"2026-07-11T05:00:00Z".parse()?)?;
        assert_eq!(result["totalActiveSeconds"],10.0);
        assert_eq!(result["activeDays"],2);
        assert_eq!(result["activity"].as_array().unwrap().len(),7);
        assert_eq!(result["rhythm"].as_array().unwrap().len(),168);
        let activity=result["activity"].as_array().unwrap();
        assert_eq!(activity[5]["periodStart"],"2026-07-10");
        assert_eq!(activity[5]["twitchSeconds"],5.0);
        assert_eq!(activity[6]["twitchSeconds"],5.0);
        assert_eq!(result["rhythm"][4*24+23]["twitchSeconds"],5.0);
        assert_eq!(result["rhythm"][5*24]["twitchSeconds"],5.0);
        // Both occurrences of 01:30 during fall-back share the local rhythm cell.
        for instant in ["2026-11-01T05:30:00Z","2026-11-01T06:30:00Z"] {
            let at=instant.parse::<DateTime<Utc>>()?.timestamp();
            db.execute("INSERT INTO playback_activity VALUES ('alice',?1,'twitch','twitch:42',60,'Live fixture','42','Fixture','Science','live')",[at])?;
        }
        let result=aggregate(db,Some("alice"),zone,"7d","twitch",false,"2026-11-01T08:00:00Z".parse()?)?;
        assert_eq!(result["totalActiveSeconds"],120.0);
        assert_eq!(result["rhythm"][6*24+1]["twitchSeconds"],120.0);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn statistics_completion_and_extra_media_counts_follow_user_state() {
    let (_temp, state, _) = fixture().await;
    state.db.write("test.fixture", |db|{
        let now=Utc::now();
        for (platform,media,kind,name,category,done) in [
            ("youtube","youtube:a","upload","Channel","",true),
            ("youtube","youtube:b","short","Channel","",true),
            ("youtube","youtube:c","live_replay","Channel","",true),
            ("movies","movie","movie","Film","",true),
            ("shows","episode","episode","Show","",false),
            ("music","track","track","Artist","Album",true),
        ] {
            db.execute("INSERT INTO playback_statistics VALUES ('alice',?1,?2,?2,?3,?3,?4,?5,?6,?6,100,?7)",params![platform,media,name,category,kind,now.timestamp(),done])?;
        }
        let result=aggregate(db,Some("alice"),chrono_tz::UTC,"7d","all",false,now)?;
        assert_eq!(result["youtubeVideosWatched"],3);
        assert_eq!(result["youtubeContentMix"],json!({"uploads":1,"liveReplays":1,"shorts":1}));
        assert_eq!(result["moviesWatched"],1);
        assert_eq!(result["episodesStarted"],1);
        assert_eq!(result["episodesWatched"],0);
        assert_eq!(result["tracksCompleted"],1);
        assert_eq!(result["artistsListened"],1);
        assert_eq!(result["albumsListened"],1);
        let films=aggregate(db,Some("alice"),chrono_tz::UTC,"all","movies",false,now)?;
        assert_eq!(films["moviesWatched"],1);
        assert_eq!(films["youtubeVideosWatched"],0);
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice','a','Video')",[])?;
        db.execute("INSERT INTO youtube_state(user_id,video_id,watched,updated_at) VALUES ('alice','a',0,1)",[])?;
        assert_eq!(aggregate(db,Some("alice"),chrono_tz::UTC,"all","youtube",false,now)?["youtubeVideosWatched"],2);
        db.execute("UPDATE youtube_state SET watched=1 WHERE user_id='alice' AND video_id='a'",[])?;
        db.execute("DELETE FROM youtube_videos WHERE user_id='alice' AND video_id='a'",[])?;
        assert_eq!(aggregate(db,Some("alice"),chrono_tz::UTC,"all","youtube",false,now)?["youtubeVideosWatched"],3);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn statistics_provider_data_erasure_is_user_scoped_and_cannot_be_undone_by_late_progress() {
    let (_temp, state, alice, bob) = live_fixture().await;
    for (user, cookie) in [("alice", &alice), ("bob", &bob)] {
        let path = format!("/api/v1/playback/{user}/progress");
        call(
            &state,
            &path,
            "POST",
            json!({"sequence":0,"position":0,"state":"playing","active_seconds":0}),
            cookie,
        )
        .await;
        backdate(&state, user, 5000).await;
        call(
            &state,
            &path,
            "POST",
            json!({"sequence":1,"position":5,"state":"playing","active_seconds":5}),
            cookie,
        )
        .await;
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/online/twitch/data",
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    let late = call(
        &state,
        "/api/v1/playback/alice/progress",
        "POST",
        json!({"sequence":2,"position":10,"state":"playing","active_seconds":10}),
        &alice,
    )
    .await
    .2;
    assert_eq!(late["accepted"], false);
    assert_eq!(
        call(
            &state,
            "/api/v1/me/statistics?range=all",
            "GET",
            Value::Null,
            &alice
        )
        .await
        .2["totalActiveSeconds"],
        0.0
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/statistics?range=all",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .2["totalActiveSeconds"],
        5.0
    );
}
