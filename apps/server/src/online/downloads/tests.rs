use super::*;
use crate::test_support::{call, fixture};
use axum::http::StatusCode;
const VIDEO: &str = "abcdefghijk";

#[path = "auto_delete_tests.rs"]
mod auto_delete_tests;

#[test]
fn progress_accepts_estimates_and_rejects_invalid_byte_counts() {
    assert_eq!(
        parse_progress("THELXINOE_PROGRESS:25\t100\tNA\t3\th264\tnone"),
        Some((25, Some(100), Some(3), "video"))
    );
    assert_eq!(
        parse_progress("THELXINOE_PROGRESS:25\tNA\t50\tNA\tnone\topus"),
        Some((25, Some(50), None, "audio"))
    );
    assert_eq!(
        parse_progress("THELXINOE_PROGRESS:25\t250000000000")
            .unwrap()
            .1,
        Some(250_000_000_000),
        "a download has no size of its own"
    );
    for input in ["NaN", "inf", "-1", "1e19"] {
        assert!(parse_progress(&format!("THELXINOE_PROGRESS:{input}\t100")).is_none());
    }
    assert!(parse_progress("unrelated extractor output").is_none());
}

#[tokio::test]
async fn download_progress_reaches_only_users_with_that_video() {
    let (_temp, state, _) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES('alice',?1,'Video')", [VIDEO])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,requested_at,updated_at,tools,downloaded_bytes,total_bytes,eta_seconds,media_kind) VALUES(?1,'g','downloading',1,1,'{}',25,100,3,'video')", [VIDEO])?;
        Ok(())
    }).await.unwrap();
    publish_progress(&state, VIDEO).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            let (user, payload): (String, String) = db.query_row(
                "SELECT user_id,payload FROM events WHERE kind='online.download.progress'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            assert_eq!(user, "alice");
            let payload: Value = serde_json::from_str(&payload)?;
            assert_eq!(payload["downloaded_bytes"], 25);
            assert_eq!(payload["total_bytes"], 100);
            assert_eq!(payload["state"], "downloading");
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn auto_removal_keeps_other_lists_and_the_manual_undo_window() {
    let (_temp, state, _) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES('alice',?1,'Video')",[VIDEO])?;
        for (id,auto_remove) in [(1,1),(2,0)] {
            db.execute("INSERT INTO youtube_watchlists(id,user_id,name,auto_remove_watched,created_at,updated_at) VALUES(?1,'alice','Test',?2,1,1)",params![id,auto_remove])?;
            db.execute("INSERT INTO youtube_watchlist_items VALUES('alice',?1,?2,0,1)",params![id,VIDEO])?;
        }
        db.execute("INSERT INTO youtube_state(user_id,video_id,watched,updated_at) VALUES('alice',?1,1,?2)",params![VIDEO,now()])?;
        Ok(())
    }).await.unwrap();
    maintain_watchlists(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM youtube_watchlist_items",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        2
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE youtube_state SET updated_at=?1", [now() - 6])?;
            Ok(())
        })
        .await
        .unwrap();
    maintain_watchlists(&state).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            assert_eq!(
                db.query_row(
                    "SELECT watchlist_id FROM youtube_watchlist_items",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                2
            );
            assert!(db.query_row(
                "SELECT watchlist FROM youtube_video_state WHERE user_id='alice'",
                [],
                |r| r.get::<_, bool>(0)
            )?);
            Ok(())
        })
        .await
        .unwrap();
    // Re-adding an already watched video gets its own undo window. Membership
    // changes no longer rewrite the user's playback-state timestamp.
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "INSERT INTO youtube_watchlist_items VALUES('alice',1,?1,0,?2)",
                params![VIDEO, now()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    maintain_watchlists(&state).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM youtube_watchlist_items", [], |r| r
                    .get::<_, u32>(0))?,
                2
            );
            db.execute(
                "UPDATE youtube_watchlist_items SET added_at=?1 WHERE watchlist_id=1",
                [now() - 6],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    maintain_watchlists(&state).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            assert_eq!(
                db.query_row(
                    "SELECT watchlist_id FROM youtube_watchlist_items",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                2
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn both_admin_configuration_routes_control_the_same_download_policy() {
    let (_temp, state, cookie) = fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/downloads",
            "GET",
            json!({}),
            &cookie
        )
        .await
        .2["enabled"],
        false
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/downloads",
            "PUT",
            json!({"enabled":true}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/admin/online", "GET", json!({}), &cookie)
            .await
            .2["youtube_downloads"],
        true
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online",
            "PUT",
            json!({"youtube_downloads":false,"youtube_daily_quota":10000,"youtube_network":"ipv6"}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/downloads",
            "GET",
            json!({}),
            &cookie
        )
        .await
        .2["enabled"],
        false
    );
    let online = call(&state, "/api/v1/admin/online", "GET", json!({}), &cookie)
        .await
        .2;
    assert_eq!(online["youtube_network"], "ipv6");
    assert!(online["youtube_network_status"]["ipv6"].is_boolean());
    assert_eq!(online["youtube_download_quality"], "best");
    for (body, status) in [
        (json!({"youtube_download_quality":"1440p"}), StatusCode::OK),
        (json!({}), StatusCode::OK),
        (
            json!({"youtube_download_quality":"1081p"}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut settings =
            json!({"youtube_downloads":false,"youtube_daily_quota":10000,"youtube_network":"ipv6"});
        settings
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        assert_eq!(
            call(&state, "/api/v1/admin/online", "PUT", settings, &cookie)
                .await
                .0,
            status
        );
    }
    assert_eq!(
        call(&state, "/api/v1/admin/online", "GET", json!({}), &cookie)
            .await
            .2["youtube_download_quality"],
        "1440p",
        "clients without the quality keep it"
    );
    assert_eq!(format_sort("1440p"), "res:1440,fps,hdr:sdr");
    assert_eq!(format_sort("best"), "res,fps,hdr:sdr");
    // An unknown family is rejected instead of silently selecting one.
    assert!(
        call(
            &state,
            "/api/v1/admin/online",
            "PUT",
            json!({"youtube_downloads":false,"youtube_daily_quota":10000,"youtube_network":"ipv5"}),
            &cookie
        )
        .await
        .0
        .is_client_error()
    );
}

#[tokio::test]
async fn public_file_sharing_keeps_progress_private_and_retention_fenced() {
    let (_temp, state, alice) = fixture().await;
    let generation = thelxinoe_core::id();
    let directory = state
        .config
        .cache
        .join("youtube")
        .join(VIDEO)
        .join(&generation);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let path = directory.join("media.mp4");
    tokio::fs::write(&path, b"fixture media bytes")
        .await
        .unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let generation_copy = generation.clone();
    state.db.write("test.fixture", move|db| {
        db.execute("INSERT INTO youtube_media(video_id) VALUES (?1)",[VIDEO])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('alice',?1,'Public fixture','public')",[VIDEO])?;
        let list=crate::online::watchlists::default_list(db,"alice")?;
        db.execute("INSERT INTO youtube_watchlist_items VALUES('alice',?1,?2,0,1)",params![list,VIDEO])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,path,size,modified,probe,requested_at,updated_at,completed_at) VALUES (?1,?2,'ready','{}',?3,?4,?5,?6,1,1,1)",params![VIDEO,generation_copy,path.to_string_lossy(),metadata.len() as i64,metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),json!({"format":{"duration":"100"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264"}]}).to_string()])?;
        Ok(())
    }).await.unwrap();
    let bob_token = crate::test_support::issue_session(
        &state.db,
        "bob".into(),
        "web".into(),
        "Bob browser".into(),
    )
    .await
    .unwrap();
    let bob = format!("thelxinoe_session={bob_token}");
    let body = json!({"media_id":format!("youtube:{VIDEO}"),"options":{"quality":"auto","capabilities":{"containers":["mp4"],"video":["h264"],"audio":[],"hls":false}}});
    assert_eq!(
        call(&state, "/api/v1/playback", "POST", body.clone(), &bob)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/tools/check",
            "POST",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    crate::auto_delete::evaluate(&state).await.unwrap();
    assert!(
        directory.exists(),
        "Alice watchlist protects the physical file"
    );
    let played = call(&state, "/api/v1/playback", "POST", body.clone(), &alice).await;
    assert_eq!(played.0, StatusCode::OK, "{}", played.2);
    let id = played.2["id"].as_str().unwrap();
    let progress = format!("/api/v1/playback/{id}/progress");
    assert_eq!(
        call(
            &state,
            &progress,
            "POST",
            json!({"sequence":1,"position":35,"state":"playing"}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    // The report's event carries the position, so clients move the card in place.
    let change: Value = state
        .db
        .read("test.fixture", |db| {
            Ok(serde_json::from_str(&db.query_row(
                "SELECT payload FROM events WHERE kind='playback.changed' AND user_id='alice' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )?)?)
        })
        .await
        .unwrap();
    assert_eq!(change["media_id"], format!("youtube:{VIDEO}"));
    assert_eq!(
        (
            change["state"].as_str(),
            change["position"].as_f64(),
            change["duration"].as_f64(),
            change["watched"].as_bool()
        ),
        (Some("playing"), Some(35.0), Some(100.0), Some(false))
    );
    assert_eq!(
        call(
            &state,
            &progress,
            "POST",
            json!({"sequence":2,"position":95,"state":"playing"}),
            &bob
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    state.db.write("test.fixture", |db|{db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('bob',?1,'Same public fixture','public')",[VIDEO])?;db.execute("INSERT INTO youtube_state(user_id,video_id,pinned,updated_at) VALUES ('bob',?1,1,1)",[VIDEO])?;Ok(())}).await.unwrap();
    let second = call(&state, "/api/v1/playback", "POST", body, &bob).await;
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(second.2["position"], 0.0);
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/data",
            "DELETE",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &progress,
            "POST",
            json!({"sequence":3,"position":95,"state":"playing"}),
            &alice
        )
        .await
        .2["accepted"],
        false
    );
    crate::auto_delete::evaluate(&state).await.unwrap();
    assert!(
        directory.exists(),
        "Bob pin protects shared file after Alice deletes data"
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE youtube_state SET pinned=0", [])?;
            db.execute(
                "UPDATE retention_policies SET storage_limit_bytes=1 WHERE domain='videos'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    crate::auto_delete::evaluate(&state).await.unwrap();
    assert!(directory.exists(), "Active Bob playback protects the file");
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE playback_sessions SET state='stopped'", [])?;
            db.execute(
                "UPDATE retention_policies SET storage_limit_bytes=1 WHERE domain='videos'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let replaced = directory.join("media.mp4");
    tokio::fs::write(&replaced, b"a different replacement generation")
        .await
        .unwrap();
    crate::auto_delete::evaluate(&state).await.unwrap();
    assert!(
        replaced.exists(),
        "A replacement must not be deleted using old metadata"
    );
    crate::auto_delete::evaluate(&state).await.unwrap();
    assert!(
        replaced.exists(),
        "Blocked generations must not be retried automatically"
    );
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM media",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0,
        "Online metadata never enters the local catalog"
    );
}

#[test]
fn format_reads_the_video_picture_rather_than_cover_art() {
    let probe = json!({"streams":[
        {"index":0,"codec_type":"video","codec_name":"mjpeg","width":1280,"height":720,"avg_frame_rate":"0/0","disposition":{"attached_pic":1}},
        {"index":1,"codec_type":"audio","codec_name":"opus"},
        {"index":2,"codec_type":"video","codec_name":"vp9","width":3840,"height":2160,"avg_frame_rate":"24000/1001","disposition":{"attached_pic":0}}
    ]})
    .to_string();
    let found = format(Some(&probe));
    assert_eq!((found.width, found.height), (Some(3840), Some(2160)));
    assert!((found.frame_rate.unwrap() - 23.976).abs() < 0.001);
    // A stream without an average rate still has its nominal one.
    let nominal = json!({"streams":[{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"0/0","r_frame_rate":"60/1"}]}).to_string();
    assert_eq!(format(Some(&nominal)).frame_rate, Some(60.0));
    for missing in [
        None,
        Some("not json"),
        Some(r#"{"streams":[{"codec_type":"audio"}]}"#),
    ] {
        let found = format(missing);
        assert_eq!(
            (found.width, found.height, found.frame_rate),
            (None, None, None)
        );
    }
}

#[tokio::test]
async fn ready_downloads_name_their_format_in_lists_status_and_progress() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES('alice',?1,'Video')", [VIDEO])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,path,size,probe,requested_at,updated_at,completed_at) VALUES(?1,'g','ready','{}','/media.webm',100,?2,1,1,1)", params![VIDEO, json!({"format":{"duration":"100"},"streams":[{"index":0,"codec_type":"video","width":3840,"height":2160,"avg_frame_rate":"60/1"}]}).to_string()])?;
        Ok(())
    }).await.unwrap();
    let listed = state
        .db
        .read("test.fixture", |db| {
            super::super::browse::one(db, "alice", VIDEO, "grant")
        })
        .await
        .unwrap();
    assert_eq!(
        (
            &listed["download"]["widthPixels"],
            &listed["download"]["heightPixels"],
            &listed["download"]["frameRate"]
        ),
        (&json!(3840), &json!(2160), &json!(60.0))
    );
    assert!(
        listed["download"].get("quality").is_none(),
        "No format is guessed"
    );
    let status = call(
        &state,
        &format!("/api/v1/online/youtube/videos/{VIDEO}/download"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status.0, StatusCode::OK, "{}", status.2);
    assert_eq!(
        (
            &status.2["download"]["height"],
            &status.2["download"]["frame_rate"]
        ),
        (&json!(2160), &json!(60.0))
    );
    publish_progress(&state, VIDEO).await.unwrap();
    let progress: Value = state
        .db
        .read("test.fixture", |db| {
            Ok(serde_json::from_str(&db.query_row(
                "SELECT payload FROM events WHERE kind='online.download.progress'",
                [],
                |r| r.get::<_, String>(0),
            )?)?)
        })
        .await
        .unwrap();
    assert_eq!(
        (
            &progress["width"],
            &progress["height"],
            &progress["frame_rate"]
        ),
        (&json!(3840), &json!(2160), &json!(60.0))
    );
}
