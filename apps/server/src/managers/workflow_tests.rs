use super::*;
use crate::test_support::{call, fixture};
use axum::http::{Request, StatusCode};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

// These HTTP regressions cover Connected maintenance and transfer review without
// requiring a live downloader or a timing-dependent Docker transport outage.
async fn existing() -> (
    tempfile::TempDir,
    AppState,
    String,
    String,
    u16,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (temp, mut state, cookie) = fixture().await;
    Arc::get_mut(&mut state.config).unwrap().media = "/media".into();
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let mutations = Arc::new(AtomicUsize::new(0));
    let writes = mutations.clone();
    let stub = Router::new().fallback(move |request: Request<axum::body::Body>| {
        let writes = writes.clone();
        async move {
            if request.method() != "GET" {
                writes.fetch_add(1, Ordering::SeqCst);
            }
            if request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                != Some("correct-fixture-api-key")
            {
                return (StatusCode::UNAUTHORIZED, Json(Value::Null));
            }
            let value = match request.uri().path() {
                "/api/v3/system/status" => json!({"appName":"Radarr","version":"6","urlBase":""}),
                "/api/v3/rootfolder" => json!([{"id":1,"path":"/movies"}]),
                "/api/v3/qualityprofile" => json!([{"id":1,"name":"Existing HD","items":[]}]),
                "/api/v3/qualityprofile/schema" => {
                    json!({"id":0,"items":[{"quality":{"id":1},"allowed":false}]})
                }
                "/api/v3/command" => json!([]),
                "/api/v3/queue" => json!({"totalRecords":0}),
                _ => Value::Null,
            };
            (StatusCode::OK, Json(value))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, stub).await.unwrap();
    });
    let container = "a".repeat(64);
    let inspection = json!({"id":container,"name":"localhost","running":false,"mounts":[{"kind":"bind","source":"/tank","destination":"/media","writable":true}],"networks":[{"id":"shared","address":"127.0.0.1"}]});
    state.managers.docker.lock().unwrap().extend([
        ("containers/self".into(), inspection.clone()),
        (format!("containers/{container}"), inspection),
    ]);
    let request = json!({"name":"Existing Radarr","kind":"radarr","container_id":container,"port":port,"api_key":"incorrect-fixture-key"});
    // A stopped container is only connected unverified after explicit confirmation.
    let stopped = call(
        &state,
        "/api/v1/admin/managers",
        "POST",
        request.clone(),
        &cookie,
    )
    .await;
    assert_eq!(stopped.0, StatusCode::CONFLICT, "{}", stopped.2);
    assert_eq!(stopped.2["error"]["code"], "service_stopped");
    assert!(
        stopped.2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("The localhost container is stopped")
    );
    let mut request = request;
    request["allow_unverified"] = json!(true);
    let registered = call(&state, "/api/v1/admin/managers", "POST", request, &cookie).await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.2);
    let key = registered.2["id"].as_str().unwrap().to_owned();
    (temp, state, cookie, key, port, mutations, upstream)
}

#[tokio::test]
async fn connected_service_can_be_repaired_without_changing_defaults_or_application() {
    let (_temp, state, cookie, key, port, writes, upstream) = existing().await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["running"] = json!(true);
    let route = format!("/api/v1/admin/services/{key}/connection");
    let original = connected(&state, &key).await.unwrap();
    assert_eq!(
        call(
            &state,
            &format!("{route}/test"),
            "POST",
            json!({"port":port,"url_base":""}),
            &cookie
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let input = json!({"port":port,"url_base":"","secret":"correct-fixture-api-key"});
    assert_eq!(
        call(
            &state,
            &format!("{route}/test"),
            "POST",
            input.clone(),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        connected(&state, &key).await.unwrap().access_revision,
        original.access_revision
    );
    assert_eq!(
        call(&state, &route, "PUT", input, &cookie).await.0,
        StatusCode::OK
    );
    let repaired = connected(&state, &key).await.unwrap();
    assert_eq!(repaired.container, original.container);
    assert_eq!(repaired.defaults, original.defaults);
    assert_ne!(repaired.access_revision, original.access_revision);
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/managers/{key}/test"),
            "POST",
            json!({}),
            &cookie
        )
        .await
        .2["healthy"],
        true
    );
    // A Connected service is validation-only; request options need an Owned service.
    let options = call(
        &state,
        &format!("/api/v1/admin/managers/{key}/options"),
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(options.0, StatusCode::CONFLICT, "{}", options.2);
    crate::test_support::own_services(&state).await;
    for _ in 0..2 {
        let options = call(
            &state,
            &format!("/api/v1/admin/managers/{key}/options"),
            "GET",
            Value::Null,
            &cookie,
        )
        .await;
        assert_eq!(options.0, StatusCode::OK, "{}", options.2);
        assert_eq!(options.2["defaults"], json!({}));
        assert_eq!(options.2["roots"][0]["path"], "/movies");
    }
    // Without saved defaults a profile is refused before anything is created upstream.
    let profile = call(
        &state,
        &format!("/api/v1/admin/managers/{key}/quality-profiles"),
        "POST",
        json!({"name":"Custom","qualities":[1],"cutoff":1}),
        &cookie,
    )
    .await;
    assert_eq!(profile.0, StatusCode::CONFLICT, "{}", profile.2);
    assert_eq!(service(&state, &key).await.unwrap().defaults, json!({}));
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    upstream.abort();
}

#[tokio::test]
async fn auto_delete_consent_does_not_carry_over_to_a_reused_manager_row() {
    let (_temp, state, cookie, key, port, _writes, upstream) = existing().await;
    crate::test_support::own_services(&state).await;
    let saved = call(
        &state,
        &format!("/api/v1/admin/managers/{key}/retention"),
        "PUT",
        json!({"enabled":true}),
        &cookie,
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.2);
    let disabled = key.clone();
    state
        .db
        .write("test.retire", move |db| {
            // Like a removed installation.
            db.execute(
                "UPDATE manager_services SET enabled=0,operational=0 WHERE id=?1",
                [&disabled],
            )?;
            db.execute(
                "DELETE FROM stack_provisions WHERE service_id=?1",
                [&disabled],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Another Radarr registered later reuses the row of its kind.
    let request = json!({"name":"Another Radarr","kind":"radarr","container_id":"a".repeat(64),
        "port":port,"api_key":"incorrect-fixture-key","allow_unverified":true});
    let registered = call(&state, "/api/v1/admin/managers", "POST", request, &cookie).await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.2);
    assert_eq!(registered.2["id"], key);
    let consent = state
        .db
        .read("test.consent", move |db| {
            Ok(db
                .query_row(
                    "SELECT value FROM settings WHERE key='manager.retention.'||?1",
                    [key],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .unwrap();
    assert_eq!(consent, None);
    upstream.abort();
}

#[tokio::test]
async fn transfer_approval_is_rejected_after_the_connection_changes() {
    let (_temp, state, cookie, key, port, writes, upstream) = existing().await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["running"] = json!(true);
    let route = format!("/api/v1/admin/services/{key}/connection");
    let input = json!({"port":port,"url_base":"","secret":"correct-fixture-api-key"});
    assert_eq!(
        call(&state, &route, "PUT", input.clone(), &cookie).await.0,
        StatusCode::OK
    );
    let review_id = id();
    state.managers.docker.lock().unwrap().insert(
        "stack/transfers/review".into(),
        json!({"review_id":review_id,"revision":"plan","kind":"radarr","blockers":[],"automation":[],"compose":null}),
    );
    let review = call(
        &state,
        &format!("/api/v1/admin/services/{key}/transfer"),
        "POST",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(review.0, StatusCode::OK, "{}", review.2);
    assert_eq!(review.2["state"], "review");
    assert_eq!(review.2["review"]["blockers"], json!([]), "{}", review.2);
    let approval = json!({"review_revision":review.2["review_revision"],"confirm":true,
        "declarations":{"cron":true,"systemd":true,"ci_cd":true,"configuration_management":true,"manual_compose":true}});
    // Reviewing reads the original's API only; it never changes the Connected service.
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&state, &route, "PUT", input, &cookie).await.0,
        StatusCode::OK
    );
    let approve = format!("/api/v1/admin/transfers/{review_id}/approve");
    let stale = call(&state, &approve, "POST", approval.clone(), &cookie).await;
    assert_eq!(stale.0, StatusCode::CONFLICT, "{}", stale.2);
    assert_eq!(stale.2["error"]["code"], "revision_changed");
    // The stale review is closed; nothing was queued and the original keeps running.
    let closed = call(
        &state,
        &format!("/api/v1/admin/transfers/{review_id}"),
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(closed.2["state"], "cancelled", "{}", closed.2);
    assert_eq!(
        call(&state, &approve, "POST", approval, &cookie).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/transfers/{review_id}"),
            "GET",
            Value::Null,
            &cookie
        )
        .await
        .2["state"],
        "cancelled"
    );
    let queued: i64 = state
        .db
        .read("test.transfer_jobs", |db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind='service.transfer'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(queued, 0);
    upstream.abort();
}

#[tokio::test]
async fn seerr_links_only_owned_pairs_and_preserves_conflicting_native_connections() {
    let (_temp, state, cookie, manager_id, manager_port, _writes, manager_upstream) =
        existing().await;
    call(
        &state,
        &format!("/api/v1/admin/services/{manager_id}/connection"),
        "PUT",
        json!({"port":manager_port,"url_base":"","secret":"correct-fixture-api-key","allow_unverified":true}),
        &cookie,
    )
    .await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["running"] = json!(true);
    let native = Arc::new(tokio::sync::Mutex::new(vec![
        json!({"id":99,"name":"Thelxinoe radarr","hostname":"other-manager","syncEnabled":false}),
    ]));
    let records = native.clone();
    let stub = Router::new().fallback(move |request: Request<axum::body::Body>| {
        let records = records.clone();
        async move {
            let path = request.uri().path().to_owned();
            let method = request.method().clone();
            let result = match path.as_str() {
                "/api/v1/status" => json!({"version":"3"}),
                "/api/v1/settings/public" => json!({"initialized":true}),
                "/api/v1/settings/sonarr" => json!([]),
                "/api/v1/settings/radarr" if method == reqwest::Method::GET => {
                    json!(*records.lock().await)
                }
                "/api/v1/settings/radarr" => {
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let mut row: Value = serde_json::from_slice(&bytes).unwrap();
                    row["id"] = json!(100);
                    records.lock().await.push(row.clone());
                    row
                }
                "/api/v1/settings/radarr/100" if method == reqwest::Method::DELETE => {
                    records.lock().await.retain(|r| r["id"] != 100);
                    Value::Null
                }
                "/api/v1/settings/radarr/100" => {
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let row: Value = serde_json::from_slice(&bytes).unwrap();
                    records.lock().await[1] = row.clone();
                    row
                }
                _ => Value::Null,
            };
            Json(result)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, stub).await.unwrap();
    });
    let container = "b".repeat(64);
    state.managers.docker.lock().unwrap().insert(format!("containers/{container}"),json!({"id":container,"name":"localhost","running":true,"mounts":[],"networks":[{"id":"shared","address":"127.0.0.1"}]}));
    let registered=call(&state,"/api/v1/admin/support","POST",json!({"kind":"seerr","name":"Existing Seerr","container_id":container,"port":port,"credentials":{"secret":"fixture-seerr-key"}}),&cookie).await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.2);
    let seerr_id = registered.2["id"].as_str().unwrap().to_owned();
    let original = native.lock().await.clone();
    let configure =
        |action: &str| json!({"source_id":seerr_id,"target_id":manager_id,"action":action});
    // Connected services are validation-only: no link is offered or written.
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/service-connections",
            "POST",
            configure("connect"),
            &cookie
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    connections::tick(&state).await.unwrap();
    assert_eq!(*native.lock().await, original);
    let stored_manager = manager_id.clone();
    state
        .db
        .write("test.request_choices", move |db| {
            db.execute(
                "UPDATE manager_services SET defaults=?1 WHERE id=?2",
                params![
                    json!({"root_folder":"/movies","quality_profile":1,"monitored":true})
                        .to_string(),
                    stored_manager
                ],
            )?;
            crate::test_support::own(db)?;
            Ok(())
        })
        .await
        .unwrap();
    // Owned pairs link automatically, beside the conflicting record it never touches.
    connections::tick(&state).await.unwrap();
    assert_eq!(native.lock().await.len(), 2);
    assert_eq!(native.lock().await[0], original[0]);
    native.lock().await[1]["activeDirectory"] = json!("/manual-choice");
    let manual = native.lock().await.clone();
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/service-connections",
            "POST",
            configure("retry"),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    connections::tick(&state).await.unwrap();
    assert_eq!(*native.lock().await, manual);
    let links = call(
        &state,
        "/api/v1/admin/service-connections",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(
        links.2["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["source_id"] == seerr_id)
            .unwrap()["state"],
        "conflict"
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/service-connections",
            "POST",
            configure("disconnect"),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    connections::tick(&state).await.unwrap();
    assert_eq!(*native.lock().await, manual);
    manager_upstream.abort();
    upstream.abort();
}

#[tokio::test]
async fn saved_defaults_retry_the_seerr_link_that_waited_for_them() {
    let (_temp, state, cookie, manager_id, manager_port, _writes, manager_upstream) =
        existing().await;
    call(
        &state,
        &format!("/api/v1/admin/services/{manager_id}/connection"),
        "PUT",
        json!({"port":manager_port,"url_base":"","secret":"correct-fixture-api-key","allow_unverified":true}),
        &cookie,
    )
    .await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["running"] = json!(true);
    let native = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let records = native.clone();
    let stub = Router::new().fallback(move |request: Request<axum::body::Body>| {
        let records = records.clone();
        async move {
            let path = request.uri().path().to_owned();
            let create = request.method() == reqwest::Method::POST;
            Json(match path.as_str() {
                "/api/v1/status" => json!({"version":"3"}),
                "/api/v1/settings/public" => json!({"initialized":true}),
                "/api/v1/settings/radarr" if create => {
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let mut row: Value = serde_json::from_slice(&bytes).unwrap();
                    row["id"] = json!(100);
                    records.lock().await.push(row.clone());
                    row
                }
                "/api/v1/settings/radarr" => json!(*records.lock().await),
                _ => Value::Null,
            })
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, stub).await.unwrap();
    });
    let container = "b".repeat(64);
    state.managers.docker.lock().unwrap().insert(format!("containers/{container}"),json!({"id":container,"name":"localhost","running":true,"mounts":[],"networks":[{"id":"shared","address":"127.0.0.1"}]}));
    let registered=call(&state,"/api/v1/admin/support","POST",json!({"kind":"seerr","name":"Seerr","container_id":container,"port":port,"credentials":{"secret":"fixture-seerr-key"}}),&cookie).await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.2);
    let seerr_id = registered.2["id"].as_str().unwrap().to_owned();
    state
        .db
        .write("test.own", |db| Ok(crate::test_support::own(db)?))
        .await
        .unwrap();
    let link = async || {
        call(
            &state,
            "/api/v1/admin/service-connections",
            "GET",
            Value::Null,
            &cookie,
        )
        .await
        .2["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["source_id"] == seerr_id)
            .unwrap()
            .clone()
    };
    let reason = "Choose an existing root folder and request profile for this manager first";
    connections::tick(&state).await.unwrap();
    assert_eq!(link().await["state"], "conflict");
    assert_eq!(link().await["error"], reason);
    // A refused request says why instead of asking to connect a linked Seerr.
    let request = call(
        &state,
        "/api/v1/seerr/requests",
        "POST",
        json!({"media_type":"movie","media_id":603,"profile":{"source":"custom","service_id":manager_id,"profile_id":1}}),
        &cookie,
    )
    .await;
    assert_eq!(request.0, StatusCode::CONFLICT, "{}", request.2);
    assert_eq!(
        request.2["error"]["message"],
        format!("Seerr isn't connected to Existing Radarr: {reason}")
    );
    let saved = call(
        &state,
        &format!("/api/v1/admin/managers/{manager_id}/defaults"),
        "PUT",
        json!({"root_folder":"/movies","quality_profile":1,"metadata_profile":null,"monitored":true}),
        &cookie,
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.2);
    connections::tick(&state).await.unwrap();
    assert_eq!(link().await["state"], "connected");
    let records = native.lock().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["activeDirectory"], "/movies");
    drop(records);
    manager_upstream.abort();
    upstream.abort();
}

fn recreate(state: &AppState, from: &str, to: &str, mounts: Value) {
    let mut docker = state.managers.docker.lock().unwrap();
    docker.insert(format!("containers/{from}"), Value::Null);
    docker.insert(
        format!("containers/{to}"),
        json!({"id":to,"name":"localhost","running":true,"mounts":mounts,"networks":[{"id":"shared","address":"127.0.0.1"}]}),
    );
    docker.insert(
        "containers".into(),
        json!({"items":[{"id":to,"names":["/localhost"]}]}),
    );
}

#[tokio::test]
async fn recreated_container_is_followed_only_with_the_same_name_mounts_and_api_identity() {
    let (_temp, state, cookie, key, port, _writes, upstream) = existing().await;
    let original = "a".repeat(64);
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{original}"))
        .unwrap()["running"] = json!(true);
    let route = format!("/api/v1/admin/services/{key}/connection");
    let saved = call(
        &state,
        &route,
        "PUT",
        json!({"port":port,"url_base":"","secret":"correct-fixture-api-key"}),
        &cookie,
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.2);
    let probe = format!("/api/v1/admin/managers/{key}/test");
    let media = json!([{"kind":"bind","source":"/tank","destination":"/media","writable":true}]);

    // Compose or Watchtower replaced the container under the same name and mounts.
    let followed = "c".repeat(64);
    recreate(&state, &original, &followed, media);
    let test = call(&state, &probe, "POST", Value::Null, &cookie).await;
    assert_eq!(test.0, StatusCode::OK, "{}", test.2);
    assert_eq!(connected(&state, &key).await.unwrap().container, followed);

    // Different storage must be reviewed before Thelxinoe uses the replacement.
    let changed = "d".repeat(64);
    recreate(
        &state,
        &followed,
        &changed,
        json!([{"kind":"bind","source":"/other","destination":"/media","writable":true}]),
    );
    let test = call(&state, &probe, "POST", Value::Null, &cookie).await;
    assert_eq!(test.0, StatusCode::CONFLICT, "{}", test.2);
    assert!(
        test.2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different storage mounts"),
        "{}",
        test.2
    );
    assert_eq!(connected(&state, &key).await.unwrap().container, followed);
    let chosen = call(
        &state,
        &route,
        "PUT",
        json!({"port":port,"url_base":"","container_id":changed}),
        &cookie,
    )
    .await;
    assert_eq!(chosen.0, StatusCode::OK, "{}", chosen.2);
    assert_eq!(connected(&state, &key).await.unwrap().container, changed);

    // Owned identities belong to the controller, never to name matching.
    crate::test_support::own_services(&state).await;
    recreate(&state, &changed, &"e".repeat(64), json!([]));
    let test = call(&state, &probe, "POST", Value::Null, &cookie).await;
    assert_eq!(test.0, StatusCode::CONFLICT, "{}", test.2);
    assert!(
        test.2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Review it in Media services"),
        "{}",
        test.2
    );
    assert_eq!(service(&state, &key).await.unwrap().container, changed);
    upstream.abort();
}
