use super::*;
use crate::test_support::fixture;
use axum::extract::Query;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fake manager on a local port, registered as an operational Owned service whose
/// `/media` mount is the server's `media` directory.
pub(in crate::managers) async fn manager(
    state: &AppState,
    media: &std::path::Path,
    kind: &str,
    stub: Router,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let container = match kind {
        "radarr" => "a",
        "sonarr" => "b",
        _ => "c",
    }
    .repeat(64);
    let evidence = json!({"id":container,"name":"localhost","running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]});
    let mut server_evidence = evidence.clone();
    server_evidence["mounts"][0]["destination"] = json!(media.to_string_lossy());
    let media_source =
        storage_paths::evidence(&server_evidence, &evidence, &media.to_string_lossy()).unwrap();
    state.managers.docker.lock().unwrap().extend([
        ("containers/self".into(), server_evidence),
        (format!("containers/{container}"), evidence),
    ]);
    let credential = state
        .secrets
        .encrypt(&format!("manager:{kind}"), b"fixture-key")
        .unwrap();
    let kind = kind.to_owned();
    state.db.write("test.manager", move |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES (?1,?1,?1,?2,?3,'g',?4,?5,'{}','1',1,1)",params![kind,container,port,credential,media_source])?;
        crate::test_support::own(db)?;
        Ok(())
    }).await.unwrap();
    server
}

/// A present library file at `relative` under `media`, in a root of `kind`.
pub(in crate::managers) async fn library_file(
    state: &AppState,
    media: &std::path::Path,
    kind: &str,
    relative: &str,
) -> String {
    let path = media.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, relative).unwrap();
    let root = media.join(relative.split('/').next().unwrap());
    let (kind, id) = (kind.to_owned(), relative.to_owned());
    let stored = id.clone();
    state.db.write("test.library_file", move |db| {
        db.execute("INSERT OR IGNORE INTO library_roots(id,name,kind,path) VALUES (?1,?1,?1,?2)",params![kind,root.to_string_lossy()])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,?2,?3,'g',1,'1','1','{}',1)",params![stored,kind,path.to_string_lossy()])?;
        Ok(())
    }).await.unwrap();
    id
}

pub(in crate::managers) async fn ownership(state: &AppState, file: &str) -> String {
    let file = file.to_owned();
    state
        .db
        .read("test.ownership", move |db| {
            Ok(db.query_row(
                "SELECT ownership FROM media_files WHERE id=?1",
                [file],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap()
}

async fn media_state() -> (tempfile::TempDir, AppState, std::path::PathBuf) {
    let (temp, mut state, _) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    (temp, state, media)
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_library_over_the_generic_response_limit_reconciles_from_its_list_alone() {
    let (_temp, state, media) = media_state().await;
    let file_requests = Arc::new(AtomicUsize::new(0));
    let counted = file_requests.clone();
    // Long overviews push the list past the 16 MiB a generic response may buffer.
    let mut movies = (2..=4000)
        .map(|id| json!({"id":id,"tmdbId":id,"hasFile":false,"overview":"x".repeat(4600)}))
        .collect::<Vec<_>>();
    movies.push(json!({"id":1,"tmdbId":1,"hasFile":true,
        "movieFile":{"id":11,"movieId":1,"path":"/media/movies/One/one.mkv"}}));
    let body = serde_json::to_vec(&movies).unwrap();
    assert!(body.len() > 16 * 1024 * 1024);
    let stub = Router::new()
        .route(
            "/api/v3/movie",
            get(move || {
                let body = body.clone();
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        body,
                    )
                }
            }),
        )
        .route(
            "/api/v3/moviefile",
            get(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async { Json(json!([])) }
            }),
        );
    let server = manager(&state, &media, "radarr", stub).await;
    let file = library_file(&state, &media, "movies", "movies/One/one.mkv").await;
    reconcile(&state).await.unwrap();
    assert_eq!(ownership(&state, &file).await, "managed");
    assert_eq!(
        file_requests.load(Ordering::SeqCst),
        0,
        "A movie's file is listed with the movie"
    );
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn reading_managers_does_not_hold_the_manager_lock() {
    let (_temp, state, media) = media_state().await;
    let control = Arc::new(crate::test_support::FaultControl::default());
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *control.pause.lock().await = Some((entered, resume));
    let paused = control.clone();
    let stub = Router::new().route(
        "/api/v3/movie",
        get(move || {
            let paused = paused.clone();
            async move {
                paused.reached().await;
                Json(json!([{"id":1,"tmdbId":1,"hasFile":true,
                    "movieFile":{"id":11,"path":"/media/movies/one.mkv"}}]))
            }
        }),
    );
    let server = manager(&state, &media, "radarr", stub).await;
    let file = library_file(&state, &media, "movies", "movies/one.mkv").await;
    let scan = {
        let state = state.clone();
        tokio::spawn(async move { crate::managers::reconcile_after_scan(&state).await })
    };
    ready.await.unwrap();
    assert!(
        state.managers.guard.try_service("radarr").is_ok(),
        "Service changes must not wait for a library read"
    );
    release.send(()).unwrap();
    scan.await.unwrap();
    assert_eq!(ownership(&state, &file).await, "managed");
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn series_files_come_with_their_episodes() {
    for sonarr4 in [true, false] {
        let (_temp, state, media) = media_state().await;
        let episode_requests = Arc::new(AtomicUsize::new(0));
        let file_requests = Arc::new(AtomicUsize::new(0));
        let (episodes, files) = (episode_requests.clone(), file_requests.clone());
        let file = |id: i64, name: &str| json!({"id":id,"path":format!("/media/tv/Show/{name}")});
        let (first, third) = (file(701, "S01E01-E02.mkv"), file(702, "S01E03.mkv"));
        let listed = json!([first.clone(), third.clone()]);
        let stub = Router::new()
            .route(
                "/api/v3/series",
                get(|| async {
                    Json(json!([
                        {"id":7,"tvdbId":70,"statistics":{"episodeFileCount":2}},
                        {"id":8,"tvdbId":80,"statistics":{"episodeFileCount":0}}
                    ]))
                }),
            )
            .route(
                "/api/v3/episode",
                get(move |Query(query): Query<HashMap<String, String>>| {
                    episodes.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(query["seriesId"], "7");
                    let embed = sonarr4
                        && query.get("includeEpisodeFile").map(String::as_str) == Some("true");
                    let with = |file: &Value| if embed { file.clone() } else { Value::Null };
                    let rows = json!([
                        {"id":71,"episodeFileId":701,"episodeFile":with(&first)},
                        {"id":72,"episodeFileId":701,"episodeFile":with(&first)},
                        {"id":73,"episodeFileId":702,"episodeFile":with(&third)},
                        {"id":74,"episodeFileId":0}
                    ]);
                    async move { Json(rows) }
                }),
            )
            .route(
                "/api/v3/episodefile",
                get(move || {
                    files.fetch_add(1, Ordering::SeqCst);
                    let listed = listed.clone();
                    async move { Json(listed) }
                }),
            );
        let server = manager(&state, &media, "sonarr", stub).await;
        let double = library_file(&state, &media, "shows", "tv/Show/S01E01-E02.mkv").await;
        let single = library_file(&state, &media, "shows", "tv/Show/S01E03.mkv").await;
        reconcile(&state).await.unwrap();
        assert_eq!(ownership(&state, &double).await, "managed");
        assert_eq!(ownership(&state, &single).await, "managed");
        let members = state
            .db
            .read("test.members", move |db| {
                Ok(db.query_row(
                    "SELECT members FROM manager_bindings WHERE file_id=?1",
                    [double],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(members, "[71,72]");
        // One request per series with files; Sonarr 3 also lists the files separately.
        assert_eq!(episode_requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            file_requests.load(Ordering::SeqCst),
            usize::from(!sonarr4),
            "Sonarr 4 lists each episode's file with the episode"
        );
        server.abort();
    }
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn claims_are_released_once_no_owned_manager_lists_them() {
    let (_temp, state, media) = media_state().await;
    let stub = Router::new().route(
        "/api/v3/movie",
        get(|| async {
            Json(json!([{"id":1,"tmdbId":1,"hasFile":true,
                "movieFile":{"id":11,"path":"/media/movies/kept.mkv"}}]))
        }),
    );
    let server = manager(&state, &media, "radarr", stub).await;
    let kept = library_file(&state, &media, "movies", "movies/kept.mkv").await;
    let dropped = library_file(&state, &media, "movies", "movies/dropped.mkv").await;
    let retired = library_file(&state, &media, "shows", "tv/retired.mkv").await;
    let held = library_file(&state, &media, "music", "music/held.flac").await;
    let claims = [
        (dropped.clone(), "radarr"),
        (retired.clone(), "sonarr"),
        (held.clone(), "lidarr"),
    ];
    state.db.write("test.claims", move |db| {
        // Radarr no longer lists one file, Sonarr's installation was retired, and Lidarr
        // is held while its replacement is verified.
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,enabled,checked_at) VALUES ('sonarr','Sonarr','sonarr','retired',1,'g',X'00','','1',0,1)",[])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','held',1,'g',X'00','','1',1)",[])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,container_id,service_id,origin,hold,created_at,updated_at) VALUES ('held','lidarr','alice','connecting','held','lidarr','installed','setup',1,1)",[])?;
        for (file, service) in claims {
            db.execute("INSERT INTO manager_bindings VALUES (?1,?2,'g','g',5,6,'/media/old','7','[8]',?3)",params![file,service,now()])?;
        }
        Ok(())
    }).await.unwrap();
    reconcile(&state).await.unwrap();
    assert_eq!(ownership(&state, &kept).await, "managed");
    assert_eq!(ownership(&state, &dropped).await, "unmanaged");
    assert_eq!(ownership(&state, &retired).await, "unmanaged");
    assert_eq!(ownership(&state, &held).await, "unresolved");
    let remaining = state
        .db
        .read("test.remaining", |db| {
            Ok(db
                .prepare("SELECT file_id FROM manager_bindings ORDER BY file_id")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(remaining, [kept, held]);
    server.abort();
}
