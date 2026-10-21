use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use serde_json::json;

// Browser fixtures cannot validate state selection or per-user visibility.
#[tokio::test]
async fn activity_summary_tracks_real_operations_and_respects_audience() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.activity", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,created_at,updated_at) VALUES ('radarr','radarr','bob','installing',1,1)", [])?;
        db.execute("INSERT INTO tool_jobs(id,tool,action,manual,revision,stage,created_at,updated_at) VALUES ('tool','ffmpeg','install',1,0,'waiting',1,1)", [])?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Movies','movies','/movies')", [])?;
        db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('scan','library.scan','root','{\"root_id\":\"root\",\"secret\":\"PRIVATE-TOKEN\"}','running',1,1)", [])?;
        db.execute("INSERT INTO settings(key,value) VALUES ('product.controller','{\"items\":[{\"id\":\"product\",\"stage\":\"preparing\"}]}'),('backups.controller','{\"items\":[{\"id\":\"backup\",\"stage\":\"snapshotting\"}]}')", [])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice','private-video','Private video')", [])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES ('private-video','g','downloading','{}',1,1)", [])?;
        db.execute("INSERT INTO youtube_download_requests(user_id,video_id,requested_at) VALUES ('alice','private-video',1)", [])?;
        Ok(())
    }).await.unwrap();
    let admin =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={admin}");
    let member = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice).await;
    assert_eq!(member.0, StatusCode::OK);
    let member_activity = member.2["activity"].as_array().unwrap();
    assert_eq!(member_activity.len(), 1);
    assert_eq!(member_activity[0]["target"], "online");
    assert_eq!(member_activity[0]["resource"], "youtube");
    let result = call(&state, "/api/v1/me/attention", "GET", json!({}), &admin).await;
    assert_eq!(result.0, StatusCode::OK);
    let activity = result.2["activity"].as_array().unwrap();
    assert!(activity.iter().any(|v| v["target"] == "services"
        && v["resource"] == "radarr"
        && v["state"] == "running"));
    assert!(
        activity.iter().any(|v| v["target"] == "server"
            && v["resource"] == "ffmpeg"
            && v["state"] == "queued")
    );
    assert!(
        activity
            .iter()
            .any(|v| v["target"] == "movies" && v["resource"] == "root")
    );
    assert!(activity.iter().any(|v| v["target"] == "backups"));
    assert!(activity.iter().any(|v| v["id"] == "product:product"));
    assert!(!activity.iter().any(|v| v["target"] == "online"));
    assert!(!result.2.to_string().contains("PRIVATE-TOKEN"));
    state.db.write("test.activity_finish", |db| {
        db.execute("UPDATE stack_provisions SET state='ready'", [])?;
        db.execute("UPDATE tool_jobs SET stage='complete'", [])?;
        db.execute("UPDATE jobs SET state='complete'", [])?;
        db.execute("UPDATE settings SET value='{\"items\":[{\"id\":\"complete\",\"stage\":\"committed\"}]}' WHERE key='product.controller'", [])?;
        db.execute("UPDATE settings SET value='{\"items\":[{\"id\":\"complete\",\"stage\":\"complete\"}]}' WHERE key='backups.controller'", [])?;
        db.execute("UPDATE youtube_downloads SET state='ready'", [])?;
        Ok(())
    }).await.unwrap();
    for cookie in [&alice, &admin] {
        assert_eq!(
            call(&state, "/api/v1/me/attention", "GET", json!({}), cookie)
                .await
                .2["activity"],
            json!([])
        );
    }
}

// A running transfer is activity; a failed or suspended one stays an attention item
// until it is recovered.
#[tokio::test]
async fn transfers_are_activity_while_running_and_attention_until_recovered() {
    let (_temp, state, _alice) = fixture().await;
    let set = |value: &'static str| {
        let state = state.clone();
        async move {
            state
                .db
                .write("test.transfer_state", move |db| {
                    db.execute("UPDATE service_transfers SET state=?1", [value])?;
                    Ok(())
                })
                .await
                .unwrap();
        }
    };
    state.db.write("test.transfer_activity", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
        db.execute("INSERT INTO service_transfers(id,kind,service_id,actor_id,container_id,engine,connection_revision,review_revision,review,state,step,created_at,updated_at) VALUES ('move','radarr','service','bob','container','engine','revision','review','{}','running','copy',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let admin =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={admin}");
    let attention = || call(&state, "/api/v1/me/attention", "GET", json!({}), &admin);
    let running = attention().await.2;
    assert!(
        running["activity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == "transfer:move"
                && v["target"] == "services"
                && v["resource"] == "radarr"
                && v["state"] == "running")
    );
    for stuck in ["failed", "suspended"] {
        set(stuck).await;
        let held = attention().await.2;
        assert!(
            held["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == "transfer:move"),
            "{held}"
        );
        assert!(
            !held["activity"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == "transfer:move")
        );
    }
    set("complete").await;
    let done = attention().await.2;
    assert!(!done.to_string().contains("transfer:move"));
}

// UI fixtures cannot catch the storage limit or administrator visibility scope.
// Follow the public history/acknowledgment routes against the real database.
#[tokio::test]
async fn older_request_updates_remain_reachable_for_members_and_administrators() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.history", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)",[])?;
        for index in 0..201 {
            db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,'alice','lidarr','g',?1,?1,'denied',?2,1)",rusqlite::params![format!("album-{index:03}"),index])?;
        }
        Ok(())
    }).await.unwrap();
    for page in 1..=2 {
        let result = call(
            &state,
            &format!("/api/v1/acquisition/requests?page={page}"),
            "GET",
            json!({}),
            &alice,
        )
        .await
        .2;
        assert_eq!(result["pages"], 2);
        assert_eq!(result["page"], page);
        let rows = result["items"].as_array().unwrap();
        assert_eq!(rows.len(), if page == 1 { 200 } else { 1 });
        for row in rows {
            let reference = &row["attention"];
            assert_eq!(
                call(
                    &state,
                    &format!("/api/v1/me/attention/{}", reference["id"].as_str().unwrap()),
                    "PUT",
                    json!({"revision":reference["revision"]}),
                    &alice
                )
                .await
                .0,
                StatusCode::OK
            );
        }
    }
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
    state.db.write("test.admin_history", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'",[])?;
        db.execute("DELETE FROM acquisition_requests WHERE id<>'album-000'",[])?;
        db.execute("UPDATE acquisition_requests SET state='failed',updated_at=2 WHERE id='album-000'",[])?;
        for index in 1..=200 {
            db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,'bob','lidarr','g',?1,?1,'denied',?2,1)",rusqlite::params![format!("bob-{index:03}"),index])?;
        }
        Ok(())
    }).await.unwrap();
    let first = call(
        &state,
        "/api/v1/acquisition/requests?page=1",
        "GET",
        json!({}),
        &alice,
    )
    .await
    .2;
    assert!(
        first["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["user_id"] == "bob" && r["attention"].is_null())
    );
    let older = call(
        &state,
        "/api/v1/acquisition/requests?page=2",
        "GET",
        json!({}),
        &alice,
    )
    .await
    .2;
    let own = &older["items"][0];
    assert_eq!(own["id"], "album-000");
    assert_eq!(
        call(
            &state,
            "/api/v1/me/attention/request:album-000",
            "PUT",
            json!({"revision":own["attention"]["revision"]}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
}
#[tokio::test]
async fn attention_is_private_live_and_diagnostics_exclude_secret_material() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,completed_at,error,created_at) VALUES ('failed','metadata.match','media','{\"secret\":\"PRIVATE-TOKEN\"}','failed',1,?1,'https://private/?ApiKey=PRIVATE-TOKEN',1)",[thelxinoe_core::now()])?;Ok(())}).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    super::operations::observe(&state).await.unwrap();
    super::operations::observe(&state).await.unwrap();
    let notices = call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
        .await
        .2;
    assert_eq!(notices["items"].as_array().unwrap().len(), 1);
    assert!(!notices.to_string().contains("PRIVATE"));
    assert!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let key = notices["items"][0]["id"].as_str().unwrap();
    call(
        &state,
        &format!("/api/v1/me/attention/{key}"),
        "PUT",
        json!({"revision":notices["items"][0]["revision"]}),
        &alice,
    )
    .await;
    assert!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
            .await
            .2["items"][0]["dismissible"]
            == true
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/diagnostics",
            "GET",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let diagnostic = call(
        &state,
        "/api/v1/admin/diagnostics",
        "GET",
        json!({}),
        &admin,
    )
    .await
    .2;
    assert_eq!(diagnostic["database_ok"], true);
    assert!(!diagnostic.to_string().contains("PRIVATE"));
    assert!(!diagnostic.to_string().contains("bob"));
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE jobs SET state='complete' WHERE id='failed'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    super::operations::observe(&state).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE jobs SET state='failed' WHERE id='failed'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    super::operations::observe(&state).await.unwrap();
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn user_deletion_revokes_personal_state_and_preserves_shared_data() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;db.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES ('alice','youtube','g',1)",[])?;db.execute("INSERT INTO events(user_id,kind,payload,created_at) VALUES ('alice','private','{}',1)",[])?;db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at) VALUES ('service','radarr','alice',7878,X'00','active',1,1)",[])?;Ok(())}).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    assert_eq!(
        call(&state, "/api/v1/users/bob", "DELETE", json!({}), &alice)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/users/bob",
            "PUT",
            json!({"role":"user"}),
            &admin
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(&state, "/api/v1/users/alice", "DELETE", json!({}), &admin)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &alice)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    state.db.write("test.fixture", |db|{assert_eq!(db.query_row("SELECT (SELECT COUNT(*) FROM online_accounts)+(SELECT COUNT(*) FROM events WHERE user_id='alice')",[],|r|r.get::<_,i64>(0))?,0);assert_eq!(db.query_row("SELECT actor_id FROM stack_provisions",[],|r|r.get::<_,String>(0))?,"bob");assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);Ok(())}).await.unwrap();
}

// Browser fixtures cannot exercise owner checks or a changed revision inside the
// storage transaction. This API integration test covers those acknowledgment races.
#[tokio::test]
async fn request_attention_maps_album_ancestors_and_rejects_foreign_or_stale_acknowledgments() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/music')", [])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,metadata,created_at) VALUES ('artist','music','artist',NULL,'artist','Artist','{}',1),('album','music','album','artist','album','Album','{\"musicbrainz_release_group_id\":\"album-id\"}',1)", [])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)", [])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('request','alice','lidarr','g','album-id','Album','available',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
            .await
            .unwrap();
    let bob = format!("thelxinoe_session={token}");
    let list = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
        .await
        .2;
    let item = &list["items"][0];
    assert_eq!(item["target"], "music");
    assert_eq!(item["media_ids"], json!(["album", "artist"]));
    let path = format!("/api/v1/me/attention/{}", item["id"].as_str().unwrap());
    let revision = item["revision"].clone();
    assert_eq!(
        call(&state, &path, "PUT", json!({"revision":revision}), &bob)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    state
        .db
        .write("test.change", |db| {
            db.execute(
                "UPDATE acquisition_requests SET updated_at=2 WHERE id='request'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(&state, &path, "PUT", json!({"revision":revision}), &alice)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let new = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
        .await
        .2;
    assert_eq!(
        call(
            &state,
            &path,
            "PUT",
            json!({"revision":new["items"][0]["revision"]}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
}

// Clients no longer poll attention. A committed domain change must push
// attention.changed to exactly the users whose summary changed.
#[tokio::test]
async fn domain_events_push_attention_to_affected_users() {
    let (_temp, state, _alice) = fixture().await;
    super::operations::observe(&state).await.unwrap();
    let cursor = state
        .db
        .read("test.cursor", |db| {
            Ok(
                db.query_row("SELECT COALESCE(MAX(id),0) FROM events", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await
        .unwrap();
    let watcher = tokio::spawn(super::operations::watch(state.clone()));
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)", [])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('request','alice','lidarr','g','album-id','Album','denied',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    state.emit(None, "jobs.changed", json!({})).await.unwrap();
    let pushed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let users = state
                .db
                .read("test.events", move |db| {
                    Ok(db
                        .prepare(
                            "SELECT user_id FROM events WHERE id>?1 AND kind='attention.changed'",
                        )?
                        .query_map([cursor], |r| r.get::<_, Option<String>>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?)
                })
                .await
                .unwrap();
            if !users.is_empty() {
                return users;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    watcher.abort();
    assert_eq!(pushed, vec![Some("alice".to_owned())]);
}

// Attention is observed every few seconds and after every change. Observing an
// unchanged summary must not take the writer, and a change pushes only to its user.
#[tokio::test]
async fn attention_observation_writes_only_changes_for_affected_users() {
    let (_temp, state, _alice) = fixture().await;
    let pushed = |state: crate::AppState| async move {
        state
            .db
            .read("test.events", |db| {
                Ok(db
                    .prepare(
                        "SELECT user_id FROM events WHERE kind='attention.changed' ORDER BY id",
                    )?
                    .query_map([], |r| r.get::<_, Option<String>>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap()
    };
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)", [])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('alice-request','alice','lidarr','g','album-a','Album A','denied',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    // After a start, clients refresh on reconnect; the first observation only records.
    super::operations::observe(&state).await.unwrap();
    assert!(pushed(state.clone()).await.is_empty());
    let writes = state.db.metrics().writes.completed;
    super::operations::observe(&state).await.unwrap();
    assert_eq!(state.db.metrics().writes.completed, writes);
    assert!(pushed(state.clone()).await.is_empty());
    state.db.write("test.change", |db| {
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('bob-request','bob','lidarr','g','album-b','Album B','failed',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    super::operations::observe(&state).await.unwrap();
    assert_eq!(pushed(state.clone()).await, vec![Some("bob".to_owned())]);
    super::operations::observe(&state).await.unwrap();
    assert_eq!(pushed(state.clone()).await, vec![Some("bob".to_owned())]);
}

// Playback adds and removes cache files while the overview measures them.
#[test]
fn folder_size_counts_files_and_tolerates_vanished_entries() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a"), [0u8; 10]).unwrap();
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    std::fs::write(temp.path().join("nested/b"), [0u8; 5]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path().join("a"), temp.path().join("link")).unwrap();
    assert_eq!(super::operations::folder_size(temp.path()), (15, false));
    assert_eq!(
        super::operations::folder_size(&temp.path().join("removed")),
        (0, false)
    );
}

#[tokio::test]
async fn administration_overview_reuses_a_recent_folder_walk() {
    let (_temp, state, _) = fixture().await;
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let admin =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={admin}");
    let first = call(&state, "/api/v1/admin/operations", "GET", json!({}), &admin).await;
    assert_eq!(first.0, StatusCode::OK);
    std::fs::write(state.config.cache.join("segment"), [0u8; 4096]).unwrap();
    let second = call(&state, "/api/v1/admin/operations", "GET", json!({}), &admin).await;
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(
        first.2["storage"]["cache"]["bytes"],
        second.2["storage"]["cache"]["bytes"]
    );
    assert!(second.2["storage"]["cache"]["free_bytes"].as_u64().unwrap() > 0);
}
