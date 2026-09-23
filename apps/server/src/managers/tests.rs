use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[test]
fn integrations_require_the_same_single_writable_media_bind() {
    let server = json!({"mounts":[{"kind":"bind","source":"/nas/media","destination":"/media","writable":true}]});
    assert_eq!(
        shared_media_source(&server, &server, "/media").unwrap(),
        "/nas/media"
    );
    for (field, value) in [
        ("source", json!("/nas/other")),
        ("destination", json!("/movies")),
        ("kind", json!("volume")),
        ("writable", json!(false)),
    ] {
        let mut manager = server.clone();
        manager["mounts"][0][field] = value;
        assert!(shared_media_source(&server, &manager, "/media").is_err());
        assert!(shared_media_source(&manager, &server, "/media").is_err());
    }
    let mut nested = server.clone();
    nested["mounts"].as_array_mut().unwrap().push(
        json!({"kind":"bind","source":"/nas/other","destination":"/media/movies","writable":true}),
    );
    assert!(shared_media_source(&server, &nested, "/media").is_err());
    assert!(shared_media_source(&nested, &server, "/media").is_err());
    assert!(shared_media_source(&server, &server, "/other").is_err());
    let desktop = json!({"mounts":[{"kind":"bind","source":"C:\\Media","destination":"/media","writable":true}]});
    assert_eq!(
        shared_media_source(&desktop, &desktop, "/media").unwrap(),
        "c:/media"
    );
}
#[test]
fn managers_must_use_the_canonical_root_for_their_media_kind() {
    for (kind, expected) in [
        ("radarr", "/media/movies"),
        ("sonarr", "/media/tv"),
        ("lidarr", "/media/music"),
    ] {
        assert!(validate_roots(kind, &json!([])).is_ok());
        assert!(validate_roots(kind, &json!([{"path":expected}])).is_ok());
        assert!(validate_roots(kind, &json!([{"path":"/movies"}])).is_err());
        assert!(validate_roots(kind, &json!([{"path":expected},{"path":"/media/other"}])).is_err());
    }
}

#[tokio::test]
async fn requests_need_approval_keep_keys_private_and_resume_without_duplicate_adds() {
    let (_temp, mut state, alice) = fixture().await;
    Arc::get_mut(&mut state.config).unwrap().media = "/media".into();
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let bob = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
        .await
        .unwrap();
    let bob = format!("thelxinoe_session={bob}");
    let movies = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let post_movies = movies.clone();
    let get_movies = movies.clone();
    let adds = Arc::new(AtomicUsize::new(0));
    let add_count = adds.clone();
    let searches = Arc::new(AtomicUsize::new(0));
    let search_count = searches.clone();
    let stub = Router::new()
        .route(
            "/api/v3/system/status",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers["x-api-key"], "fixture-manager-secret-key");
                Json(json!({"appName":"Radarr","version":"6.0"}))
            }),
        )
        .route(
            "/api/v3/rootfolder",
            get(|| async { Json(json!([{"id":1,"path":"/media/movies"}])) }),
        )
        .route(
            "/api/v3/moviefile",
            get(|| async {
                Json(json!([{"id":91,"movieId":17,"path":"/media/movies/fixture.mkv"}]))
            }),
        )
        .route(
            "/api/v3/qualityprofile",
            get(|| async { Json(json!([{"id":1,"name":"HD"}])) }),
        )
        .route(
            "/api/v3/movie/lookup",
            get(|| async { Json(json!([{"title":"Public fixture","tmdbId":603}])) }),
        )
        .route(
            "/api/v3/movie",
            get(move || {
                let m = get_movies.clone();
                async move { Json(json!(*m.lock().await)) }
            })
            .post(move |Json(mut body): Json<Value>| {
                let m = post_movies.clone();
                let count = add_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(body["rootFolderPath"], "/media/movies");
                    assert_eq!(body["qualityProfileId"], 1);
                    assert_eq!(body["addOptions"]["searchForMovie"], false);
                    body["id"] = json!(17);
                    m.lock().await.push(body.clone());
                    Json(body)
                }
            }),
        )
        .route(
            "/api/v3/command",
            post(move |Json(body): Json<Value>| {
                let count = search_count.clone();
                async move {
                    assert_eq!(body["name"], "MoviesSearch");
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"id":33}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let container = "a".repeat(64);
    let inspection = json!({"id":container,"running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]});
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("containers/self".into(), inspection.clone());
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert(format!("containers/{container}"), inspection);
    let input = json!({"name":"Fixture","kind":"radarr","container_id":container,"port":port,"api_key":"fixture-manager-secret-key"});
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/managers",
            "POST",
            input.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let registered = call(&state, "/api/v1/admin/managers", "POST", input, &bob).await;
    assert_eq!(registered.0, StatusCode::OK, "{}", registered.2);
    let service = registered.2["id"].as_str().unwrap();
    let replacement = "c".repeat(64);
    state.managers.docker.lock().unwrap().insert(
        format!("containers/{replacement}"),
        json!({"id":replacement,"running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]}),
    );
    let replaced = call(
        &state,
        "/api/v1/admin/managers",
        "POST",
        json!({"name":"Replacement","kind":"radarr","container_id":replacement,"port":port,"api_key":"fixture-manager-secret-key"}),
        &bob,
    )
    .await;
    assert_eq!(replaced.0, StatusCode::OK, "{}", replaced.2);
    assert_eq!(replaced.2["id"], service);
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM manager_services WHERE kind='radarr'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        1
    );
    let container = replacement;
    let metadata = call(
        &state,
        "/api/v1/metadata/search?kind=movie&q=Public",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(metadata.0, StatusCode::OK, "{}", metadata.2);
    assert_eq!(metadata.2["items"][0]["service_id"], service);
    assert!(metadata.2["items"][0]["service_generation"].is_string());
    assert_eq!(metadata.2["items"][0]["external_id"], "603");
    let defaults=call(&state,&format!("/api/v1/admin/managers/{service}/defaults"),"PUT",json!({"root_folder":"/media/movies","quality_profile":1,"metadata_profile":null,"monitored":true}),&bob).await;
    assert_eq!(defaults.0, StatusCode::OK, "{}", defaults.2);
    let listing = call(&state, "/api/v1/admin/managers", "GET", Value::Null, &bob).await;
    assert!(!listing.2.to_string().contains("secret-key"));
    let input = json!({"service_id":service,"external_id":"603"});
    let request = call(
        &state,
        "/api/v1/acquisition/requests",
        "POST",
        input.clone(),
        &alice,
    )
    .await;
    assert_eq!(request.0, StatusCode::OK, "{}", request.2);
    assert_eq!(request.2["state"], "pending");
    let duplicate = call(
        &state,
        "/api/v1/acquisition/requests",
        "POST",
        input,
        &alice,
    )
    .await;
    assert_eq!(request.2["id"], duplicate.2["id"]);
    let key = request.2["id"].as_str().unwrap();
    let queue = thelxinoe_jobs::Queue(state.db.clone());
    assert!(queue.claim().await.unwrap().is_none());
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/acquisition/requests/{key}"),
            "POST",
            json!({"action":"approve"}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/acquisition/requests/{key}"),
            "POST",
            json!({"action":"approve"}),
            &bob
        )
        .await
        .0,
        StatusCode::OK
    );
    let job = queue.claim().await.unwrap().unwrap();
    acquire(&state, &job).await.unwrap();
    acquire(&state, &job).await.unwrap();
    assert_eq!(adds.load(Ordering::SeqCst), 1);
    assert_eq!(searches.load(Ordering::SeqCst), 1);
    let rows = call(
        &state,
        "/api/v1/acquisition/requests",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(rows.2["items"][0]["state"], "requested");
    let request_id = key.to_owned();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "UPDATE acquisition_requests SET state='searching' WHERE id=?1",
                [request_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(acquire(&state, &job).await.is_err());
    assert_eq!(
        searches.load(Ordering::SeqCst),
        1,
        "A search with an uncertain outcome must not be repeated automatically"
    );
    let rows = call(
        &state,
        "/api/v1/acquisition/requests",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(rows.2["items"][0]["state"], "uncertain");
    state.db.write("test.fixture", |db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('binding-root','Fixture','movies','/media/movies')",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('binding-file','binding-root','/media/movies/fixture.mkv','g',1,'1','hash','{}',1)",[])?;
        Ok(())
    }).await.unwrap();
    bindings::reconcile(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT ownership FROM media_files WHERE id='binding-file'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await
            .unwrap(),
        "managed"
    );
    // A changed Docker mount invalidates even an otherwise healthy manager connection.
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{container}"))
        .unwrap()["mounts"][0]["source"] = json!("/different");
    bindings::reconcile(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT ownership FROM media_files WHERE id='binding-file'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await
            .unwrap(),
        "unresolved"
    );
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM manager_bindings WHERE file_id='binding-file'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        1,
        "An outage must preserve previous ownership evidence"
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/managers/{service}/test"),
            "POST",
            json!({}),
            &bob
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .get_mut(&format!("containers/{container}"))
        .unwrap()["mounts"][0]["source"] = json!("/physical");
    let provision_container = container.clone();
    state.db.write("test.fixture", move |db|{
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,created_at,updated_at) VALUES ('manager-provision','radarr','bob',7878,X'00','queued',?1,1,1)",[provision_container])?;
        Ok(())
    }).await.unwrap();
    let blocked = call(
        &state,
        "/api/v1/admin/managers",
        "POST",
        json!({"name":"Blocked","kind":"radarr","container_id":container,"port":port,"api_key":"fixture-manager-secret-key"}),
        &bob,
    )
    .await;
    assert_eq!(blocked.0, StatusCode::CONFLICT, "{}", blocked.2);
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE stack_provisions SET state='connecting' WHERE id='manager-provision'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let internal = call(
        &state,
        "/api/v1/admin/managers",
        "POST",
        json!({"name":"Managed","kind":"radarr","container_id":container,"port":port,"api_key":"fixture-manager-secret-key"}),
        &bob,
    )
    .await;
    assert_eq!(internal.0, StatusCode::OK, "{}", internal.2);
    server.abort();
}

#[tokio::test]
async fn deletion_rechecks_keep_and_file_replacement_and_never_retries_completed_work() {
    use sha2::{Digest, Sha256};
    let (temp, state, alice) = fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let directory = temp.path().join("media");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("fixture.mp4");
    std::fs::write(&path, b"original media bytes").unwrap();
    let path = path.canonicalize().unwrap();
    let root = directory.canonicalize().unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    let modified = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let hash = Sha256::digest(b"original media bytes")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let stored = path.to_string_lossy().to_string();
    state.db.write("test.fixture", move|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Fixture','movies',?1)",[root.to_string_lossy().as_ref()])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('movie','root','movie','fixture','Fixture',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('file','root',?1,'generation',?2,?3,?4,'{}',1)",params![stored,meta.len() as i64,modified,hash])?;
        db.execute("INSERT INTO media_sources VALUES ('movie','file')",[])?;Ok(())
    }).await.unwrap();
    let body = json!({"media_id":"movie","action":"delete"});
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/media/operations",
            "POST",
            body.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let prepared = call(
        &state,
        "/api/v1/admin/media/operations",
        "POST",
        body,
        &admin,
    )
    .await;
    assert_eq!(prepared.0, StatusCode::OK, "{}", prepared.2);
    let execute = format!(
        "/api/v1/admin/media/operations/{}/execute",
        prepared.2["id"].as_str().unwrap()
    );
    call(
        &state,
        "/api/v1/admin/media/movie/keep",
        "PUT",
        json!({"keep":true}),
        &admin,
    )
    .await;
    assert_eq!(
        call(&state, &execute, "POST", json!({}), &admin).await.0,
        StatusCode::CONFLICT
    );
    assert!(path.exists());
    call(
        &state,
        "/api/v1/admin/media/movie/keep",
        "PUT",
        json!({"keep":false}),
        &admin,
    )
    .await;
    let original_time = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, b"replaced media bytes").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_time)
        .unwrap();
    assert_eq!(
        call(&state, &execute, "POST", json!({}), &admin).await.0,
        StatusCode::CONFLICT,
        "Same size and mtime do not excuse a fingerprint mismatch"
    );
    assert!(path.exists());
    std::fs::write(&path, b"original media bytes").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_time)
        .unwrap();
    let result = call(&state, &execute, "POST", json!({}), &admin).await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.2);
    assert!(!path.exists());
    assert_eq!(
        call(&state, &execute, "POST", json!({}), &admin).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn support_services_are_admin_only_redacted_and_expose_only_allowed_commands() {
    let (_temp, mut state, alice) = fixture().await;
    Arc::get_mut(&mut state.config).unwrap().media = "/media".into();
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let commands = Arc::new(AtomicUsize::new(0));
    let count = commands.clone();
    let stub=Router::new().route("/jsonrpc",post(move|headers:HeaderMap,Json(body):Json<Value>|{let count=count.clone();async move{
        assert!(headers["authorization"].to_str().unwrap().starts_with("Basic "));
        let result=match body["method"].as_str().unwrap(){
            "version"=>json!("26.3"),"status"=>json!({"DownloadPaused":true,"DownloadRate":0,"DownloadLimit":0}),
            "history"=>{assert_eq!(body["params"],json!([true]));json!([{"NZBID":8,"Name":"Completed fixture","Status":"SUCCESS/HIDDEN"}])},
            "listgroups"=>json!([{"NZBID":7,"NZBName":"Fixture","Status":"PAUSED","URL":"https://provider.invalid/?apikey=private-indexer-secret","Parameters":[{"Name":"password","Value":"private-download-secret"}]}]),
            "pausedownload"=>{count.fetch_add(1,Ordering::SeqCst);json!(true)},
            _=>panic!("Unexpected RPC method")};Json(json!({"result":result,"id":1}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let container = "b".repeat(64);
    let inspection = json!({"id":container,"running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]});
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("containers/self".into(), inspection.clone());
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert(format!("containers/{container}"), inspection);
    let input = json!({"name":"Fixture downloader","kind":"nzbget","container_id":container,"port":port,"credentials":{"username":"fixture","secret":"private-test-secret"},"native_url":"http://localhost:6789"});
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/support",
            "POST",
            input.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let response = call(&state, "/api/v1/admin/support", "POST", input, &admin).await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let key = response.2["id"].as_str().unwrap();
    let login_path = format!("/api/v1/admin/support/{key}/login");
    assert_eq!(
        call(&state, &login_path, "POST", Value::Null, &alice)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&state, &login_path, "GET", Value::Null, &admin)
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let login = call(&state, &login_path, "POST", Value::Null, &admin).await;
    assert_eq!(login.0, StatusCode::OK);
    assert_eq!(
        login.2,
        json!({"username":"fixture","password":"private-test-secret"})
    );
    let replacement = "e".repeat(64);
    state.managers.docker.lock().unwrap().insert(
        format!("containers/{replacement}"),
        json!({"id":replacement,"running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]}),
    );
    let replaced = call(
        &state,
        "/api/v1/admin/support",
        "POST",
        json!({"name":"Replacement downloader","kind":"nzbget","container_id":replacement,"port":port,"credentials":{"username":"fixture","secret":"private-test-secret"},"native_url":"http://localhost:6789"}),
        &admin,
    )
    .await;
    assert_eq!(replaced.0, StatusCode::OK, "{}", replaced.2);
    assert_eq!(replaced.2["id"], key);
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM support_services WHERE kind='nzbget'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        1
    );
    let path = format!("/api/v1/admin/support/{key}");
    assert_eq!(
        call(&state, &path, "GET", Value::Null, &alice).await.0,
        StatusCode::FORBIDDEN
    );
    let view = call(&state, &path, "GET", Value::Null, &admin).await;
    assert_eq!(view.0, StatusCode::OK);
    assert!(!view.2.to_string().contains("private"));
    assert_eq!(view.2["queue"][0]["id"], 7);
    assert_eq!(view.2["history"][0]["title"], "Completed fixture");
    assert_eq!(
        call(&state, &path, "POST", json!({"action":"shutdown"}), &admin)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &path,
            "POST",
            json!({"action":"remove","item_id":99}),
            &admin
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(commands.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&state, &path, "POST", json!({"action":"pause_all"}), &admin)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(commands.load(Ordering::SeqCst), 1);
    let stored = state
        .db
        .write("test.fixture", |db| {
            Ok(
                db.query_row("SELECT credential FROM support_services", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("private-test-secret"));
    let provision_container = replacement.clone();
    state.db.write("test.fixture", move |db|{
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,created_at,updated_at) VALUES ('support-provision','nzbget','bob',6789,X'00','queued',?1,1,1)",[provision_container])?;
        Ok(())
    }).await.unwrap();
    let blocked = call(
        &state,
        "/api/v1/admin/support",
        "POST",
        json!({"name":"Blocked downloader","kind":"nzbget","container_id":replacement,"port":port,"credentials":{"username":"fixture","secret":"private-test-secret"},"native_url":"http://localhost:6789"}),
        &admin,
    )
    .await;
    assert_eq!(blocked.0, StatusCode::CONFLICT, "{}", blocked.2);
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE stack_provisions SET state='connecting' WHERE id='support-provision'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let internal = call(
        &state,
        "/api/v1/admin/support",
        "POST",
        json!({"name":"Managed downloader","kind":"nzbget","container_id":replacement,"port":port,"credentials":{"username":"fixture","secret":"private-test-secret"},"native_url":"http://localhost:6789"}),
        &admin,
    )
    .await;
    assert_eq!(internal.0, StatusCode::OK, "{}", internal.2);
    task.abort();
}

#[tokio::test]
async fn nzbget_login_is_available_to_admins_while_provisioning() {
    let (_temp, state, alice) = fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let key = "queued-nzbget";
    let credential = state
        .secrets
        .encrypt(&format!("provision:{key}"), b"generated-private-password")
        .unwrap();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at) VALUES (?1,'nzbget','bob',16789,?2,'queued',1,1)",
                rusqlite::params![key, credential],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let path = format!("/api/v1/admin/stack/{key}/login");
    assert_eq!(
        call(&state, &path, "POST", Value::Null, &alice).await.0,
        StatusCode::FORBIDDEN
    );
    let login = call(&state, &path, "POST", Value::Null, &admin).await;
    assert_eq!(login.0, StatusCode::OK);
    assert_eq!(
        login.2,
        json!({"username":"thelxinoe","password":"generated-private-password"})
    );
    let adopted = state
        .secrets
        .encrypt(
            &format!("provision:{key}"),
            br#"{"username":"existing","secret":"existing-private-password"}"#,
        )
        .unwrap();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "UPDATE stack_provisions SET origin='adopted',credential=?1 WHERE id=?2",
                rusqlite::params![adopted, key],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let login = call(&state, &path, "POST", Value::Null, &admin).await;
    assert_eq!(login.0, StatusCode::OK);
    assert_eq!(
        login.2,
        json!({"username":"existing","password":"existing-private-password"})
    );
    let other_key = "queued-radarr";
    let other_credential = state
        .secrets
        .encrypt(&format!("provision:{other_key}"), b"radarr-private-key")
        .unwrap();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at) VALUES (?1,'radarr','bob',17878,?2,'queued',1,1)",
                rusqlite::params![other_key, other_credential],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/stack/{other_key}/login"),
            "POST",
            Value::Null,
            &admin,
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn provisioning_is_admin_only_durable_and_never_puts_credentials_in_jobs() {
    let (_temp, state, alice) = fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let input =
        json!({"kind":"radarr","host_port":37878,"native_url":"https://radarr.example.test"});
    for (path, payload) in [
        (
            "/api/v1/admin/stack/adopt/preview".to_owned(),
            json!({"service_id":"fixture"}),
        ),
        (
            "/api/v1/admin/stack/adopt".to_owned(),
            json!({"service_id":"fixture","review_id":id(),"released_compose":true}),
        ),
        (
            format!("/api/v1/admin/stack/{}/restore-original", id()),
            json!({}),
        ),
    ] {
        assert_eq!(
            call(&state, &path, "POST", payload, &alice).await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/stack/install",
            "POST",
            input.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/stack/install",
            "POST",
            json!({"kind":"arbitrary/image","host_port":37878}),
            &admin
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('existing-radarr','Existing Radarr','radarr','existing-container',7878,'generation',X'00','/media','1',1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/stack/install",
            "POST",
            input.clone(),
            &admin,
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "DELETE FROM manager_services WHERE id='existing-radarr'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let result = call(
        &state,
        "/api/v1/admin/stack/install",
        "POST",
        input.clone(),
        &admin,
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(
        call(&state, "/api/v1/admin/stack/install", "POST", input, &admin)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (key,encrypted,payload)=state.db.write("test.fixture", |db|Ok(db.query_row("SELECT p.id,p.credential,j.payload FROM stack_provisions p JOIN jobs j ON json_extract(j.payload,'$.id')=p.id",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?)))?)).await.unwrap();
    let plain = state
        .secrets
        .decrypt(&format!("provision:{key}"), &encrypted)
        .unwrap();
    let plain = String::from_utf8(plain).unwrap();
    assert!(!payload.contains(&plain));
    assert!(!result.2.to_string().contains(&plain));
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload, json!({"id":key}));
    let policy = json!({"policy":"automatic","window_start":22,"window_end":3});
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/product-update/policy",
            "POST",
            policy.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/product-update/policy",
            "POST",
            policy,
            &admin
        )
        .await
        .0,
        StatusCode::OK
    );
    let provision = key.clone();
    state.db.write("test.fixture", move|db|{db.execute("UPDATE stack_provisions SET state='complete',service_id='fixture-integration' WHERE id=?1",[provision])?;Ok(())}).await.unwrap();
    let update_settings = call(
        &state,
        "/api/v1/admin/service-updates",
        "GET",
        Value::Null,
        &admin,
    )
    .await;
    assert_eq!(update_settings.0, StatusCode::OK);
    assert_eq!(
        update_settings.2["server_policy"],
        json!({"policy":"automatic","window_start":22,"window_end":3})
    );
    assert!(
        update_settings.2["policies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["service_id"] != "default")
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/admin/service-updates/policy/{key}"),
            "POST",
            json!({"policy":"inherit","window_start":0,"window_end":0}),
            &admin
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut attempts = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let state = state.clone();
        let admin = admin.clone();
        let path = format!("/api/v1/admin/service-updates/preflight/{key}");
        attempts.spawn(async move { call(&state, &path, "POST", json!({}), &admin).await.0 });
    }
    let mut accepted = 0;
    while let Some(result) = attempts.join_next().await {
        let status = result.unwrap();
        assert!(status == StatusCode::OK || status == StatusCode::CONFLICT);
        if status == StatusCode::OK {
            accepted += 1;
        }
    }
    assert_eq!(
        accepted, 1,
        "Concurrent requests must enqueue exactly one update"
    );
}
