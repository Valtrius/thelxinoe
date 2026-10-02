use super::*;
use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[tokio::test]
async fn removed_target_cleanup_stays_visible_retries_and_removes_only_its_link() {
    let (_temp, state, cookie, mock) = connected_services().await;
    action(&state, &cookie, "radarr", "connect").await;
    action(&state, &cookie, "sonarr", "connect").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.rows.lock().await.len(), 2);
    let key = id();
    let insert_key = key.clone();
    state.db.write("test.remove", move |db| {
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,created_at,updated_at) VALUES (?1,'radarr','alice',17878,X'01','complete','old','radarr',1,1)",[insert_key])?;
        Ok(())
    }).await.unwrap();
    state.managers.docker.lock().unwrap().extend([
        (
            "stack".into(),
            json!({"items":[{"id":key,"can_remove":true}]}),
        ),
        (format!("stack/{key}/action"), json!({"removed":true})),
    ]);
    let response = call(
        &state,
        &format!("/api/v1/admin/stack/{key}/action"),
        "POST",
        json!({"action":"remove"}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    running(&state, 'c', false);
    tick(&state).await.unwrap();
    let response = call(
        &state,
        "/api/v1/admin/service-connections",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert!(
        response.2["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["target_id"] == "radarr"
                && l["cleanup_pending"] == true
                && l["source_kind"] == "prowlarr")
    );
    action(&state, &cookie, "radarr", "retry").await;
    running(&state, 'c', true);
    tick(&state).await.unwrap();
    assert_eq!(mock.rows.lock().await.len(), 1);
    assert!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "sonarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "connected"
    );
}

#[tokio::test]
async fn retired_target_cleanup_preserves_disabled_link_for_reinstall() {
    let (_temp, state, cookie, mock) = connected_services().await;
    action(&state, &cookie, "radarr", "connect").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.rows.lock().await.len(), 1);

    let key = id();
    let insert_key = key.clone();
    state
        .db
        .write("test.retire", move |db| {
            db.execute(
                "INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,created_at,updated_at) VALUES (?1,'radarr','alice',17878,X'01','complete','old','radarr',1,1)",
                [insert_key],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    state.managers.docker.lock().unwrap().extend([
        ("stack".into(), json!({"items":[]})),
        (format!("stack/{key}/action"), json!({"retired":true})),
    ]);

    let response = call(
        &state,
        &format!("/api/v1/admin/stack/{key}/action"),
        "POST",
        json!({"action":"retire"}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);

    let pending = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert!(!pending.enabled);
    assert!(pending.cleanup);
    assert!(pending.retain_missing);

    tick(&state).await.unwrap();
    assert!(mock.rows.lock().await.is_empty());
    let retired = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retired.state, "disconnected");
    assert!(!retired.enabled);
    assert!(!retired.cleanup);
    assert!(retired.retain_missing);

    state
        .db
        .write("test.reinstall", |db| {
            db.execute(
                "UPDATE manager_services SET enabled=1,error=NULL WHERE id='radarr'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    tick(&state).await.unwrap();
    let restored = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert!(!restored.enabled);
    assert_eq!(restored.state, "disconnected");

    let mut retained = restored;
    retained.upstream_id = Some(42);
    retained.applied_hash = Some("old-applied".into());
    retained.pending_hash = Some("old-pending".into());
    retained.prepared = true;
    storage::save(&state.db, retained).await.unwrap();
    action(&state, &cookie, "radarr", "connect").await;
    let reconnecting = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert!(reconnecting.enabled);
    assert!(!reconnecting.retain_missing);
    assert!(reconnecting.upstream_id.is_none());
    assert!(reconnecting.applied_hash.is_none());
    assert!(reconnecting.pending_hash.is_none());
    assert!(!reconnecting.prepared);
}

struct Mock {
    rows: Arc<tokio::sync::Mutex<Vec<Value>>>,
    posts: Arc<AtomicUsize>,
    lose_response: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn masked(rows: &[Value]) -> Value {
    let mut rows = rows.to_vec();
    for row in &mut rows {
        for field in row["fields"].as_array_mut().unwrap() {
            if field["name"] == "apiKey" && field["value"].as_str().is_some_and(|v| !v.is_empty()) {
                field["value"] = json!("********");
            }
        }
    }
    json!(rows)
}

async fn connected_services() -> (tempfile::TempDir, AppState, String, Mock) {
    connected_services_on(false, "").await
}
async fn connected_services_on(
    split: bool,
    url_base: &'static str,
) -> (tempfile::TempDir, AppState, String, Mock) {
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
    let rows = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let posts = Arc::new(AtomicUsize::new(0));
    let lose_response = Arc::new(AtomicBool::new(false));
    let get_rows = rows.clone();
    let create_rows = rows.clone();
    let update_rows = rows.clone();
    let delete_rows = rows.clone();
    let creates = posts.clone();
    let lost = lose_response.clone();
    let source=Router::new()
        .route("/api/v1/applications/test",post(move |Json(record):Json<Value>| async move {
            let fields = record["fields"].as_array().unwrap();
            let field = |name: &str| fields.iter().find(|f|f["name"]==name).unwrap()["value"].as_str().unwrap().to_owned();
            let source = field("prowlarrUrl");
            let target = field("baseUrl");
            if split && (!source.starts_with("http://127.0.0.2:") || !target.starts_with("http://127.0.0.3:")) { return StatusCode::BAD_REQUEST; }
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            for (base, api, app) in [(source, "v1", "Prowlarr"), (target, "v3", record["implementation"].as_str().unwrap())] {
                let response = client.get(format!("{base}/api/{api}/system/status")).send().await.unwrap();
                if response.json::<Value>().await.unwrap()["appName"] != app { return StatusCode::BAD_REQUEST; }
            }
            StatusCode::NO_CONTENT
        }))
        .route("/api/v1/system/status",get(||async{Json(json!({"appName":"Prowlarr","version":"1"}))}))
        .route("/api/v1/applications/schema",get(||async{Json(json!([
            {"implementation":"Radarr","fields":[{"name":"baseUrl","value":""},{"name":"prowlarrUrl","value":""},{"name":"apiKey","value":""}]},
            {"implementation":"Sonarr","fields":[{"name":"baseUrl","value":""},{"name":"prowlarrUrl","value":""},{"name":"apiKey","value":""}]}
        ]))}))
        .route("/api/v1/applications",get(move||{let rows=get_rows.clone();async move{Json(masked(&rows.lock().await))}})
            .post(move|Json(mut value):Json<Value>|{
                let rows=create_rows.clone();let creates=creates.clone();let lost=lost.clone();
                async move {
                    creates.fetch_add(1,Ordering::SeqCst);
                    let mut rows=rows.lock().await;value["id"]=json!(rows.len()+1);rows.push(value.clone());
                    (if lost.swap(false,Ordering::SeqCst){StatusCode::BAD_GATEWAY}else{StatusCode::OK},Json(value))
                }
            }))
        .route("/api/v1/applications/{id}",axum::routing::put(move|Path(id):Path<i64>,Json(mut value):Json<Value>|{
            let rows=update_rows.clone();async move {
                let mut rows=rows.lock().await;let row=rows.iter_mut().find(|r|r["id"]==id).unwrap();
                for field in value["fields"].as_array_mut().unwrap() {
                    if field["value"]=="********" {
                        field["value"]=row["fields"].as_array().unwrap().iter().find(|f|f["name"]==field["name"]).unwrap()["value"].clone();
                    }
                }
                *row=value.clone();Json(value)
            }
        }).delete(move|Path(id):Path<i64>|{let rows=delete_rows.clone();async move{rows.lock().await.retain(|r|r["id"]!=id);StatusCode::NO_CONTENT}}));
    let source = if url_base.is_empty() {
        source
    } else {
        Router::new().nest(url_base, source)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source_port = listener.local_addr().unwrap().port();
    let peer_source = source.clone();
    let mut tasks = vec![tokio::spawn(async move {
        axum::serve(listener, source).await.unwrap()
    })];
    if split {
        let listener =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::new(127, 0, 0, 2), source_port))
                .await
                .unwrap();
        tasks.push(tokio::spawn(async move {
            axum::serve(listener, peer_source).await.unwrap()
        }));
    }
    for (kind, character) in [("prowlarr", 'c'), ("radarr", 'a'), ("sonarr", 'b')] {
        let port = if kind == "prowlarr" {
            source_port
        } else {
            let app = if kind == "radarr" { "Radarr" } else { "Sonarr" };
            let router = Router::new().route(
                "/api/v3/system/status",
                get(move || async move { Json(json!({"appName":app,"version":"1"})) }),
            );
            let router = if url_base.is_empty() {
                router
            } else {
                Router::new().nest(url_base, router)
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            if split {
                let peer = router.clone();
                let listener =
                    tokio::net::TcpListener::bind((std::net::Ipv4Addr::new(127, 0, 0, 3), port))
                        .await
                        .unwrap();
                tasks.push(tokio::spawn(async move {
                    axum::serve(listener, peer).await.unwrap()
                }));
            }
            tasks.push(tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap()
            }));
            port
        };
        let container = character.to_string().repeat(64);
        let networks = if split {
            json!([{"id":if kind=="prowlarr"{"server-source"}else{"server-target"},"address":"127.0.0.1"},{"id":"peers","address":if kind=="prowlarr"{"127.0.0.2"}else{"127.0.0.3"}}])
        } else {
            json!([{"id":"shared","address":"127.0.0.1"}])
        };
        let inspection = json!({"id":container,"name":if kind=="prowlarr" {"localhost"} else {kind},"running":true,"mounts":[{"kind":"bind","source":"/media","destination":"/media","writable":true}],"networks":networks});
        state
            .managers
            .docker
            .lock()
            .unwrap()
            .insert(format!("containers/{container}"), inspection.clone());
        state
            .managers
            .docker
            .lock()
            .unwrap()
            .insert("containers/self".into(), inspection);
        if split {
            state
                .managers
                .docker
                .lock()
                .unwrap()
                .get_mut("containers/self")
                .unwrap()["networks"] = json!([{"id":"server-source","address":"127.0.0.1"},{"id":"server-target","address":"127.0.0.1"}]);
        }
        let credential = if kind == "prowlarr" {
            state
                .secrets
                .encrypt(
                    "support:prowlarr",
                    br#"{"username":"","secret":"fixture-prowlarr-key"}"#,
                )
                .unwrap()
        } else {
            state
                .secrets
                .encrypt(&format!("manager:{kind}"), b"fixture-manager-key")
                .unwrap()
        };
        state.db.write("test.endpoint",move|db| {
            let table=if kind=="prowlarr"{"support_services"}else{"manager_services"};
            db.execute(&format!("INSERT INTO {table}(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES (?1,?1,?1,?2,?3,'generation',?4,?5,'1',1)"),params![kind,container,port,credential,if kind=="prowlarr"{""}else{"/media"}])?;
            db.execute(&format!("UPDATE {table} SET url_base=?1 WHERE id=?2"),params![url_base,kind])?;
            Ok(())
        }).await.unwrap();
    }
    // Adapter regressions explicitly opt into each link; automatic discovery is
    // covered against real services in scripts/test-service-connections.mjs.
    action(&state, &cookie, "radarr", "disconnect").await;
    action(&state, &cookie, "sonarr", "disconnect").await;
    (
        temp,
        state,
        cookie,
        Mock {
            rows,
            posts,
            lose_response,
            tasks,
        },
    )
}

#[tokio::test]
async fn prowlarr_advertises_the_peer_route_on_three_networks_with_url_bases() {
    for prefix in ["", "/custom"] {
        let (_temp, state, cookie, mock) = connected_services_on(true, prefix).await;
        action(&state, &cookie, "radarr", "connect").await;
        tick(&state).await.unwrap();
        let link = storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.state, "connected", "{:?}", link.error);
        let source_port: u16 = state
            .db
            .read("test.port", |db| {
                Ok(db.query_row(
                    "SELECT port FROM support_services WHERE id='prowlarr'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        let target = service(&state, "radarr").await.unwrap();
        let rows = mock.rows.lock().await;
        let values = &rows[0]["fields"];
        assert!(
            values
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "prowlarrUrl"
                    && f["value"] == format!("http://127.0.0.2:{source_port}{prefix}"))
        );
        assert!(
            values
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "baseUrl"
                    && f["value"] == format!("http://127.0.0.3:{}{prefix}", target.port))
        );
    }
}

fn running(state: &AppState, character: char, value: bool) {
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", character.to_string().repeat(64)))
        .unwrap()["running"] = json!(value);
}
async fn action(state: &AppState, cookie: &str, target: &str, action: &str) {
    let response = call(
        state,
        "/api/v1/admin/service-connections",
        "POST",
        json!({"source_id":"prowlarr","target_id":target,"action":action}),
        cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
}
async fn due(state: &AppState, target: &str) {
    let mut link = storage::load(&state.db, &link_id("prowlarr", target))
        .await
        .unwrap()
        .unwrap();
    link.next_attempt = 0;
    storage::save(&state.db, link).await.unwrap();
}

#[tokio::test]
async fn existing_manual_dns_connection_is_not_duplicated_or_adopted() {
    let (_temp, state, cookie, mock) = connected_services().await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{}", "a".repeat(64)))
        .unwrap()["networks"][0]["aliases"] = json!(["my-movie-manager"]);
    mock.rows.lock().await.push(json!({"id":41,"name":"My own connection","implementation":"Radarr","fields":[{"name":"baseUrl","value":"http://my-movie-manager:7878"}]}));
    action(&state, &cookie, "radarr", "connect").await;
    tick(&state).await.unwrap();
    let link = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link.state, "conflict");
    assert!(link.upstream_id.is_none());
    assert_eq!(mock.posts.load(Ordering::SeqCst), 0);
    action(&state, &cookie, "radarr", "disconnect").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.rows.lock().await.len(), 1);
}

#[tokio::test]
async fn unavailable_target_does_not_block_another_link_and_retries_survive_runtime_restart() {
    let (_temp, mut state, cookie, mock) = connected_services().await;
    running(&state, 'a', false);
    action(&state, &cookie, "radarr", "connect").await;
    action(&state, &cookie, "sonarr", "connect").await;
    tick(&state).await.unwrap();
    let failed = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, "unavailable");
    assert!(failed.error.unwrap().contains("stopped"));
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "sonarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "connected"
    );
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    // The worker has no authoritative in-memory queue: rebuild its runtime and
    // recover the pending intent from the database alone.
    let docker = state.managers.docker.lock().unwrap().clone();
    state.managers = Arc::new(Runtime::new().unwrap());
    *state.managers.docker.lock().unwrap() = docker;
    running(&state, 'a', true);
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 2);
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "connected"
    );
    let response = call(
        &state,
        "/api/v1/admin/service-connections",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert!(!response.2.to_string().contains("fixture-manager-key"));
    let records = storage::list(&state.db).await.unwrap();
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("fixture-manager-key")
    );
}

#[tokio::test]
async fn lost_create_response_is_verified_without_duplicates_and_renaming_keeps_ownership() {
    let (_temp, state, cookie, mock) = connected_services().await;
    mock.lose_response.store(true, Ordering::SeqCst);
    action(&state, &cookie, "radarr", "connect").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "unavailable"
    );
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "connected"
    );
    mock.rows.lock().await[0]["name"] = json!("My renamed application");
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
    assert_eq!(mock.rows.lock().await[0]["name"], "My renamed application");
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "connected"
    );
}

#[tokio::test]
async fn disconnect_during_outage_cancels_reconnection_and_cleans_only_the_owned_record() {
    let (_temp, state, cookie, mock) = connected_services().await;
    action(&state, &cookie, "radarr", "connect").await;
    tick(&state).await.unwrap();
    mock.rows
        .lock()
        .await
        .push(json!({"id":99,"name":"Manual application","implementation":"Radarr","fields":[]}));
    running(&state, 'c', false);
    action(&state, &cookie, "radarr", "disconnect").await;
    tick(&state).await.unwrap();
    let pending = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert!(!pending.enabled);
    assert!(pending.cleanup);
    running(&state, 'c', true);
    running(&state, 'a', false);
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    let disconnected = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(disconnected.state, "disconnected");
    assert!(!disconnected.enabled);
    assert!(!disconnected.cleanup);
    assert_eq!(mock.rows.lock().await.len(), 1);
    assert_eq!(mock.rows.lock().await[0]["id"], 99);
    running(&state, 'a', true);
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    assert_eq!(mock.posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn manual_connection_changes_are_reported_without_overwriting_or_deleting_them() {
    let (_temp, state, cookie, mock) = connected_services().await;
    action(&state, &cookie, "radarr", "connect").await;
    tick(&state).await.unwrap();
    mock.rows.lock().await[0]["fields"][0]["value"] = json!("http://different-manager:7878");
    due(&state, "radarr").await;
    tick(&state).await.unwrap();
    assert_eq!(
        storage::load(&state.db, &link_id("prowlarr", "radarr"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "conflict"
    );
    action(&state, &cookie, "radarr", "disconnect").await;
    tick(&state).await.unwrap();
    let link = storage::load(&state.db, &link_id("prowlarr", "radarr"))
        .await
        .unwrap()
        .unwrap();
    assert!(!link.enabled);
    assert_eq!(link.state, "conflict");
    assert_eq!(
        mock.rows.lock().await[0]["fields"][0]["value"],
        "http://different-manager:7878"
    );
}
