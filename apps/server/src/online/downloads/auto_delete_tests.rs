use super::*;

async fn seed(state: &AppState, video: &'static str, size: usize, age: i64) -> PathBuf {
    let generation = thelxinoe_core::id();
    let directory = state
        .config
        .cache
        .join("youtube")
        .join(video)
        .join(&generation);
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("media.mp4");
    std::fs::write(&path, vec![b'x'; size]).unwrap();
    let path = path.canonicalize().unwrap();
    let stored = path.clone();
    let metadata = std::fs::metadata(&path).unwrap();
    state.db.write("test.download", move |db| {
        db.execute("INSERT INTO youtube_media(video_id) VALUES (?1)", [video])?;
        for user in ["alice", "bob"] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy,metadata_at) VALUES (?1,?2,?2,'public',1)", params![user,video])?;
            let list = crate::online::watchlists::default_list(db,user)?;
            db.execute("INSERT INTO youtube_watchlist_items VALUES (?1,?2,?3,0,1)",params![user,list,video])?;
        }
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,path,size,modified,probe,requested_at,updated_at,completed_at) VALUES (?1,?2,'ready','{}',?3,?4,?5,?6,1,?7,?7)",params![video,generation,stored.to_string_lossy(),size as i64,metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string(),json!({"format":{"duration":"100"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264"}]}).to_string(),age])?;
        Ok(())
    }).await.unwrap();
    path
}

/// Sets the unpinned limit below the smallest one the API accepts, for byte-sized fixtures.
async fn limit(state: &AppState, bytes: i64) {
    state
        .db
        .write("test.limit", move |db| {
            db.execute(
                "UPDATE retention_policies SET storage_limit_bytes=?1 WHERE domain='videos'",
                [bytes],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn api(state: &AppState, cookie: &str, path: &str, method: &str, body: Value) -> Value {
    let response = call(state, path, method, body, cookie).await;
    assert_eq!(response.0, StatusCode::OK, "{path}: {}", response.2);
    response.2
}

#[tokio::test]
async fn auto_delete_shared_downloads_respect_pins_capacity_watched_users_and_file_ownership() {
    let (_temp, state, cookie) = fixture().await;
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let settings = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    let mut policy = settings["policies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["domain"] == "videos")
        .expect("video deletion belongs in the shared settings")
        .clone();
    assert_eq!(policy["storage_limit_bytes"], 100_000_000_000_i64);
    assert_eq!(policy["enabled"], false);
    let pinned = seed(&state, "pinned00001", 100, 1).await;
    let oldest = seed(&state, "oldvideo001", 4, 2).await;
    let newest = seed(&state, "newvideo001", 4, 3).await;
    let manual = oldest.with_file_name("manual.txt");
    std::fs::write(&manual, b"manually placed sidecar").unwrap();
    api(
        &state,
        &cookie,
        "/api/v1/online/youtube/videos/pinned00001",
        "PUT",
        json!({"pinned":true}),
    )
    .await;
    policy["storage_limit_bytes"] = json!(39_999_999_999_i64);
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/retention/policy/videos",
            "POST",
            policy.clone(),
            &cookie
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "the unpinned limit holds at least 40 GB"
    );
    policy["storage_limit_bytes"] = json!(100_000_000_000_i64);
    limit(&state, 4).await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(pinned.exists(), "pins do not consume the unpinned limit");
    assert!(
        !oldest.exists(),
        "oldest saved but unwatched download is evicted"
    );
    assert!(newest.exists());
    assert!(
        manual.exists(),
        "unrecorded files in an owned directory stay untouched"
    );
    let capacity = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    assert_eq!(capacity["video_usage"]["unpinned_bytes"], 4);
    assert_eq!(capacity["video_usage"]["pinned_bytes"], 100);
    assert!(
        capacity["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["video_id"] == "oldvideo001"
                && i["reason"] == "storage_limit"
                && i["state"] == "complete")
    );
    state.db.read("test.preserved_lists",|db| {
        assert_eq!(db.query_row("SELECT COUNT(*) FROM youtube_watchlist_items WHERE video_id='oldvideo001'",[],|r|r.get::<_,i64>(0))?,2);
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM youtube_download_suppressed WHERE video_id='oldvideo001')",[],|r|r.get::<_,bool>(0))?);
        Ok(())
    }).await.unwrap();

    policy["enabled"] = json!(true);
    policy["grace_seconds"] = json!(86400);
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/policy/videos",
        "POST",
        policy.clone(),
    )
    .await;
    limit(&state, 1000).await;
    api(
        &state,
        &cookie,
        "/api/v1/online/youtube/videos/newvideo001",
        "PUT",
        json!({"watched":true}),
    )
    .await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    let one = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    assert!(
        !one["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["video_id"] == "newvideo001" && i["state"] == "pending")
    );
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
            .await
            .unwrap();
    let bob = format!("thelxinoe_session={token}");
    api(
        &state,
        &bob,
        "/api/v1/online/youtube/videos/newvideo001",
        "PUT",
        json!({"watched":true}),
    )
    .await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    let both = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    let candidate = both["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["video_id"] == "newvideo001" && i["state"] == "pending")
        .unwrap();
    assert!(
        candidate["due_at"].as_i64().unwrap() >= now() + 86395,
        "old watches get a fresh countdown"
    );
    let candidate_id = candidate["id"].as_str().unwrap().to_owned();
    policy["enabled"] = json!(false);
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/policy/videos",
        "POST",
        policy.clone(),
    )
    .await;
    let off = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    assert!(
        !off["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["state"] == "pending" && i["reason"] == "watched")
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/retention/{candidate_id}/delete"),
            "POST",
            json!({}),
            &cookie
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    policy["enabled"] = json!(true);
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/policy/videos",
        "POST",
        policy.clone(),
    )
    .await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    let again = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    let candidate = again["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["video_id"] == "newvideo001" && i["state"] == "pending")
        .unwrap();
    assert_ne!(candidate["id"], candidate_id);
    api(
        &state,
        &cookie,
        &format!(
            "/api/v1/admin/retention/{}/keep",
            candidate["id"].as_str().unwrap()
        ),
        "POST",
        json!({}),
    )
    .await;
    limit(&state, 1).await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(
        newest.exists(),
        "Keep protects against both watched and capacity deletion"
    );

    let playing = seed(&state, "playing0001", 4, 4).await;
    let playback=api(&state,&bob,"/api/v1/playback","POST",json!({"media_id":"youtube:playing0001","options":{"quality":"auto","capabilities":{"containers":["mp4"],"video":["h264"],"audio":[],"hls":false}}})).await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(
        playing.exists(),
        "active playback is never evicted to satisfy the cap"
    );
    let waiting = seed(&state, "waiting0001", 1, 6).await;
    std::fs::remove_file(&waiting).unwrap();
    state.db.write("test.queued_download", |db| {
        db.execute("DELETE FROM youtube_watchlist_items WHERE video_id='waiting0001'", [])?;
        db.execute("INSERT INTO youtube_download_requests VALUES ('alice','waiting0001',1)", [])?;
        db.execute("UPDATE youtube_downloads SET state='queued',path=NULL,size=NULL,modified=NULL,probe=NULL,completed_at=NULL WHERE video_id='waiting0001'", [])?;
        Ok(())
    }).await.unwrap();
    let (waiting_id, waiting_generation, ..) = storage::claim_download(&state.db, Vec::new(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        waiting_id, "waiting0001",
        "An explicit download does not require a pin or watchlist"
    );
    assert!(
        !crate::auto_delete::admit(&state, &waiting_id, &waiting_generation)
            .await
            .unwrap(),
        "Protected playback makes the new download wait before starting a process"
    );
    storage::finish_download(
        "waiting_for_space",
        None,
        waiting_id.clone(),
        waiting_generation.clone(),
        None,
        &state.db,
    )
    .await
    .unwrap();
    let waiting_status = api(
        &state,
        &cookie,
        "/api/v1/online/youtube/videos/waiting0001/download",
        "GET",
        json!({}),
    )
    .await;
    assert_eq!(waiting_status["download"]["state"], "waiting_for_space");
    let waiting_settings = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    assert_eq!(waiting_settings["video_usage"]["waiting"], 1);
    api(
        &state,
        &bob,
        &format!(
            "/api/v1/playback/{}/progress",
            playback["id"].as_str().unwrap()
        ),
        "POST",
        json!({"sequence":1,"position":10,"state":"stopped"}),
    )
    .await;
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(!playing.exists());
    let (_, generation, _, waiting, _) = storage::claim_download(&state.db, Vec::new(), None)
        .await
        .unwrap()
        .unwrap();
    assert!(waiting);
    assert_eq!(generation, waiting_generation);
    assert!(
        crate::auto_delete::admit(&state, &waiting_id, &generation)
            .await
            .unwrap(),
        "The waiting download can resume once playback releases capacity"
    );
    api(
        &state,
        &cookie,
        "/api/v1/online/youtube/videos/waiting0001/download",
        "DELETE",
        json!({}),
    )
    .await;
    let changed = seed(&state, "changed0001", 4, 5).await;
    std::fs::write(&changed, b"a manually replaced file").unwrap();
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(
        changed.exists(),
        "changed file identities are blocked, never retried destructively"
    );
    limit(&state, 1000).await;
    let interrupted = seed(&state, "interrupted", 4, 7).await;
    for viewer in [&cookie, &bob] {
        api(
            &state,
            viewer,
            "/api/v1/online/youtube/videos/interrupted",
            "PUT",
            json!({"watched":true}),
        )
        .await;
    }
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    state.db.write("test.interrupted_deletion", |db| {
        // Simulate a restart after a cap deletion recorded its destructive intent.
        db.execute("UPDATE video_retention_candidates SET due_at=1 WHERE video_id='interrupted' AND state='pending'", [])?;
        let inserted=db.execute("INSERT INTO video_retention_candidates(id,video_id,generation,stamp,title,reason,state,eligible_at,due_at) SELECT 'interrupted-attempt',video_id,generation,stamp,title,'storage_limit','executing',1,1 FROM video_retention_candidates WHERE video_id='interrupted' AND state='pending'", [])?;
        assert_eq!(inserted,1);
        Ok(())
    }).await.unwrap();
    api(
        &state,
        &cookie,
        "/api/v1/admin/retention/evaluate",
        "POST",
        json!({}),
    )
    .await;
    assert!(
        interrupted.exists(),
        "Recovery must not retry the same generation through its watched countdown"
    );
    let final_state = api(&state, &cookie, "/api/v1/admin/retention", "GET", json!({})).await;
    assert!(
        !final_state["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["video_id"] == "interrupted" && i["state"] == "pending")
    );
    assert!(
        final_state["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["video_id"] == "changed0001" && i["state"] == "blocked")
    );
    let evidence =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/thelxinoe-auto-delete");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("result.json"),serde_json::to_vec_pretty(&json!({"passed":true,"capacity":capacity,"final":final_state,"manual_file_preserved":manual.exists()})).unwrap()).unwrap();
}

// A ready download whose copy went missing or changed no longer gets stuck: playback falls
// back to streaming, a missing copy can be removed, and a changed file is never deleted.
#[tokio::test]
async fn downloads_missing_or_changed_on_disk_stop_claiming_their_rows() {
    let (_temp, state, _cookie) = fixture().await;
    let missing = seed(&state, "missing0001", 10, 1).await;
    let changed = seed(&state, "changed0001", 10, 2).await;
    let removed = seed(&state, "removed0001", 10, 3).await;
    std::fs::remove_file(&missing).unwrap();
    std::fs::write(&changed, b"replaced by someone else").unwrap();
    std::fs::remove_dir_all(removed.parent().unwrap()).unwrap();
    for video in ["missing0001", "changed0001"] {
        let error = crate::online::downloads::source(&state, video)
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT, "{video}");
    }
    assert!(changed.exists());
    let root = state.config.cache.join("youtube");
    let states = state
        .db
        .write("test.remove", move |db| {
            crate::auto_delete::remove_ready(db, &root, "removed0001")?;
            Ok(db
                .prepare("SELECT video_id,state FROM youtube_downloads ORDER BY video_id")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(
        states,
        [
            ("changed0001".to_owned(), "failed".to_owned()),
            ("missing0001".to_owned(), "failed".to_owned()),
            ("removed0001".to_owned(), "ready".to_owned()),
        ]
    );
}

// A download waiting for space is tried again when the capacity it waits on changes, or
// after a while, not on every pass; and a retry that still waits announces nothing.
#[tokio::test]
async fn downloads_waiting_for_space_are_retried_when_capacity_changes() {
    let (_temp, state, _cookie) = fixture().await;
    seed(&state, "playing0001", 10, 1).await;
    state.db.write("test.waiting", |db| {
        db.execute("UPDATE retention_policies SET storage_limit_bytes=10 WHERE domain='videos'", [])?;
        // Playback keeps the only ready download from eviction, so nothing fits.
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id) SELECT 'playing','alice',id,'g','public','playing','direct','{}',100,?1,?1,'playing0001' FROM sessions WHERE user_id='alice' LIMIT 1", [now()])?;
        db.execute("INSERT INTO youtube_media(video_id) VALUES ('waiting0001')", [])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('alice','waiting0001','Waiting','public')", [])?;
        db.execute("INSERT INTO youtube_download_requests VALUES ('alice','waiting0001',1)", [])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES ('waiting0001','g','queued','{}',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    // The download's state, whether it was claimed since `mark`, and the events so far.
    let observe = async || {
        state
            .db
            .read("test.observe", |db| {
                Ok((
                    db.query_row(
                        "SELECT state,updated_at>1 FROM youtube_downloads WHERE video_id='waiting0001'",
                        [],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
                    )?,
                    db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
                ))
            })
            .await
            .unwrap()
    };
    let mark = async || {
        state
            .db
            .write("test.mark", |db| {
                db.execute("UPDATE youtube_downloads SET updated_at=1", [])?;
                Ok(())
            })
            .await
            .unwrap();
    };
    let mut parked = Parked::new();
    next(&state, &mut parked).await.unwrap();
    let (_, events) = observe().await;
    assert_eq!(observe().await.0, ("waiting_for_space".into(), true));
    mark().await;
    for _ in 0..3 {
        next(&state, &mut parked).await.unwrap();
    }
    assert_eq!(
        observe().await,
        (("waiting_for_space".into(), false), events),
        "Unchanged capacity leaves a waiting download alone"
    );
    for (_, until) in parked.values_mut() {
        *until = Instant::now();
    }
    next(&state, &mut parked).await.unwrap();
    assert_eq!(
        observe().await,
        (("waiting_for_space".into(), true), events),
        "A late retry that still waits announces nothing"
    );
    mark().await;
    state
        .db
        .write("test.stop", |db| {
            db.execute("UPDATE playback_sessions SET state='stopped'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    next(&state, &mut parked).await.unwrap();
    let ((status, claimed), after) = observe().await;
    // Admitted once playback stops protecting the other file; the fixture's empty tool
    // bundle then fails the download itself.
    assert!(claimed && status == "failed" && after > events, "{status}");
}

// A finished download makes room by evicting older unpinned downloads, never itself. When
// playback keeps too much space for it, nothing is evicted; a pinned one always fits.
#[tokio::test]
async fn finished_downloads_evict_older_ones_to_fit_the_limit() {
    let (_temp, state, _cookie) = fixture().await;
    let oldest = seed(&state, "oldvideo001", 6, 1).await;
    let playing = seed(&state, "playing0001", 6, 2).await;
    limit(&state, 20).await;
    state.db.write("test.room", |db| {
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,generation,edition,state,mode,options,duration,created_at,updated_at,youtube_video_id) SELECT 'playing','alice',id,'g','public','playing','direct','{}',100,?1,?1,'playing0001' FROM sessions WHERE user_id='alice' LIMIT 1", [now()])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title,privacy) VALUES ('alice','pinned00001','Pinned','public')", [])?;
        db.execute("INSERT INTO youtube_state(user_id,video_id,pinned,updated_at) VALUES ('alice','pinned00001',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(
        crate::auto_delete::budget(&state.db, "newvideo001".into())
            .await
            .unwrap(),
        Some(14),
        "only the download being played can't make room"
    );
    assert_eq!(
        crate::auto_delete::budget(&state.db, "pinned00001".into())
            .await
            .unwrap(),
        None
    );
    let room = async |video: &'static str, size: i64| {
        let root = state.config.cache.join("youtube");
        state
            .db
            .write("test.make_room", move |db| {
                crate::auto_delete::make_room(db, &root, video, size)
            })
            .await
            .unwrap()
    };
    assert!(!room("newvideo001", 15).await);
    assert!(
        oldest.exists(),
        "nothing is evicted for a download that can't fit"
    );
    assert!(room("pinned00001", 1_000).await);
    assert!(oldest.exists(), "pinned downloads are outside the limit");
    assert!(room("newvideo001", 12).await);
    assert!(!oldest.exists());
    assert!(playing.exists());
}
