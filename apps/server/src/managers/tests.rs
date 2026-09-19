use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[test]
fn mount_mapping_requires_the_same_physical_media_and_correct_path_boundaries() {
    let server = json!({"mounts":[{"source":"/host/media","destination":"/data"}]});
    let manager =
        json!({"mounts":[{"source":"/host/media/movies","destination":"/movies","writable":true}]});
    let result = mappings(&server, &manager, "/data").unwrap();
    assert_eq!(result[0].server, "/data/movies");
    assert_eq!(result[0].manager, "/movies");
    assert!(mappings(&server,&json!({"mounts":[{"source":"/host/media-other","destination":"/data","writable":true}]}),"/data").is_err());
    assert!(
        mappings(
            &server,
            &json!({"mounts":[{"source":"/host/media","destination":"/data","writable":false}]}),
            "/data"
        )
        .is_err()
    );
    assert!(mappings(&server, &manager, "/data/../private").is_err());
    let desktop = json!({"mounts":[{"source":"C:\\Media","destination":"/data","writable":true}]});
    assert_eq!(
        mappings(&desktop, &desktop, "/data").unwrap()[0].source,
        "c:/media"
    );
}

#[tokio::test]
async fn requests_need_approval_keep_keys_private_and_resume_without_duplicate_adds() {
    let (_temp, mut state, alice) = fixture().await;
    Arc::get_mut(&mut state.config).unwrap().media = "/data".into();
    state
        .db
        .call(|db| {
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
            get(|| async { Json(json!([{"id":1,"path":"/data/movies"}])) }),
        )
        .route(
            "/api/v3/moviefile",
            get(|| async {
                Json(json!([{"id":91,"movieId":17,"path":"/data/movies/fixture.mkv"}]))
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
                    assert_eq!(body["rootFolderPath"], "/data/movies");
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
    let inspection = json!({"id":container,"running":true,"mounts":[{"source":"/physical","destination":"/data","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]});
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
    let defaults=call(&state,&format!("/api/v1/admin/managers/{service}/defaults"),"PUT",json!({"root_folder":"/data/movies","quality_profile":1,"metadata_profile":null,"monitored":true}),&bob).await;
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
        .call(move |db| {
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
    state.db.call(|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('binding-root','Fixture','movies','/data/movies')",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('binding-file','binding-root','/data/movies/fixture.mkv','g',1,'1','hash','{}',1)",[])?;
        Ok(())
    }).await.unwrap();
    bindings::reconcile(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .call(|db| Ok(db.query_row(
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
            .call(|db| Ok(db.query_row(
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
            .call(|db| Ok(db.query_row(
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
    server.abort();
}

#[tokio::test]
async fn deletion_rechecks_keep_and_file_replacement_and_never_retries_completed_work() {
    use sha2::{Digest, Sha256};
    let (temp, state, alice) = fixture().await;
    state
        .db
        .call(|db| {
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
    state.db.call(move|db|{
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
