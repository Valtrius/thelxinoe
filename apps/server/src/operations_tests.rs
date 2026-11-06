use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use serde_json::{Value, json};

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
        db.execute("INSERT INTO twitch_attempts(user_id,session_id,generation,client_hash,device,expires_at,next_poll,interval,phase) SELECT user_id,id,'g','hash',x'00',?1,0,5,'connecting' FROM sessions WHERE user_id='alice' LIMIT 1", [thelxinoe_core::now() + 600])?;
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
    assert_eq!(member_activity[0]["resource"], "twitch");
    assert_eq!(member_activity[0]["state"], "running");
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
        db.execute("DELETE FROM twitch_attempts", [])?;
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
async fn jobs_page(state: &crate::AppState, cookie: &str, query: &str) -> Value {
    let (status, _, body) = call(
        state,
        &format!("/api/v1/admin/jobs{query}"),
        "GET",
        json!({}),
        cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
fn job_ids(jobs: &Value) -> Vec<String> {
    jobs.as_array()
        .unwrap()
        .iter()
        .map(|job| job["id"].as_str().unwrap().to_owned())
        .collect()
}
#[tokio::test]
async fn job_list_names_targets_and_triggers_without_exposing_payloads() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.jobs", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('films','Movies','movies','/media/movies')", [])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES ('film','films','movie','film','film.2020','{\"title\":\"The Film\"}',1)", [])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,created_at,updated_at) VALUES ('provision','sonarr','alice','complete',1,1)", [])?;
        db.execute("INSERT INTO service_updates(id,service_id,actor_id,state,created_at,updated_at,automatic) VALUES ('update','provision',NULL,'committed',1,1,1)", [])?;
        for (id, kind, subject, payload, state, started, completed, error, created) in [
            ("scan", "library.scan", "films", "{\"root_id\":\"films\"}", "complete", Some(10), Some(18), None, 9),
            ("removed", "library.scan", "gone", "{\"root_id\":\"gone\"}", "complete", Some(8), Some(8), None, 8),
            ("mine", "metadata.refresh", "film", "{\"media_id\":\"film\",\"actor_id\":\"alice\",\"secret\":\"PRIVATE-TOKEN\"}", "failed", Some(7), Some(7), Some("No match"), 7),
            ("deleted-user", "metadata.refresh", "film", "{\"media_id\":\"film\",\"actor_id\":\"removed\"}", "complete", Some(6), Some(6), None, 6),
            ("automatic", "metadata.refresh", "film", "{\"media_id\":\"film\"}", "complete", Some(5), Some(5), None, 5),
            ("sync", "recyclarr.sync", "gone-run", "{\"id\":\"gone-run\",\"preview\":true,\"automatic\":true,\"files\":\"PRIVATE-TOKEN\"}", "complete", Some(4), Some(4), None, 4),
            ("install", "stack.install", "provision", "{\"id\":\"provision\",\"actor_id\":\"bob\"}", "complete", Some(2), Some(2), None, 2),
            ("old-install", "stack.install", "provision", "{\"id\":\"provision\"}", "complete", Some(1), Some(1), None, 1),
            ("check", "service.update", "update", "{\"id\":\"update\",\"action\":\"preflight\",\"automatic\":true}", "queued", None, None, None, 3),
        ] {
            db.execute(
                "INSERT INTO jobs(id,kind,subject,payload,state,available_at,started_at,completed_at,error,created_at) VALUES (?1,?2,?3,?4,?5,20,?6,?7,?8,?9)",
                rusqlite::params![id, kind, subject, payload, state, started, completed, error, created],
            )?;
        }
        Ok(())
    }).await.unwrap();
    let page = jobs_page(&state, &alice, "").await;
    assert!(!page.to_string().contains("PRIVATE-TOKEN"));
    let job = |id: &str| {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == id)
            .unwrap()
            .clone()
    };
    assert!(job("scan").get("payload").is_none());
    let scan = job("scan");
    assert_eq!(
        (
            &scan["target"],
            &scan["created_at"],
            &scan["available_at"],
            &scan["started_at"],
            &scan["completed_at"],
            &scan["automatic"]
        ),
        (
            &json!("Movies"),
            &json!(9),
            &json!(20),
            &json!(10),
            &json!(18),
            &Value::Null
        )
    );
    assert!(job("removed")["target"].is_null());
    let mine = job("mine");
    assert_eq!(
        (&mine["target"], &mine["automatic"], &mine["actor"]),
        (&json!("The Film"), &json!(false), &json!("alice"))
    );
    let deleted = job("deleted-user");
    assert_eq!(
        (&deleted["automatic"], &deleted["actor"]),
        (&json!(false), &Value::Null)
    );
    assert_eq!(job("automatic")["automatic"], true);
    let sync = job("sync");
    assert_eq!(
        (&sync["service"], &sync["action"], &sync["automatic"]),
        (&json!("recyclarr"), &json!("preview"), &json!(true))
    );
    let check = job("check");
    assert_eq!(
        (&check["service"], &check["action"], &check["automatic"]),
        (&json!("sonarr"), &json!("preflight"), &json!(true))
    );
    // Whoever owns the service now, an installation names who asked for it, if that was recorded.
    let install = job("install");
    assert_eq!(
        (
            &install["service"],
            &install["automatic"],
            &install["actor"]
        ),
        (&json!("sonarr"), &json!(false), &json!("bob"))
    );
    let old_install = job("old-install");
    assert_eq!(
        (&old_install["automatic"], &old_install["actor"]),
        (&Value::Null, &Value::Null)
    );
    // A job still in progress stays above the history whatever followed it.
    assert_eq!(job_ids(&page["pinned"]), ["check"]);
}
#[tokio::test]
async fn job_list_filters_counts_and_pages() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.jobs", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        for (id, kind, payload, state, created) in [
            ("running-scan", "library.scan", "{}", "running", 50),
            ("automatic", "metadata.refresh", "{}", "complete", 40),
            ("automatic-failed", "metadata.refresh", "{}", "failed", 30),
            ("automatic-queued", "metadata.refresh", "{}", "queued", 20),
            ("requested", "metadata.refresh", "{\"actor_id\":\"alice\"}", "complete", 10),
        ] {
            db.execute(
                "INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES (?1,?2,'subject',?3,?4,1,?5)",
                rusqlite::params![id, kind, payload, state, created],
            )?;
        }
        for index in 0..101 {
            db.execute(
                "INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES (?1,'recyclarr.sync','run','{}','complete',1,5)",
                [format!("sync-{index:03}")],
            )?;
        }
        Ok(())
    }).await.unwrap();

    let all = jobs_page(&state, &alice, "").await;
    assert_eq!(
        all["totals"],
        json!({"running":1,"queued":1,"failed":1,"complete":103})
    );
    assert_eq!(
        job_ids(&all["pinned"]),
        ["running-scan", "automatic-queued"]
    );

    // Automatic refreshes hide unless they failed, in the rows, the pinned jobs and the totals.
    let hidden = jobs_page(&state, &alice, "?hide_automatic=true&kind=metadata.refresh").await;
    assert_eq!(job_ids(&hidden["items"]), ["automatic-failed", "requested"]);
    assert!(job_ids(&hidden["pinned"]).is_empty());
    assert_eq!(
        hidden["totals"],
        json!({"running":0,"queued":0,"failed":1,"complete":1})
    );

    // A state narrows the rows but not the totals, which still count what each state lists.
    let completed = jobs_page(
        &state,
        &alice,
        "?state=complete&kind=metadata.refresh&hide_automatic=false",
    )
    .await;
    assert_eq!(job_ids(&completed["items"]), ["automatic", "requested"]);
    assert!(job_ids(&completed["pinned"]).is_empty());
    assert_eq!(completed["totals"]["running"], 0);
    assert_eq!(completed["totals"]["failed"], 1);

    // A longer list starts with the shorter one, even for jobs created in the same second.
    let synced = job_ids(&jobs_page(&state, &alice, "?kind=recyclarr.sync").await["items"]);
    assert_eq!(synced.len(), 100);
    let longer =
        job_ids(&jobs_page(&state, &alice, "?kind=recyclarr.sync&limit=200").await["items"]);
    assert_eq!(longer.len(), 101);
    assert_eq!(longer[..100], synced);

    // Everything since a time stays listed, past the limit and past the default page.
    let since = jobs_page(&state, &alice, "?kind=metadata.refresh&limit=1&since=20").await;
    assert_eq!(
        job_ids(&since["items"]),
        ["automatic", "automatic-failed", "automatic-queued"]
    );
    let since = jobs_page(&state, &alice, "?kind=recyclarr.sync&since=5").await;
    assert_eq!(job_ids(&since["items"]).len(), 101);

    let invalid = call(
        &state,
        "/api/v1/admin/jobs?limit=nonsense",
        "GET",
        json!({}),
        &alice,
    )
    .await;
    assert_eq!(invalid.0, StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn job_list_pins_failures_awaiting_dismissal_beyond_the_latest_jobs() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.jobs", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,completed_at,error,created_at) VALUES ('failed','service.update','update','{}','failed',1,?1,'Image registry rate limit exceeded',1)", [thelxinoe_core::now()])?;
        for index in 0..100 {
            db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,completed_at,created_at) VALUES (?1,'metadata.refresh','media','{}','complete',1,1,?2)", rusqlite::params![format!("refresh-{index}"), 2 + index])?;
        }
        Ok(())
    }).await.unwrap();
    let page = jobs_page(&state, &alice, "").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 100);
    assert!(!job_ids(&page["items"]).contains(&"failed".to_owned()));
    assert_eq!(job_ids(&page["pinned"]), ["failed"]);
    let notices = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
        .await
        .2;
    let revision = notices["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "job:failed")
        .unwrap()["revision"]
        .clone();
    let dismissed = call(
        &state,
        "/api/v1/me/attention/job:failed",
        "PUT",
        json!({ "revision": revision }),
        &alice,
    )
    .await;
    assert_eq!(dismissed.0, StatusCode::OK, "{}", dismissed.2);
    assert!(job_ids(&jobs_page(&state, &alice, "").await["pinned"]).is_empty());
}
#[tokio::test]
async fn job_list_pins_every_failure_and_keeps_running_jobs_within_the_cap_on_active_ones() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.jobs", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        for index in 0..101 {
            db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,completed_at,error,created_at) VALUES (?1,'library.scan','root','{}','failed',1,?2,'Unreadable',?3)", rusqlite::params![format!("failed-{index:03}"), thelxinoe_core::now(), 1000 + index])?;
            db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES (?1,'library.scan','root','{}','queued',1,?2)", rusqlite::params![format!("queued-{index:03}"), 2000 + index])?;
        }
        db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('running','library.scan','root','{}','running',1,1)", [])?;
        Ok(())
    }).await.unwrap();

    let pinned = job_ids(&jobs_page(&state, &alice, "").await["pinned"]);
    assert_eq!(pinned.len(), 101 + 100);
    assert_eq!(pinned[0], "failed-100");
    assert_eq!(pinned[100], "failed-000");
    assert_eq!(pinned[101], "running");
    assert_eq!(pinned[102], "queued-100");
    assert_eq!(pinned[200], "queued-002");

    // A queued job past the cap is still listed.
    let queued = jobs_page(&state, &alice, "?state=queued&limit=200").await;
    assert_eq!(job_ids(&queued["items"]).len(), 101);
    assert_eq!(job_ids(&queued["pinned"]).len(), 100);
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
