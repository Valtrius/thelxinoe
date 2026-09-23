use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
use chrono::Timelike;

async fn managed(state: &AppState, mode: &str) -> String {
    let key = id();
    let insert = key.clone();
    let mode = mode.to_owned();
    // Keep the window several hours away even if this test crosses an hour.
    let start = (chrono::Utc::now().hour() + 6) % 24;
    let end = (start + 1) % 24;
    state.db.write("test.update_fixture", move |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,created_at,updated_at) VALUES (?1,'radarr','alice',17878,X'01','complete','old','integration',1,1)", [&insert])?;
        db.execute("INSERT INTO service_update_policy(service_id,policy,window_start,window_end) VALUES (?1,?2,?3,?4)", params![insert,mode,start,end])?;
        Ok(())
    }).await.unwrap();
    state.managers.docker.lock().unwrap().extend([
        (
            "stack".into(),
            json!({"items":[{"id":key,"kind":"radarr","image":"repository@old"}]}),
        ),
        (
            "stack/releases".into(),
            json!({"items":[{"kind":"radarr","image":"repository@new"}]}),
        ),
    ]);
    assert!(
        !crate::timezones::in_server_window(state, start, end)
            .await
            .unwrap()
    );
    key
}

async fn allow_maintenance_now(state: &AppState) {
    let hour = chrono::Utc::now().hour();
    let start = (hour + 23) % 24;
    let end = (hour + 2) % 24;
    state
        .db
        .write("test.allow_maintenance", move |db| {
            db.execute(
                "UPDATE service_update_policy SET window_start=?1,window_end=?2",
                params![start, end],
            )?;
            db.execute(
                "INSERT INTO settings(key,value) VALUES ('product.policy',?1)",
                [json!({"policy":"notify","window_start":start,"window_end":end}).to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn notify_and_inherited_notify_discover_without_installing_and_observe_check_interval() {
    for mode in ["notify", "inherit"] {
        let (_temp, state, _cookie) = fixture().await;
        managed(&state, mode).await;
        allow_maintenance_now(&state).await;
        check_releases(&state, false).await.unwrap();
        schedule_updates(&state).await.unwrap();
        let value = storage::list(&state.db).await.unwrap();
        assert_eq!(value["policies"][0]["candidate"], "repository@new");
        assert!(value["policies"][0]["checked_at"].as_i64().unwrap() > 0);
        assert_eq!(value["items"], json!([]));
        state.managers.docker.lock().unwrap().insert(
            "stack/releases".into(),
            json!({"items":[{"kind":"radarr","image":"repository@newer"}]}),
        );
        check_releases(&state, false).await.unwrap();
        assert_eq!(
            storage::list(&state.db).await.unwrap()["policies"][0]["candidate"],
            "repository@new"
        );
        state
            .db
            .write("test.check_due", |db| {
                db.execute(
                    "UPDATE service_update_policy SET checked_at=?1",
                    [now() - 21601],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        check_releases(&state, false).await.unwrap();
        assert_eq!(
            storage::list(&state.db).await.unwrap()["policies"][0]["candidate"],
            "repository@newer"
        );
    }
}

#[tokio::test]
async fn automatic_scheduling_uses_cached_discovery_at_window_open_and_worker_rechecks_policy() {
    let (_temp, state, _cookie) = fixture().await;
    let service = managed(&state, "automatic").await;
    check_releases(&state, false).await.unwrap();
    schedule_updates(&state).await.unwrap();
    let queue = thelxinoe_jobs::Queue(state.db.clone());
    assert!(queue.claim_services().await.unwrap().is_none());
    let open_window = || {
        let hour = chrono::Utc::now().hour();
        ((hour + 23) % 24, (hour + 2) % 24)
    };
    let (start, end) = open_window();
    state
        .db
        .write("test.open_window", move |db| {
            db.execute(
                "UPDATE service_update_policy SET window_start=?1,window_end=?2",
                params![start, end],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // No new registry scan is due. The minute scheduler uses the cached image.
    schedule_updates(&state).await.unwrap();
    let job = queue.claim_services().await.unwrap().unwrap();
    let start = (chrono::Utc::now().hour() + 6) % 24;
    let end = (start + 1) % 24;
    state
        .db
        .write("test.close_window", move |db| {
            db.execute(
                "UPDATE service_update_policy SET window_start=?1,window_end=?2",
                params![start, end],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(!run_job(&state, &job).await.unwrap());
    let value = storage::list(&state.db).await.unwrap();
    assert_eq!(value["items"][0]["state"], "queued");
    assert!(
        value["items"][0]["error"]
            .as_str()
            .unwrap()
            .contains("maintenance window")
    );
    progress(
        &state,
        job.payload["id"].as_str().unwrap(),
        "ready",
        Some("repository@new".into()),
        None,
    )
    .await
    .unwrap();
    let (start, end) = open_window();
    state
        .db
        .write("test.reopen_window", move |db| {
            db.execute(
                "UPDATE service_update_policy SET window_start=?1,window_end=?2",
                params![start, end],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    check_releases(&state, true).await.unwrap();
    schedule_updates(&state).await.unwrap();
    assert_eq!(
        storage::list(&state.db).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        enqueue(&state, &service, Some("alice".into()))
            .await
            .is_err()
    );
    // Failed automatic candidates must not be retried every minute.
    progress(
        &state,
        job.payload["id"].as_str().unwrap(),
        "blocked",
        None,
        Some("Failed preflight".into()),
    )
    .await
    .unwrap();
    schedule_updates(&state).await.unwrap();
    assert_eq!(
        storage::list(&state.db).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Administrators can still explicitly retry the same image.
    assert!(
        enqueue(&state, &service, Some("alice".into()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn matching_images_do_not_queue_updates_and_missing_releases_surface_errors() {
    let (_temp, state, _cookie) = fixture().await;
    managed(&state, "automatic").await;
    allow_maintenance_now(&state).await;
    state.managers.docker.lock().unwrap().insert(
        "stack/releases".into(),
        json!({"items":[{"kind":"radarr","image":"repository@old"}]}),
    );
    check_releases(&state, false).await.unwrap();
    schedule_updates(&state).await.unwrap();
    assert_eq!(storage::list(&state.db).await.unwrap()["items"], json!([]));
    state.managers.docker.lock().unwrap().insert(
        "stack/releases".into(),
        json!({"items":[{"kind":"radarr","image":null}]}),
    );
    check_releases(&state, true).await.unwrap();
    schedule_updates(&state).await.unwrap();
    let value = storage::list(&state.db).await.unwrap();
    assert!(value["policies"][0]["candidate"].is_null());
    assert_eq!(
        value["policies"][0]["error"],
        "Stable release discovery unavailable"
    );
    assert_eq!(value["items"], json!([]));
}

#[tokio::test]
async fn server_and_service_update_policies_reject_manual_and_accept_notify() {
    let (_temp, state, cookie) = fixture().await;
    let key = managed(&state, "notify").await;
    for path in [
        "/api/v1/admin/product-update/policy".to_owned(),
        format!("/api/v1/admin/service-updates/policy/{key}"),
    ] {
        for (mode, status) in [
            ("manual", StatusCode::BAD_REQUEST),
            ("notify", StatusCode::OK),
            ("automatic", StatusCode::OK),
        ] {
            let response = call(
                &state,
                &path,
                "POST",
                json!({"policy":mode,"window_start":3,"window_end":5}),
                &cookie,
            )
            .await;
            assert_eq!(response.0, status, "{}", response.2);
        }
    }
}
