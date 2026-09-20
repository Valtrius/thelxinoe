use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
const VIDEO: &str = "abcdefghijk";

#[tokio::test]
async fn auto_removal_keeps_other_lists_and_the_manual_undo_window() {
    let (_temp, state, _) = fixture().await;
    state.db.call(|db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES('alice',?1,'Video')",[VIDEO])?;
        for (id,auto_remove) in [(1,1),(2,0)] {
            db.execute("INSERT INTO youtube_watchlists(id,user_id,name,auto_remove_watched,created_at,updated_at) VALUES(?1,'alice','Test',?2,1,1)",params![id,auto_remove])?;
            db.execute("INSERT INTO youtube_watchlist_items VALUES('alice',?1,?2,0,1)",params![id,VIDEO])?;
        }
        db.execute("UPDATE youtube_state SET watched=1,updated_at=?1 WHERE user_id='alice'",[now()])?;
        Ok(())
    }).await.unwrap();
    maintain_watchlists(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .call(|db| Ok(db.query_row(
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
        .call(|db| {
            db.execute("UPDATE youtube_state SET updated_at=?1", [now() - 6])?;
            Ok(())
        })
        .await
        .unwrap();
    maintain_watchlists(&state).await.unwrap();
    state
        .db
        .call(|db| {
            assert_eq!(
                db.query_row(
                    "SELECT watchlist_id FROM youtube_watchlist_items",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                2
            );
            assert!(db.query_row(
                "SELECT watchlist FROM youtube_state WHERE user_id='alice'",
                [],
                |r| r.get::<_, bool>(0)
            )?);
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
        .call(|db| {
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
            json!({"youtube_downloads":false,"youtube_daily_quota":10000}),
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
    state.db.call(move|db| {
        db.execute("INSERT INTO youtube_media(video_id) VALUES (?1)",[VIDEO])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('alice',?1,'Public fixture','public')",[VIDEO])?;
        db.execute("INSERT INTO youtube_state(user_id,video_id,watchlist,added_at,updated_at) VALUES ('alice',?1,1,1,1)",[VIDEO])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,path,size,modified,probe,requested_at,updated_at,unprotected_at) VALUES (?1,?2,'ready','{}',?3,?4,?5,?6,1,1,1)",params![VIDEO,generation_copy,path.to_string_lossy(),metadata.len() as i64,metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),json!({"format":{"duration":"100"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264"}]}).to_string()])?;
        Ok(())
    }).await.unwrap();
    let bob_token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob browser".into())
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
            "/api/v1/admin/online/tools",
            "POST",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    cleanup(&state).await.unwrap();
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
    state.db.call(|db|{db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('bob',?1,'Same public fixture','public')",[VIDEO])?;db.execute("INSERT INTO youtube_state(user_id,video_id,pinned,added_at,updated_at) VALUES ('bob',?1,1,1,1)",[VIDEO])?;Ok(())}).await.unwrap();
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
    cleanup(&state).await.unwrap();
    assert!(
        directory.exists(),
        "Bob pin protects shared file after Alice deletes data"
    );
    state
        .db
        .call(|db| {
            db.execute("UPDATE youtube_state SET pinned=0", [])?;
            db.execute("UPDATE youtube_downloads SET unprotected_at=1", [])?;
            Ok(())
        })
        .await
        .unwrap();
    cleanup(&state).await.unwrap();
    assert!(directory.exists(), "Active Bob playback protects the file");
    state
        .db
        .call(|db| {
            db.execute("UPDATE playback_sessions SET state='stopped'", [])?;
            db.execute("UPDATE youtube_downloads SET unprotected_at=1", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let replaced = directory.join("media.mp4");
    tokio::fs::write(&replaced, b"a different replacement generation")
        .await
        .unwrap();
    assert!(cleanup(&state).await.is_err());
    assert!(
        replaced.exists(),
        "A replacement must not be deleted using old metadata"
    );
    let metadata = std::fs::metadata(&replaced).unwrap();
    state
        .db
        .call(move |db| {
            db.execute(
                "UPDATE youtube_downloads SET size=?1,modified=?2",
                params![
                    metadata.len() as i64,
                    metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)?
                        .as_nanos()
                        .to_string()
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    cleanup(&state).await.unwrap();
    assert!(!directory.exists());
    assert_eq!(
        state
            .db
            .call(|db| Ok(db.query_row("SELECT COUNT(*) FROM media", [], |r| r.get::<_, i64>(0))?))
            .await
            .unwrap(),
        0,
        "Online metadata never enters the local catalog"
    );
}
