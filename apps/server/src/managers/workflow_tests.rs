use super::*;
use crate::test_support::{call, fixture};
use axum::http::{Request, StatusCode};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

// These HTTP regressions cover interrupted ownership and API maintenance without
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
async fn connection_can_be_repaired_after_stopped_adoption_without_changing_defaults_or_application()
 {
    let (_temp, state, cookie, key, port, writes, upstream) = existing().await;
    let review_id = id();
    state.managers.docker.lock().unwrap().extend([
        ("stack/adopt/preview".into(), json!({"review_id":review_id})),
        ("stack/adopt/check".into(), json!({"accepted":true})),
    ]);
    let preview = call(
        &state,
        "/api/v1/admin/stack/adopt/preview",
        "POST",
        json!({"service_id":key}),
        &cookie,
    )
    .await;
    assert_eq!(preview.0, StatusCode::OK);
    let adopted = call(
        &state,
        "/api/v1/admin/stack/adopt",
        "POST",
        json!({"service_id":key,"review_id":review_id}),
        &cookie,
    )
    .await;
    assert_eq!(adopted.0, StatusCode::OK, "{}", adopted.2);
    state
        .db
        .write("test.complete", move |db| {
            db.execute(
                "UPDATE stack_provisions SET state='complete' WHERE id=?1",
                [review_id],
            )?;
            db.execute(
                "UPDATE jobs SET state='complete' WHERE kind='stack.install'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["running"] = json!(true);
    let route = format!("/api/v1/admin/services/{key}/connection");
    let original = service(&state, &key).await.unwrap();
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
        service(&state, &key).await.unwrap().access_revision,
        original.access_revision
    );
    assert_eq!(
        call(&state, &route, "PUT", input, &cookie).await.0,
        StatusCode::OK
    );
    let repaired = service(&state, &key).await.unwrap();
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
    assert_eq!(service(&state, &key).await.unwrap().defaults, json!({}));
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    upstream.abort();
}

#[tokio::test]
async fn release_keeps_a_retryable_intent_after_transport_or_confirmation_loss() {
    for controller_committed in [false, true] {
        let (_temp, state, cookie, key, _port, _writes, upstream) = existing().await;
        let provision = id();
        let insert = provision.clone();
        state.db.write("test.owned",move |db| {
            db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,container_id,service_id,origin,created_at,updated_at) VALUES (?1,'radarr','alice','complete',?2,?3,'adopted',1,1)",params![insert,"a".repeat(64),key])?;
            Ok(())
        }).await.unwrap();
        let route = format!("/api/v1/admin/stack/{provision}/action");
        let failed = crate::test_support::CONTROLLER_FAILURE
            .scope(
                ("dependency_unavailable", "Response lost"),
                call(&state, &route, "POST", json!({"action":"release"}), &cookie),
            )
            .await;
        assert_eq!(failed.0, StatusCode::CONFLICT);
        state.managers.docker.lock().unwrap().extend([
            ("stack".into(),json!({"items":if controller_committed {json!([])} else {json!([{"id":provision,"kind":"radarr"}])}})),
            (format!("stack/{provision}/action"),json!({"accepted":true,"released":true})),
        ]);
        let list = call(&state, "/api/v1/admin/stack", "GET", Value::Null, &cookie).await;
        assert_eq!(list.2["provisions"][0]["operation"], "release");
        assert_eq!(
            call(&state, &route, "POST", json!({"action":"remove"}), &cookie)
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(&state, &route, "POST", json!({"action":"release"}), &cookie)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(&state, &route, "POST", json!({"action":"release"}), &cookie)
                .await
                .0,
            StatusCode::OK
        );
        assert!(
            call(&state, "/api/v1/admin/stack", "GET", Value::Null, &cookie)
                .await
                .2["provisions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/admin/managers",
                "GET",
                Value::Null,
                &cookie
            )
            .await
            .2["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        upstream.abort();
    }
}

#[tokio::test]
async fn adoption_rejects_a_review_after_connection_revision_changes() {
    let (_temp, state, cookie, key, port, _writes, upstream) = existing().await;
    let review_id = id();
    state.managers.docker.lock().unwrap().extend([
        ("stack/adopt/preview".into(), json!({"review_id":review_id})),
        ("stack/adopt/check".into(), json!({"accepted":true})),
    ]);
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/stack/adopt/preview",
            "POST",
            json!({"service_id":key}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/services/{key}/connection"),
            "PUT",
            json!({"port":port,"url_base":"","secret":"correct-fixture-api-key","allow_unverified":true}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    let result = call(
        &state,
        "/api/v1/admin/stack/adopt",
        "POST",
        json!({"service_id":key,"review_id":review_id}),
        &cookie,
    )
    .await;
    assert_eq!(result.0, StatusCode::CONFLICT, "{}", result.2);
    upstream.abort();
}

#[tokio::test]
async fn seerr_requires_a_managed_link_and_preserves_conflicting_native_connections() {
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
    let stored_manager = manager_id.clone();
    state.db.write("test.request_choices",move |db| {
        db.execute("UPDATE manager_services SET defaults=?1 WHERE id=?2",params![json!({"root_folder":"/movies","quality_profile":1,"monitored":true}).to_string(),stored_manager])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,service_id,created_at,updated_at) VALUES ('installed-radarr','radarr','alice','complete',?1,1,1)",[stored_manager])?;
        Ok(())
    }).await.unwrap();
    connections::tick(&state).await.unwrap();
    assert_eq!(*native.lock().await, original);
    let configure =
        |action: &str| json!({"source_id":seerr_id,"target_id":manager_id,"action":action});
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
        StatusCode::OK
    );
    connections::tick(&state).await.unwrap();
    assert_eq!(native.lock().await.len(), 2);
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
    assert_eq!(service(&state, &key).await.unwrap().container, followed);

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
    assert_eq!(service(&state, &key).await.unwrap().container, followed);
    let chosen = call(
        &state,
        &route,
        "PUT",
        json!({"port":port,"url_base":"","container_id":changed}),
        &cookie,
    )
    .await;
    assert_eq!(chosen.0, StatusCode::OK, "{}", chosen.2);
    assert_eq!(service(&state, &key).await.unwrap().container, changed);

    // Installed and adopted identities belong to the controller, never to name matching.
    let stored = key.clone();
    state.db.write("test.owned", move |db| {
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,service_id,created_at,updated_at) VALUES ('adopted-radarr','radarr','alice','complete',?1,1,1)",[stored])?;
        Ok(())
    }).await.unwrap();
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
