use super::*;
use crate::test_support::{CONTROLLER_FAILURE, call, fixture};
use axum::http::StatusCode;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn queued(input: Value) -> (tempfile::TempDir, AppState, String, thelxinoe_jobs::Job) {
    let (temp, mut state, cookie) = fixture().await;
    Arc::make_mut(&mut state.config).media = "/media".into();
    state.db.write("test.recyclarr", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id IN ('alice','bob')", [])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,created_at,updated_at) VALUES ('recyclarr','recyclarr','alice','complete',1,1)", [])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('radarr','Radarr','radarr','aaaaaaaaaaaa',7878,'generation',X'01','/media','1',1)", [])?;
        Ok(())
    }).await.unwrap();
    storage::installed(&state.db, "recyclarr".into(), "alice".into())
        .await
        .unwrap();
    storage::queue(&state.db, "recyclarr".into(), "alice".into(), false, input)
        .await
        .unwrap();
    let job = thelxinoe_jobs::Queue(state.db.clone())
        .claim()
        .await
        .unwrap()
        .unwrap();
    (temp, state, cookie, job)
}

#[tokio::test]
async fn retry_policy_uses_failure_kind_and_preserves_actor_authority() {
    for (code, message, pinned, retry) in [
        ("dependency_unavailable", "Connection reset", false, true),
        (
            "operation_contended",
            "Another run owns the lease",
            false,
            true,
        ),
        ("revision_changed", "The image has changed", false, true),
        ("revision_changed", "The image has changed", true, false),
        (
            "conflict",
            "Access is unavailable for this selection",
            false,
            false,
        ),
        ("forbidden", "No access", false, false),
    ] {
        let (_temp, state, cookie, job) = queued(if pinned {
            json!({"revision":"pinned"})
        } else {
            json!({})
        })
        .await;
        let result = CONTROLLER_FAILURE
            .scope((code, message), run_job(&state, &job))
            .await;
        assert_eq!(matches!(result, Ok(false)), retry, "{code}: {result:?}");
        let details = call(
            &state,
            &format!(
                "/api/v1/admin/recyclarr/runs/{}",
                job.payload["id"].as_str().unwrap()
            ),
            "GET",
            Value::Null,
            &cookie,
        )
        .await;
        assert_eq!(details.0, StatusCode::OK);
        assert_eq!(
            details.2["state"],
            if retry { "retrying" } else { "blocked" }
        );
        let job_id = job.id.clone();
        state
            .db
            .read("test.retry", move |db| {
                let (state, available, started): (String, i64, Option<i64>) = db.query_row(
                    "SELECT state,available_at,started_at FROM jobs WHERE id=?1",
                    [job_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                assert_eq!(state == "queued", retry);
                if retry {
                    assert!(available > now());
                    assert!(started.is_none());
                }
                Ok(())
            })
            .await
            .unwrap();
        if retry {
            state
                .db
                .write("test.revoke", |db| {
                    db.execute("UPDATE users SET role='user' WHERE id='alice'", [])?;
                    Ok(())
                })
                .await
                .unwrap();
            assert!(
                CONTROLLER_FAILURE
                    .scope((code, message), run_job(&state, &job))
                    .await
                    .is_err()
            );
            assert_eq!(
                storage::run(&state.db, job.payload["id"].as_str().unwrap().into())
                    .await
                    .unwrap()
                    .1,
                "blocked"
            );
        }
    }
}

#[tokio::test]
async fn partial_controller_result_remains_terminal_when_arr_cannot_be_read_afterward() {
    let (_temp, state, cookie, job) = queued(json!({})).await;
    let requests = Arc::new(AtomicUsize::new(0));
    let calls = requests.clone();
    let app = Router::new().fallback(move || {
        let calls = calls.clone();
        async move {
            if calls.fetch_add(1, Ordering::SeqCst) >= 5 {
                (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
            } else {
                (StatusCode::OK, Json(json!([])))
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let credential = state
        .secrets
        .encrypt("manager:radarr", b"test-key")
        .unwrap();
    state
        .db
        .write("test.manager", move |db| {
            db.execute(
                "UPDATE manager_services SET port=?1,credential=?2",
                params![port, credential],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let docker = json!({"id":"aaaaaaaaaaaa","name":"localhost","running":true,"networks":[{"id":"shared","address":"127.0.0.1"}],"mounts":[{"destination":"/media","source":"/media","kind":"bind","writable":true}]});
    state.managers.docker.lock().unwrap().extend([
        ("containers/self".into(), docker.clone()),
        ("containers/aaaaaaaaaaaa".into(), docker),
        ("stack/recyclarr/recyclarr/configuration".into(), json!({"initialized":true,"revision":"config-v1","mode":"customized","used_services":["radarr"]})),
        ("stack/recyclarr/recyclarr/catalog".into(), json!({"image":"image","resources":"resources","items":[]})),
        (format!("stack/recyclarr/recyclarr/results/{}", job.payload["id"].as_str().unwrap()), json!({"state":"partial","output":"Applied one profile before failure","configuration_revision":"config-v1"})),
    ]);
    let media_source = {
        let inspections = state.managers.docker.lock().unwrap();
        crate::managers::storage_paths::evidence(
            &inspections["containers/self"],
            &inspections["containers/aaaaaaaaaaaa"],
            &state.config.media.to_string_lossy(),
        )
        .unwrap()
    };
    state
        .db
        .write("test.storage", move |db| {
            db.execute(
                "UPDATE manager_services SET media_source=?1",
                [media_source],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(run_job(&state, &job).await.is_err());
    let path = format!(
        "/api/v1/admin/recyclarr/runs/{}",
        job.payload["id"].as_str().unwrap()
    );
    let details = call(&state, &path, "GET", Value::Null, &cookie).await;
    assert_eq!(details.0, StatusCode::OK);
    assert_eq!(details.2["state"], "partial", "{}", details.2);
    assert_eq!(
        details.2["evidence"]["output"],
        "Applied one profile before failure"
    );
    assert!(details.2["evidence"]["before"]["radarr"].is_object());
    let before = requests.load(Ordering::SeqCst);
    assert!(run_job(&state, &job).await.unwrap());
    assert_eq!(
        requests.load(Ordering::SeqCst),
        before,
        "terminal recovery must not replay writes"
    );
    server.abort();
}
