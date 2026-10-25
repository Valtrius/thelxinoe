use super::*;
use crate::test_support::{call, fixture};
use axum::http::{Request, StatusCode};
use std::sync::atomic::{AtomicBool, Ordering};

async fn roots(state: &AppState) -> Vec<thelxinoe_catalog::Root> {
    thelxinoe_catalog::roots(&state.db).await.unwrap()
}

async fn scan_count(state: &AppState) -> i64 {
    state
        .db
        .read("test.scans", |db| {
            Ok(db.query_row(
                "SELECT count(*) FROM jobs WHERE kind='library.scan'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

fn errors(state: &AppState) -> HashMap<String, String> {
    state.managers.libraries.errors.read().unwrap().clone()
}

#[cfg_attr(windows, ignore = "needs Linux Docker mount paths")]
#[tokio::test]
async fn existing_managers_discover_roots_retry_and_preserve_libraries() {
    let (_temp, state, _) = fixture().await;
    let paths = Arc::new(std::sync::RwLock::new(vec!["/media/films".to_owned()]));
    let offline = Arc::new(AtomicBool::new(false));
    let mut servers = Vec::new();
    for (kind, folder) in [
        ("radarr", "films"),
        ("sonarr", "series"),
        ("lidarr", "albums"),
    ] {
        std::fs::create_dir_all(state.config.media.join(folder)).unwrap();
        let paths = paths.clone();
        let offline = offline.clone();
        let stub = Router::new().fallback(move |request: Request<axum::body::Body>| {
            assert_eq!(
                request.method(),
                "GET",
                "discovery must never change a manager"
            );
            let unavailable = kind == "radarr" && offline.load(Ordering::SeqCst);
            let paths = if kind == "radarr" {
                paths.read().unwrap().clone()
            } else {
                vec![format!("/media/{folder}")]
            };
            async move {
                if unavailable {
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(Value::Null));
                }
                assert!(request.uri().path().ends_with("/rootfolder"));
                (
                    StatusCode::OK,
                    Json(json!(
                        paths
                            .into_iter()
                            .map(|path| json!({"path":path}))
                            .collect::<Vec<_>>()
                    )),
                )
            }
        });
        servers.push(bindings::tests::manager(&state, &state.config.media, kind, stub).await);
    }
    // A plain Connected Radarr is sufficient; discovery does not require Docker ownership.
    state
        .db
        .write("test.connected", |db| {
            db.execute("DELETE FROM stack_provisions WHERE service_id='radarr'", [])?;
            db.execute(
                "UPDATE manager_services SET operational=0 WHERE id='radarr'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    refresh(&state).await.unwrap();
    let original = roots(&state).await;
    assert_eq!(original.len(), 3);
    for (kind, folder) in [
        ("movies", "films"),
        ("shows", "series"),
        ("music", "albums"),
    ] {
        assert!(
            original
                .iter()
                .any(|r| r.kind == kind
                    && r.path == state.config.media.join(folder).to_string_lossy())
        );
    }
    assert!(errors(&state).is_empty());
    refresh(&state).await.unwrap();
    assert_eq!(scan_count(&state).await, 3);

    paths.write().unwrap().extend([
        "/media/later".into(),
        "/private/films".into(),
        "/media/films".into(),
    ]);
    refresh(&state).await.unwrap();
    assert_eq!(roots(&state).await.len(), 3);
    assert!(errors(&state)["radarr"].contains("/private/films"));
    assert!(errors(&state)["radarr"].contains("/media/later"));
    std::fs::create_dir_all(state.config.media.join("later")).unwrap();
    paths
        .write()
        .unwrap()
        .retain(|path| path != "/private/films");
    refresh(&state).await.unwrap();
    assert_eq!(roots(&state).await.len(), 4);
    assert_eq!(scan_count(&state).await, 4);
    assert!(errors(&state).is_empty());

    offline.store(true, Ordering::SeqCst);
    refresh(&state).await.unwrap();
    assert!(errors(&state).contains_key("radarr"));
    assert_eq!(roots(&state).await.len(), 4);
    offline.store(false, Ordering::SeqCst);
    refresh(&state).await.unwrap();
    assert!(errors(&state).is_empty());
    let recovered = roots(&state).await;
    for root in original {
        assert!(
            recovered
                .iter()
                .any(|r| r.id == root.id && r.path == root.path)
        );
    }

    state
        .db
        .write("test.disconnected", |db| {
            db.execute(
                "UPDATE manager_services SET enabled=0, operational=0 WHERE id='radarr'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    refresh(&state).await.unwrap();
    assert_eq!(roots(&state).await.len(), 4);
    for server in servers {
        server.abort();
    }
}

#[cfg_attr(windows, ignore = "needs Linux Docker mount paths")]
#[tokio::test]
async fn downloaded_movie_appears_after_automatic_discovery_and_scan() {
    let (_temp, state, cookie) = fixture().await;
    let folder = state.config.media.join("cinema");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("Existing Movie (2024).mkv");
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=32x32:rate=1",
            "-t",
            "1",
            "-c:v",
            "ffv1",
        ])
        .arg(&file)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stub = Router::new().route("/api/v3/rootfolder", get(|| async { Json(json!([{"path":"/media/cinema"}])) }))
        .route("/api/v3/movie", get(|| async { Json(json!([{"id":1,"tmdbId":603,"title":"Existing Movie","year":2024,"hasFile":true,"movieFile":{"id":10,"path":"/media/cinema/Existing Movie (2024).mkv"}}])) }));
    let server = bindings::tests::manager(&state, &state.config.media, "radarr", stub).await;
    assert!(roots(&state).await.is_empty());
    refresh(&state).await.unwrap();
    let queue = thelxinoe_jobs::Queue(state.db.clone());
    let job = queue.claim_general().await.unwrap().unwrap();
    assert_eq!(job.kind, "library.scan");
    crate::run_job(state.clone(), job).await;
    assert!(roots(&state).await[0].scan_error.is_some());
    crate::test_support::tool_runtime::install(
        &state.tools.runtime,
        &state.config.state.join("tools"),
    );
    refresh(&state).await.unwrap();
    let retry = queue.claim_general().await.unwrap().unwrap();
    assert_eq!(retry.kind, "library.scan");
    crate::run_job(state.clone(), retry).await;
    let response = call(
        &state,
        "/api/v1/catalog?kind=movie",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let items = response.2["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{}", response.2);
    assert_eq!(items[0]["available"], true);
    assert_eq!(items[0]["title"], "Existing Movie");
    refresh(&state).await.unwrap();
    assert_eq!(scan_count(&state).await, 2);
    assert!(roots(&state).await[0].scan_error.is_none());
    server.abort();
}

#[tokio::test]
async fn local_libraries_are_automatic_and_existing_roots_are_preserved() {
    let (_temp, state, _) = fixture().await;
    for folder in ["movies", "tv", "music", "downloads"] {
        std::fs::create_dir_all(state.config.media.join(folder)).unwrap();
    }
    let movie_path = state
        .config
        .media
        .join("movies")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    state.db.write("test.legacy_root", move |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path,last_scan) VALUES ('legacy','My movies','movies',?1,123)", [movie_path])?;
        Ok(())
    }).await.unwrap();
    refresh(&state).await.unwrap();
    refresh(&state).await.unwrap();
    let roots = roots(&state).await;
    assert_eq!(roots.len(), 3);
    let legacy = roots.iter().find(|r| r.id == "legacy").unwrap();
    assert_eq!(legacy.name, "My movies");
    assert_eq!(legacy.last_scan, Some(123));
    assert_eq!(scan_count(&state).await, 2);
    let child = std::path::Path::new(&legacy.path)
        .join("child")
        .to_string_lossy()
        .into_owned();
    storage::ensure_root(&state.db, None, "movies".into(), child.clone())
        .await
        .unwrap();
    assert!(
        storage::ensure_root(&state.db, None, "shows".into(), child)
            .await
            .is_err()
    );
    assert!(
        storage::ensure_root(
            &state.db,
            None,
            "movies".into(),
            state
                .config
                .media
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
        .await
        .is_err()
    );
    assert!(
        storage::ensure_root(
            &state.db,
            Some(("removed".into(), "old".into())),
            "movies".into(),
            "/ignored".into()
        )
        .await
        .is_err()
    );
    assert_eq!(scan_count(&state).await, 2);
}

#[cfg(unix)]
#[tokio::test]
async fn mapped_directories_reject_escapes_and_support_named_volume_subpaths() {
    let (temp, state, _) = fixture().await;
    std::fs::create_dir_all(state.config.media.join("films")).unwrap();
    std::fs::create_dir_all(temp.path().join("private")).unwrap();
    std::os::unix::fs::symlink(
        temp.path().join("private"),
        state.config.media.join("escape"),
    )
    .unwrap();
    let media = state.config.media.to_string_lossy();
    let service = json!({"mounts":[{"kind":"volume","name":"shared","source":"/volume/shared","destination":"/movies","subpath":"films"}]});
    let server = json!({"mounts":[{"kind":"volume","name":"shared","source":"/volume/shared","destination":media,"subpath":""}]});
    let evidence = storage_paths::evidence(&server, &service, &media).unwrap();
    assert_eq!(
        storage_paths::directory(&state, &evidence, "/movies")
            .await
            .unwrap(),
        Some(
            state
                .config
                .media
                .join("films")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert!(
        storage_paths::resolve(&state, &evidence, "/movies")
            .await
            .is_err()
    );
    let service = json!({"mounts":[{"kind":"bind","source":"/disk","destination":"/data"}]});
    let server = json!({"mounts":[{"kind":"bind","source":"/disk","destination":media}]});
    let evidence = storage_paths::evidence(&server, &service, &media).unwrap();
    assert!(
        storage_paths::directory(&state, &evidence, "/data/escape")
            .await
            .is_err()
    );
    std::os::unix::fs::symlink(
        state.config.media.join("films"),
        state.config.media.join("alias"),
    )
    .unwrap();
    let canonical = state
        .config
        .media
        .join("films")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        storage_paths::directory(&state, &evidence, "/data/alias")
            .await
            .unwrap(),
        Some(canonical.clone())
    );
    let file = state.config.media.join("films/Movie.mkv");
    std::fs::write(&file, b"fixture").unwrap();
    assert_eq!(
        storage_paths::resolve(&state, &evidence, "/data/alias/Movie.mkv")
            .await
            .unwrap(),
        Some(file.to_string_lossy().into_owned())
    );
    for path in ["/data/films", "/data/alias"] {
        let path = storage_paths::directory(&state, &evidence, path)
            .await
            .unwrap()
            .unwrap();
        storage::ensure_root(&state.db, None, "movies".into(), path)
            .await
            .unwrap();
    }
    assert_eq!(roots(&state).await.len(), 1);
    assert_eq!(scan_count(&state).await, 1);
    assert!(
        storage_paths::directory(&state, &evidence, "/data/../private")
            .await
            .is_err()
    );
    let server = json!({"mounts":[{"kind":"bind","source":"/disk","destination":media},{"kind":"bind","source":"/other","destination":state.config.media.join("films")}]});
    let evidence = storage_paths::evidence(&server, &service, &media).unwrap();
    assert_eq!(
        storage_paths::directory(&state, &evidence, "/data/films")
            .await
            .unwrap(),
        None
    );
}
