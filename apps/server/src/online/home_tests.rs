use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use rusqlite::params;
use serde_json::{Value, json};

#[tokio::test]
async fn home_progress_includes_direct_videos_and_keeps_unstarted_subscriptions_private() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.home", |db| {
        db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES('alice','UCaaaaaaaaaaaaaaaaaaaaaa','Channel','s',1)", [])?;
        for (id, channel, position, watched, available, short, broadcast, played) in [
            ("aaaaaaaaaaa", "direct", 40., 0, 1, Some(0), "none", 200),
            ("bbbbbbbbbbb", "UCaaaaaaaaaaaaaaaaaaaaaa", 80., 0, 1, Some(0), "replay", 300),
            ("ccccccccccc", "UCaaaaaaaaaaaaaaaaaaaaaa", 0., 0, 1, Some(0), "none", 0),
            ("ddddddddddd", "direct", 0., 0, 1, Some(0), "none", 0),
            ("eeeeeeeeeee", "direct", 30., 1, 1, Some(0), "none", 500),
            ("fffffffffff", "direct", 30., 0, 0, Some(0), "none", 500),
            ("ggggggggggg", "direct", 30., 0, 1, Some(1), "none", 500),
            ("hhhhhhhhhhh", "direct", 30., 0, 1, Some(0), "live", 500),
            ("iiiiiiiiiii", "UCaaaaaaaaaaaaaaaaaaaaaa", 0., 0, 1, Some(0), "upcoming", 0),
            ("jjjjjjjjjjj", "UCaaaaaaaaaaaaaaaaaaaaaa", 0., 0, 1, None, "none", 0),
            ("mmmmmmmmmmm", "direct", 25., 0, 1, None, "none", 100),
        ] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title,channel_id,published_at,duration,broadcast,is_short,available,privacy) VALUES('alice',?1,?1,?2,100,600,?3,?4,?5,'public')", params![id,channel,broadcast,short,available])?;
            db.execute("INSERT INTO youtube_state(user_id,video_id,position,watched,updated_at) VALUES('alice',?1,?2,?3,?4)", params![id,position,watched,played])?;
        }
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,duration,is_short) VALUES('bob','kkkkkkkkkkk','Private Bob video',600,0)", [])?;
        db.execute("INSERT INTO youtube_state(user_id,video_id,position,updated_at) VALUES('bob','kkkkkkkkkkk',50,999)", [])?;
        Ok(())
    }).await.unwrap();
    let response = call(
        &state,
        "/api/v1/online/youtube/home",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let resume = response.2["continue_watching"].as_array().unwrap();
    assert_eq!(
        resume
            .iter()
            .map(|v| v["videoId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["bbbbbbbbbbb", "aaaaaaaaaaa", "mmmmmmmmmmm"]
    );
    assert_eq!(resume[0]["positionSeconds"], 80.);
    assert_eq!(resume[0]["last_played"], 300);
    assert!(
        resume[0]["thumbnailUrl"]
            .as_str()
            .unwrap()
            .contains("grant=")
    );
    assert_eq!(
        response.2["next_up"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["videoId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ccccccccccc"]
    );
    let bob =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
            .await
            .unwrap();
    let response = call(
        &state,
        "/api/v1/online/youtube/home",
        "GET",
        Value::Null,
        &format!("thelxinoe_session={bob}"),
    )
    .await;
    assert_eq!(
        response.2["continue_watching"][0]["title"],
        "Private Bob video"
    );
    assert!(response.2["next_up"].as_array().unwrap().is_empty());
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/home",
            "GET",
            Value::Null,
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // Disable through the actual administrative operation, using an admin session.
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/providers/youtube",
            "PUT",
            json!({"enabled":false}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/home",
            "GET",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
