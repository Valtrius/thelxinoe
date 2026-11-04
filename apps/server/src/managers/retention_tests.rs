use super::*;
use crate::test_support::fixture;

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn batch_inventory_is_shared_and_does_not_exclude_unrelated_playback() {
    for change in ["none", "service", "candidate-write"] {
        let (temp, mut state, cookie) = fixture().await;
        let media = temp.path().join("media");
        std::fs::create_dir_all(media.join("movies")).unwrap();
        for i in 1..=5 {
            std::fs::write(media.join(format!("movies/{i}.mp4")), b"fixture media").unwrap();
        }
        Arc::get_mut(&mut state.config).unwrap().media = media.clone();
        let control = Arc::new(crate::test_support::FaultControl::default());
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        *control.pause.lock().await = Some((entered, resume));
        let calls = control.clone();
        let file_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let files = file_calls.clone();
        let stub = Router::new()
            .route("/api/v3/movie", get(move || {
                let calls = calls.clone();
                async move {
                    calls.reached().await;
                    Json(json!((1..=5).map(|i| json!({"id":i,"tmdbId":i,"hasFile":true,
                        "movieFile":{"id":i,"movieId":i,"path":format!("/media/movies/{i}.mp4")}})).collect::<Vec<_>>()))
                }
            }))
            .route("/api/v3/moviefile", get(move |axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String,String>>| {
                let files = files.clone();
                async move {
                    files.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let i = query["movieId"].parse::<i64>().unwrap();
                    Json(json!([{"id":i,"movieId":i,"path":format!("/media/movies/{i}.mp4")}]))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
        let container = "a".repeat(64);
        let evidence = json!({"id":container,"name":"localhost","running":true,"mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],"networks":[{"id":"network","address":"127.0.0.1"}]});
        let mut server_evidence = evidence.clone();
        server_evidence["mounts"][0]["destination"] = json!(media.to_string_lossy());
        let media_source = crate::managers::storage_paths::evidence(
            &server_evidence,
            &evidence,
            &media.to_string_lossy(),
        )
        .unwrap();
        state.managers.docker.lock().unwrap().extend([
            ("containers/self".into(), server_evidence),
            (format!("containers/{container}"), evidence),
        ]);
        let credential = state
            .secrets
            .encrypt("manager:radarr", b"fixture-key")
            .unwrap();
        let other = temp.path().join("other.mp4");
        std::fs::write(&other, b"unrelated playback fixture").unwrap();
        let other = other.canonicalize().unwrap();
        let meta = std::fs::metadata(&other).unwrap();
        let modified = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        state.db.write("test.batch", move |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('batch','Batch','movies',?1)", [media.join("movies").to_string_lossy()])?;
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('other-root','Other','movies',?1)",[other.parent().unwrap().to_string_lossy()])?;
            db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES ('radarr','Fixture','radarr',?1,?2,'g',?3,?4,'{}','1',1,1)",params![container,port,credential,media_source])?;
            crate::test_support::own(db)?;
            for i in 1..=5 {
                let key = format!("batch-{i}");
                db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES (?1,'batch','movie',?1,?1,1)",[&key])?;
                db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES (?1,'batch',?2,'first',1,'1','1','{}','unmanaged',1)",params![key,media.join(format!("movies/{i}.mp4")).to_string_lossy()])?;
                db.execute("INSERT INTO media_sources VALUES (?1,?1)",[&key])?;
                db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice',?1,1,1)",[&key])?;
            }
            db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('other','other-root','movie','other','Other',1)", [])?;
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES ('other','other-root',?1,'first',?2,?3,'1',?4,'unmanaged',1)",params![other.to_string_lossy(),meta.len() as i64,modified,json!({"format":{"duration":"120"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264"}]}).to_string()])?;
            db.execute("INSERT INTO media_sources VALUES ('other','other')", [])?;
            db.execute("INSERT INTO settings(key,value) VALUES ('manager.retention.radarr','true')", [])?;
            db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\"]',grace_seconds=3600 WHERE domain='movies'", [])?;
            Ok(())
        }).await.unwrap();
        let evaluation = {
            let state = state.clone();
            tokio::spawn(async move { evaluate_all(&state, None).await })
        };
        ready.await.unwrap();
        let playback = tokio::time::timeout(std::time::Duration::from_secs(2), crate::test_support::call(&state,"/api/v1/playback","POST",json!({"media_id":"other","options":{"quality":"auto","capabilities":{"containers":["mp4"],"video":["h264"],"audio":[],"hls":false}}}),&cookie)).await;
        let progress = if let Ok((_, _, body)) = &playback {
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                crate::test_support::call(
                    &state,
                    &format!("/api/v1/playback/{}/progress", body["id"].as_str().unwrap()),
                    "POST",
                    json!({"sequence":1,"position":1,"state":"playing"}),
                    &cookie,
                ),
            )
            .await
            .ok()
        } else {
            None
        };
        if change == "service" {
            state
                .db
                .write("test.change_service", |db| {
                    db.execute(
                        "UPDATE manager_services SET generation='new' WHERE id='radarr'",
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        } else if change == "candidate-write" {
            state.db.write("test.reject_candidate", |db| { db.execute_batch("CREATE TRIGGER reject_candidate BEFORE INSERT ON retention_candidates BEGIN SELECT RAISE(ABORT,'injected candidate failure'); END;")?; Ok(()) }).await.unwrap();
        }
        release.send(()).unwrap();
        let result = evaluation.await.unwrap();
        server.abort();
        assert_eq!(
            playback
                .expect("inventory must not block unrelated playback")
                .0,
            axum::http::StatusCode::OK
        );
        assert_eq!(progress.unwrap().0, axum::http::StatusCode::OK);
        if change == "none" {
            assert_eq!(result.unwrap(), 5);
            assert_eq!(control.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(
                file_calls.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "The movie list carries each movie's file"
            );
        } else {
            assert!(result.is_err());
            let count = state
                .db
                .read("test.no_orphans", |db| {
                    Ok(
                        db.query_row("SELECT COUNT(*) FROM media_operations", [], |r| {
                            r.get::<_, i64>(0)
                        })?,
                    )
                })
                .await
                .unwrap();
            assert_eq!(
                count, 0,
                "failed proposals must not leave pending operations"
            );
        }
    }
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn retention_never_reads_music_libraries() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (temp, mut state, _) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let stub = Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { Json(json!([])) }
    });
    let server = bindings::tests::manager(&state, &media, "lidarr", stub).await;
    let track =
        bindings::tests::library_file(&state, &media, "music", "music/Artist/Album/01.flac").await;
    let managed = track.clone();
    state.db.write("test.music", move |db| {
        db.execute("UPDATE media_files SET ownership='managed' WHERE id=?1", [managed])?;
        db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\"]' WHERE domain='movies'", [])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 0);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        bindings::tests::ownership(&state, &track).await,
        "managed",
        "Music keeps what its own reconciliation decided"
    );
    server.abort();
}

#[tokio::test]
async fn validation_of_one_file_keeps_unrelated_playback_responsive_and_hashes_once() {
    let (_temp, state, path) = movie().await;
    let other = path.with_file_name("other.mp4");
    std::fs::copy(&path, &other).unwrap();
    let modified = std::fs::metadata(&other)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    state.db.write("test.other_media", move |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('other','ret-root','movie','other','Other',1)", [])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) SELECT 'other','ret-root',?1,'other',size,?2,fingerprint,?3,'unmanaged',1 FROM media_files WHERE id='ret-file'",params![other.to_string_lossy(), modified,json!({"format":{"duration":"120"},"streams":[{"index":0,"codec_type":"video","codec_name":"h264"}]}).to_string()])?;
        db.execute("INSERT INTO media_sources VALUES ('other','other')", [])?;
        Ok(())
    }).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "alice".into(), "web".into(), "test".into())
            .await
            .unwrap();
    let cookie = format!("thelxinoe_session={token}");
    let operation =
        operations::prepare_operation(&state, None, "ret-movie".into(), "delete".into())
            .await
            .unwrap()
            .0["id"]
            .as_str()
            .unwrap()
            .to_owned();
    let control = Arc::new(crate::test_support::FaultControl::default());
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *control.pause.lock().await = Some((entered, resume));
    let work = {
        let state = state.clone();
        let control = control.clone();
        tokio::spawn(
            crate::test_support::MEDIA_VALIDATION.scope(control, async move {
                operations::execute_locked(&state, operation, None, false, Default::default()).await
            }),
        )
    };
    ready.await.unwrap();
    let playback=tokio::time::timeout(std::time::Duration::from_secs(2),crate::test_support::call(&state,"/api/v1/playback","POST",json!({"media_id":"other","options":{"quality":"auto","capabilities":{"containers":["mp4"],"video":["h264"],"audio":[],"hls":false}}}),&cookie)).await.expect("unrelated playback must not wait for hashing");
    assert_eq!(playback.0, axum::http::StatusCode::OK, "{}", playback.2);
    let progress = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        crate::test_support::call(
            &state,
            &format!(
                "/api/v1/playback/{}/progress",
                playback.2["id"].as_str().unwrap()
            ),
            "POST",
            json!({"sequence":1,"position":1,"state":"playing"}),
            &cookie,
        ),
    )
    .await
    .unwrap();
    assert_eq!(progress.0, axum::http::StatusCode::OK);
    release.send(()).unwrap();
    let _ = work.await.unwrap().unwrap();
    assert!(!path.exists());
    assert_eq!(control.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mutation_rechecks_protection_generation_and_open_file_identity_after_validation() {
    for change in ["keep", "generation", "replacement", "content"] {
        let (_temp, state, path) = movie().await;
        let operation =
            operations::prepare_operation(&state, None, "ret-movie".into(), "delete".into())
                .await
                .unwrap()
                .0["id"]
                .as_str()
                .unwrap()
                .to_owned();
        let control = Arc::new(crate::test_support::FaultControl::default());
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        *control.pause.lock().await = Some((entered, resume));
        let work = {
            let state = state.clone();
            tokio::spawn(
                crate::test_support::MEDIA_VALIDATION.scope(control, async move {
                    operations::execute_locked(&state, operation, None, false, Default::default())
                        .await
                }),
            )
        };
        ready.await.unwrap();
        match change {
            "keep" => {
                state
                    .db
                    .write("test.keep", |db| {
                        db.execute(
                            "INSERT INTO media_protection(media_id,keep) VALUES ('ret-movie',1)",
                            [],
                        )?;
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            "generation" => {
                state
                    .db
                    .write("test.generation", |db| {
                        db.execute("UPDATE media_files SET generation='new'", [])?;
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            _ => {
                let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
                let content = std::fs::read(&path).unwrap();
                if change == "replacement" {
                    std::fs::rename(&path, path.with_extension("old")).unwrap();
                }
                std::fs::write(&path, vec![b'x'; content.len()]).unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
        }
        release.send(()).unwrap();
        assert!(work.await.unwrap().is_err(), "{change}");
        assert!(path.exists(), "{change}");
    }
}

#[tokio::test]
async fn reacquisition_restores_exact_episode_ids_and_preserves_foreign_exclusions() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for owned in [false, true] {
        let (_temp, state, _) = movie().await;
        let monitors = Arc::new(AtomicUsize::new(0));
        let deletes = Arc::new(AtomicUsize::new(0));
        let recorded_monitors = monitors.clone();
        let recorded_deletes = deletes.clone();
        let seasons = Arc::new(std::sync::Mutex::new(Value::Null));
        let recorded_seasons = seasons.clone();
        let stub=Router::new()
            .route("/api/v3/series/9",get(||async {Json(json!({"id":9,"tvdbId":42,"seasons":[{"seasonNumber":1,"monitored":false},{"seasonNumber":2,"monitored":false}]}))}).put(move|Json(body):Json<Value>|{let seasons=recorded_seasons.clone();async move{*seasons.lock().unwrap()=body["seasons"].clone();Json(json!(9))}}))
            .route("/api/v3/episode",get(||async {Json(json!([{"id":37,"seriesId":9,"seasonNumber":1,"episodeNumber":2},{"id":80,"seriesId":9,"seasonNumber":2,"episodeNumber":1}]))}))
            .route("/api/v3/episode/monitor",axum::routing::put(move|Json(body):Json<Value>|{let count=recorded_monitors.clone();async move{assert_eq!(body,json!({"episodeIds":[37],"monitored":true}));count.fetch_add(1,Ordering::SeqCst);Json(json!({}))}}))
            .route("/api/v3/importlistexclusion",get(||async{Json(json!([{"id":5,"tvdbId":42}]))}))
            .route("/api/v3/importlistexclusion/5",axum::routing::delete(move||{let count=recorded_deletes.clone();async move{count.fetch_add(1,Ordering::SeqCst);Json(json!({}))}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
        state.db.write("test.fixture", move|db|{
            db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('sonarr','Fixture','sonarr','container',8989,'g',X'00','[]','{}','1',1,1,NULL)",[])?;
            crate::test_support::own(db)?;
            let targets=json!([{"id":"ret-file","generation":"first","path":"fixture","root":"fixture","size":0,"modified":"1","fingerprint":"1","ownership":"managed","claims":[{"service_id":"sonarr","service_generation":"g","manager_file_id":17,"entity_id":9,"manager_path":"fixture","external_id":"42","members":[37],"server_path":"fixture"}]}]);
            db.execute("INSERT INTO media_operations VALUES ('operation',NULL,'ret-movie','delete','complete',?1,1,1,NULL)",[targets.to_string()])?;
            db.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,trigger_user,state,eligible_at,due_at,error) VALUES ('candidate','ret-movie','operation','stamp','alice','complete',1,1,NULL)",[])?;
            db.execute("INSERT INTO retention_exclusions VALUES ('sonarr','42',5,?1)",[owned])?;
            Ok(())
        }).await.unwrap();
        let s = service(&state, "sonarr").await.unwrap();
        let connection = Connection {
            grant: ownership::Grant {
                scope: ownership::Scope::Operational,
                service: s.id.clone(),
                generation: s.generation.clone(),
                container: s.container.clone(),
            },
            ..Connection::validation(
                &state,
                format!("http://{address}"),
                String::new(),
                "fixture-key".into(),
                "sonarr".into(),
            )
        };
        reacquire(&state, &s, &connection, "42", 9).await.unwrap();
        assert_eq!(monitors.load(Ordering::SeqCst), 1);
        assert_eq!(deletes.load(Ordering::SeqCst), usize::from(owned));
        assert_eq!(
            *seasons.lock().unwrap(),
            json!([{"seasonNumber":1,"monitored":true},{"seasonNumber":2,"monitored":false}]),
            "A series search skips a season that isn't monitored"
        );
        state
            .db
            .write("test.fixture", |db| {
                assert_eq!(
                    db.query_row("SELECT state FROM retention_candidates", [], |r| r
                        .get::<_, String>(0))?,
                    "cancelled"
                );
                Ok(())
            })
            .await
            .unwrap();
        server.abort();
    }
}

// After auto-delete, the movie may be removed from Radarr; asked for again, it comes back
// under a new id.
#[tokio::test]
async fn a_movie_added_back_to_radarr_is_reacquired_under_its_new_id() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (_temp, state, _) = movie().await;
    let deletes = Arc::new(AtomicUsize::new(0));
    let recorded = deletes.clone();
    let stub = Router::new()
        .route(
            "/api/v3/exclusions",
            get(|| async { Json(json!([{"id":7,"tmdbId":603}])) }),
        )
        .route(
            "/api/v3/exclusions/7",
            axum::routing::delete(move || {
                let count = recorded.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(json!({}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('radarr','Fixture','radarr','container',7878,'g',X'00','[]','{}','1',1,1,NULL)",[])?;
        crate::test_support::own(db)?;
        let targets=json!([{"id":"ret-file","generation":"first","path":"fixture","root":"fixture","size":0,"modified":"1","fingerprint":"1","ownership":"managed","claims":[{"service_id":"radarr","service_generation":"g","manager_file_id":11,"entity_id":1,"manager_path":"fixture","external_id":"603","members":[],"server_path":"fixture"}]}]);
        db.execute("INSERT INTO media_operations VALUES ('operation',NULL,'ret-movie','delete','complete',?1,1,1,NULL)",[targets.to_string()])?;
        db.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,trigger_user,state,eligible_at,due_at,error) VALUES ('candidate','ret-movie','operation','stamp','alice','complete',1,1,NULL)",[])?;
        db.execute("INSERT INTO retention_exclusions VALUES ('radarr','603',7,1)",[])?;
        Ok(())
    }).await.unwrap();
    let s = service(&state, "radarr").await.unwrap();
    let connection = Connection {
        grant: ownership::Grant {
            scope: ownership::Scope::Operational,
            service: s.id.clone(),
            generation: s.generation.clone(),
            container: s.container.clone(),
        },
        ..Connection::validation(
            &state,
            format!("http://{address}"),
            String::new(),
            "fixture-key".into(),
            "radarr".into(),
        )
    };
    reacquire(&state, &s, &connection, "603", 2).await.unwrap();
    assert_eq!(deletes.load(Ordering::SeqCst), 1);
    let remaining = state
        .db
        .read("test.exclusions", |db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM retention_exclusions", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    server.abort();
}

async fn movie() -> (tempfile::TempDir, AppState, std::path::PathBuf) {
    let (temp, state, _) = fixture().await;
    let directory = temp.path().join("retention");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("movie.mp4");
    std::fs::write(&path, b"generated retention fixture").unwrap();
    let path = path.canonicalize().unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    let modified = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let hash = thelxinoe_catalog::fingerprint(&mut std::fs::File::open(&path).unwrap()).unwrap();
    let stored = path.to_string_lossy().to_string();
    let root = directory
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    state.db.write("test.fixture", move|db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('ret-root','Retention fixture','movies',?1)",[root])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('ret-movie','ret-root','movie','fixture','Retention fixture',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES ('ret-file','ret-root',?1,'first',?2,?3,?4,'{}','unmanaged',1)",params![stored,meta.len() as i64,modified,hash])?;
        db.execute("INSERT INTO media_sources VALUES ('ret-movie','ret-file')",[])?;
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','ret-movie',1,1)",[])?;
        db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\",\"bob\"]',grace_seconds=3600 WHERE domain='movies'",[])?;
        Ok(())
    }).await.unwrap();
    (temp, state, path)
}

#[tokio::test]
async fn watched_revision_protection_and_generation_are_revalidated() {
    let (_temp, state, _) = movie().await;
    state
        .db
        .write("test.fixture", |db| {
            assert!(eligibility(db, "ret-movie")?.is_none(), "Unmanaged files never qualify");
            db.execute("UPDATE media_files SET ownership='managed'", [])?;
            assert!(eligibility(db, "ret-movie")?.is_none(), "Bob has not watched");
            db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('bob','ret-movie',1,1)", [])?;
            let original = eligibility(db, "ret-movie")?.unwrap();
            db.execute(
                "UPDATE media_state SET updated_at=999,favorite=1 WHERE media_id='ret-movie'",
                [],
            )?;
            assert_eq!(
                original.stamp,
                eligibility(db, "ret-movie")?.unwrap().stamp,
                "Progress and favorites must not restart grace"
            );
            db.execute(
                "UPDATE media_state SET watched=0 WHERE media_id='ret-movie'",
                [],
            )?;
            assert!(eligibility(db, "ret-movie")?.is_none());
            db.execute(
                "UPDATE media_state SET watched=1 WHERE media_id='ret-movie'",
                [],
            )?;
            assert_ne!(original.stamp, eligibility(db, "ret-movie")?.unwrap().stamp);
            db.execute("INSERT INTO media_protection VALUES ('ret-movie',1)", [])?;
            assert!(eligibility(db, "ret-movie")?.is_none());
            db.execute("DELETE FROM media_protection", [])?;
            db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) SELECT 'active','alice',id,'ret-movie','ret-file','first','','paused','direct','{}',10,?1,?1 FROM sessions WHERE user_id='alice' LIMIT 1",[now()])?;
            assert!(eligibility(db,"ret-movie")?.is_none(),"Paused playback protects the file");
            assert!(matches!(storage::assess(db,"ret-movie")?,storage::Assessment::Waiting(_)),"Playback only postpones it");
            db.execute("UPDATE playback_sessions SET state='stopped'",[])?;
            let before = eligibility(db, "ret-movie")?.unwrap().stamp;
            db.execute(
                "UPDATE media_files SET generation='replacement' WHERE id='ret-file'",
                [],
            )?;
            assert_ne!(before, eligibility(db, "ret-movie")?.unwrap().stamp);
            db.execute(
                "UPDATE media_files SET ownership='unresolved' WHERE id='ret-file'",
                [],
            )?;
            assert!(eligibility(db, "ret-movie")?.is_none());
            assert!(
                matches!(storage::assess(db, "ret-movie")?, storage::Assessment::Waiting(_)),
                "A manager that can't be read says nothing about its files"
            );
            db.execute(
                "UPDATE media_files SET ownership='unmanaged' WHERE id='ret-file'",
                [],
            )?;
            assert!(matches!(storage::assess(db, "ret-movie")?, storage::Assessment::Ineligible));
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn unmanaged_media_is_excluded_from_the_auto_delete_queue() {
    let (_temp, state, path) = movie().await;
    state.db.write("test.watched", |db| {
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('bob','ret-movie',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 0);
    assert!(path.exists());
}

#[tokio::test]
async fn season_requires_complete_confirmed_aired_metadata_and_all_selected_users() {
    let (_temp, state, _) = movie().await;
    let refreshed = now();
    state.db.write("test.fixture", move |db| {
        db.execute("UPDATE media_files SET ownership='managed'",[])?;
        db.execute("UPDATE library_roots SET kind='shows'",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at,metadata) VALUES ('show','ret-root','show','show','Show',1,?1)",[json!({"refreshed_at":refreshed,"status":"Ended","seasons":[{"season_number":1,"episode_count":2}]}).to_string()])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('season','ret-root','season','show','season','Season',1,1)",[])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('sonarr','Fixture Sonarr','sonarr','container-sonarr',8989,'g',X'00','','{}','1',1,1,NULL)",[])?;
        crate::test_support::own(db)?;
        db.execute("INSERT INTO metadata_bindings VALUES ('show','sonarr','g','42',9,?1)",[refreshed])?;
        for (episode,user,manager_episode) in [("e1","alice",101),("e2","bob",102)] {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'ret-root','episode','season',?1,?1,1)",[episode])?;
            db.execute("INSERT INTO media_sources VALUES (?1,'ret-file')",[episode])?;
            db.execute("INSERT INTO manager_episodes VALUES ('sonarr','g','42',?1,1,?2,?3,?4)",params![manager_episode,manager_episode-100,refreshed,json!({"air_date":"2000-01-01"}).to_string()])?;
            db.execute("INSERT INTO manager_episode_mappings VALUES (?1,'sonarr','g',?2,'confirmed')",params![episode,manager_episode])?;
            db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES (?1,?2,1,1)",params![user,episode])?;
        }
        db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\",\"bob\"]' WHERE domain='shows'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Users' partial watches cannot be combined");
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','e2',1,1)",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Every selected user must finish the season");
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('bob','e1',1,1)",[])?;
        assert!(eligibility(db,"season")?.is_some());
        db.execute("UPDATE media SET sort_number=0 WHERE id='season'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Specials are excluded by default");
        db.execute("UPDATE media SET sort_number=1 WHERE id='season'",[])?;
        db.execute("UPDATE media SET kind='album' WHERE id='season'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Music never participates");
        db.execute("UPDATE media SET kind='season' WHERE id='season'",[])?;
        db.execute("UPDATE manager_episode_mappings SET state='complex' WHERE media_id='e2'",[])?;
        assert!(eligibility(db,"season")?.is_none());
        db.execute("UPDATE manager_episode_mappings SET state='confirmed'",[])?;
        db.execute("UPDATE manager_episodes SET metadata='{}' WHERE manager_episode_id=102",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Missing air dates are uncertain");
        db.execute("UPDATE manager_episodes SET metadata='{\"air_date\":\"2000-01-01\"}'",[])?;
        db.execute("UPDATE media SET metadata=json_set(metadata,'$.status','Returning Series') WHERE id='show'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"An airing season needs evidence of completion");
        db.execute("UPDATE media SET metadata=json_set(metadata,'$.status','Ended','$.seasons[0].episode_count',3) WHERE id='show'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Missing episodes block retention");
        assert!(matches!(storage::assess(db,"season")?,storage::Assessment::Ineligible));
        db.execute("UPDATE media SET metadata=json_set(metadata,'$.seasons[0].episode_count',2,'$.refreshed_at',0) WHERE id='show'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Stale manager metadata blocks retention");
        assert!(matches!(storage::assess(db,"season")?,storage::Assessment::Waiting(_)),"Stale metadata only postpones it");
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES ('second','ret-root','second','first',1,'1','1','{}','managed',1)",[])?;
        db.execute("UPDATE media_sources SET file_id='second' WHERE media_id='e2'",[])?;
        assert!(matches!(storage::assess(db,"season")?,storage::Assessment::Waiting(_)));
        db.execute("UPDATE media_files SET present=0 WHERE id='second'",[])?;
        assert!(matches!(storage::assess(db,"season")?,storage::Assessment::Ineligible),"Stale metadata doesn't hide a missing episode");
        Ok(())
    }).await.unwrap();
}

// Hundreds of movies, seasons and downloads can wait at once. The admin sees every one, the
// next to go first, while the outcomes that only accumulate stay the latest ones.
#[tokio::test]
async fn every_scheduled_deletion_is_listed_soonest_first() {
    let (_temp, state, _) = movie().await;
    state.db.write("test.fixture", |db| {
        let tx = db.transaction()?;
        tx.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        tx.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('show','ret-root','show','show','Silo (2023)',1)",[])?;
        tx.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('season','ret-root','season','show','season','Season 2',2,1)",[])?;
        // Seasons wait less than movies: the latest to qualify is the next to go.
        tx.execute("INSERT INTO retention_candidates(id,media_id,stamp,state,eligible_at,due_at) VALUES ('season','season','stamp','pending',5000,6000)",[])?;
        let outcomes = ["complete", "kept", "cancelled", "blocked"].into_iter().cycle();
        for (i, outcome) in (0..250_i64).zip(outcomes) {
            let (movie, video) = (format!("movie-{i}"), format!("video-{i}"));
            tx.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES (?1,'ret-root','movie',?1,?1,1)",[&movie])?;
            tx.execute("INSERT INTO retention_candidates(id,media_id,stamp,state,eligible_at,due_at) VALUES (?1,?1,'stamp','pending',?2,?3)",params![movie,1000+i,100_000+2*i])?;
            tx.execute("INSERT INTO video_retention_candidates(id,video_id,generation,stamp,title,reason,state,eligible_at,due_at) VALUES (?1,?1,'g','stamp',?1,'watched','pending',?2,?3)",params![video,1000+i,100_001+2*i])?;
            tx.execute("INSERT INTO retention_candidates(id,media_id,stamp,state,eligible_at,due_at) VALUES (?1,'ret-movie','stamp',?2,?3,?3)",params![format!("{movie}-outcome"),outcome,2*i])?;
            tx.execute("INSERT INTO video_retention_candidates(id,video_id,generation,stamp,title,reason,state,eligible_at,due_at) VALUES (?1,?1,'g','stamp',?1,'storage_limit',?2,?3,?3)",params![format!("{video}-outcome"),outcome,2*i+1])?;
        }
        tx.commit()?;
        Ok(())
    }).await.unwrap();
    let cookie = format!(
        "thelxinoe_session={}",
        crate::test_support::issue_session(&state.db, "alice".into(), "web".into(), "Alice".into())
            .await
            .unwrap()
    );
    let (status, _, listed) = crate::test_support::call(
        &state,
        "/api/v1/admin/retention/deletions",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{listed}");
    assert_eq!(listed["totals"], json!({"pending":501,"history":500}));
    let items = listed["items"].as_array().unwrap();
    let ids = items
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let scheduled = std::iter::once("season".to_owned())
        .chain((0..250).flat_map(|i| [format!("movie-{i}"), format!("video-{i}")]))
        .collect::<Vec<_>>();
    assert_eq!(ids[..501], scheduled[..]);
    let latest = (300..500)
        .rev()
        .map(|at| format!("{}-{}-outcome", ["movie", "video"][at % 2], at / 2))
        .collect::<Vec<_>>();
    assert_eq!(ids[501..], latest[..]);
    let title = |id: &str| items.iter().find(|item| item["id"] == id).unwrap()["title"].clone();
    assert_eq!(title("season"), "Silo (2023) · Season 2");
    assert_eq!(title("movie-249-outcome"), "Retention fixture");
}

#[tokio::test]
async fn auto_delete_waits_at_least_a_day() {
    let (_temp, state, cookie) = fixture().await;
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    for (grace, expected) in [
        (0, axum::http::StatusCode::BAD_REQUEST),
        (86399, axum::http::StatusCode::BAD_REQUEST),
        (86400, axum::http::StatusCode::OK),
    ] {
        let policy = json!({"enabled":true,"grace_seconds":grace,"exclude_specials":true,"trigger_users":["alice"]});
        let (status, _, body) = crate::test_support::call(
            &state,
            "/api/v1/admin/retention/policy/movies",
            "POST",
            policy,
            &cookie,
        )
        .await;
        assert_eq!(status, expected, "{grace}: {body}");
    }
}

/// What a fake manager reports, changed as a test goes on.
#[derive(Default)]
struct Upstream {
    /// The movies or series with a download in the queue.
    queued: std::sync::Mutex<Vec<i64>>,
    /// While set, the manager's library can't be read.
    down: std::sync::atomic::AtomicBool,
    /// Whether each coming read of the commands finds one running, and `busy` after those.
    commands: std::sync::Mutex<std::collections::VecDeque<bool>>,
    busy: std::sync::atomic::AtomicBool,
    /// The seasons of each series update Sonarr accepted, and whether it refuses them.
    series_updates: std::sync::Mutex<Vec<Value>>,
    refuses_series_updates: std::sync::atomic::AtomicBool,
    /// While set, reads of the commands after the first two never get an answer in time.
    hung: std::sync::atomic::AtomicBool,
    command_reads: std::sync::atomic::AtomicUsize,
    library_reads: std::sync::atomic::AtomicUsize,
    exclusions: std::sync::atomic::AtomicUsize,
    /// Monitoring changes and file deletions.
    changes: std::sync::atomic::AtomicUsize,
}
impl Upstream {
    async fn commands(&self) -> Json<Value> {
        use std::sync::atomic::Ordering;
        if self.hung.load(Ordering::SeqCst) && self.command_reads.load(Ordering::SeqCst) >= 2 {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
        self.count(&self.command_reads);
        let next = self.commands.lock().unwrap().pop_front();
        let busy = next.unwrap_or_else(|| self.busy.load(std::sync::atomic::Ordering::SeqCst));
        Json(if busy {
            json!([{"name":"ProcessMonitoredDownloads","status":"started"}])
        } else {
            json!([])
        })
    }
    /// A filtered queue counts only what it names, as Radarr and Sonarr do.
    fn queue(&self, filter: &str, query: &[(String, String)]) -> Json<Value> {
        let ids = query
            .iter()
            .filter(|(name, _)| name == filter)
            .map(|(_, id)| id.parse::<i64>().unwrap())
            .collect::<Vec<_>>();
        let queued = self.queued.lock().unwrap();
        let total = queued
            .iter()
            .filter(|id| ids.is_empty() || ids.contains(id))
            .count();
        Json(json!({"totalRecords":total}))
    }
    fn count(&self, counter: &std::sync::atomic::AtomicUsize) {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn modified(file: &std::path::Path) -> String {
    std::fs::metadata(file)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string()
}

async fn media_library(
    file: &str,
) -> (
    tempfile::TempDir,
    AppState,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let (temp, mut state, _) = fixture().await;
    let media = temp.path().join("media");
    let path = media.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("fixture {file}")).unwrap();
    let media = media.canonicalize().unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    (temp, state, media, path.canonicalize().unwrap())
}

/// A watched movie on a fake Radarr, with its deletion scheduled.
async fn scheduled_movie() -> (
    tempfile::TempDir,
    AppState,
    Arc<Upstream>,
    std::path::PathBuf,
    String,
    tokio::task::JoinHandle<()>,
) {
    let (temp, state, media, file) = media_library("movies/one.mp4").await;
    let upstream = Arc::new(Upstream::default());
    let movie = json!({"id":1,"tmdbId":603,"hasFile":true,"title":"One","year":1999});
    let (library, entity, removed) = (movie.clone(), movie, file.clone());
    let (u1, u2, u3, u4, u5, u6) = (
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
    );
    let stub = Router::new()
        .route(
            "/api/v3/movie",
            get(move || {
                let (u, library) = (u1.clone(), library.clone());
                async move {
                    u.count(&u.library_reads);
                    if u.down.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
                    }
                    Ok(Json(json!([library])))
                }
            }),
        )
        .route(
            "/api/v3/movie/1",
            get(move || {
                let entity = entity.clone();
                async move { Json(entity) }
            })
            .put(move |Json(body): Json<Value>| {
                let u = u2.clone();
                async move {
                    u.count(&u.changes);
                    Json(body)
                }
            }),
        )
        .route(
            "/api/v3/moviefile",
            get(|| async { Json(json!([{"id":11,"movieId":1,"path":"/media/movies/one.mp4"}])) }),
        )
        .route(
            "/api/v3/moviefile/11",
            get(|| async { Json(json!({"id":11,"movieId":1,"path":"/media/movies/one.mp4"})) })
                .delete(move || {
                    let (u, removed) = (u3.clone(), removed.clone());
                    async move {
                        u.count(&u.changes);
                        std::fs::remove_file(&removed).unwrap();
                        Json(json!({}))
                    }
                }),
        )
        .route(
            "/api/v3/command",
            get(move || {
                let u = u4.clone();
                async move { u.commands().await }
            }),
        )
        .route(
            "/api/v3/queue",
            get(
                move |axum::extract::Query(query): axum::extract::Query<Vec<(String, String)>>| {
                    let u = u5.clone();
                    async move { u.queue("movieIds", &query) }
                },
            ),
        )
        .route(
            "/api/v3/exclusions",
            get(|| async { Json(json!([])) }).post(move || {
                let u = u6.clone();
                async move {
                    u.count(&u.exclusions);
                    Json(json!({"id":7}))
                }
            }),
        );
    let server = bindings::tests::manager(&state, &media, "radarr", stub).await;
    let meta = std::fs::metadata(&file).unwrap();
    let size = meta.len() as i64;
    let stamp = modified(&file);
    let hash = thelxinoe_catalog::fingerprint(&mut std::fs::File::open(&file).unwrap()).unwrap();
    let stored = file.to_string_lossy().to_string();
    let root = media.join("movies").to_string_lossy().to_string();
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies',?1)",[root])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('one','movies','movie','one','One',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES ('one','movies',?1,'first',?2,?3,?4,'{}','unmanaged',1)",params![stored,size,stamp,hash])?;
        db.execute("INSERT INTO media_sources VALUES ('one','one')",[])?;
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','one',1,1)",[])?;
        db.execute("INSERT INTO settings(key,value) VALUES ('manager.retention.radarr','true')",[])?;
        db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\"]',grace_seconds=3600 WHERE domain='movies'",[])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 1);
    let candidate = state
        .db
        .read("test.candidate", |db| {
            Ok(
                db.query_row("SELECT id FROM retention_candidates", [], |r| {
                    r.get::<_, String>(0)
                })?,
            )
        })
        .await
        .unwrap();
    (temp, state, upstream, file, candidate, server)
}

async fn make_due(state: &AppState) {
    state
        .db
        .write("test.due", |db| {
            db.execute("UPDATE retention_candidates SET due_at=1", [])?;
            Ok(())
        })
        .await
        .unwrap();
}

/// The only scheduled deletion's state, its operation's state and its recorded error.
async fn deletion_states(state: &AppState) -> (String, String, Option<String>) {
    state.db.read("test.states", |db| {
        Ok(db.query_row("SELECT c.state,o.state,c.error FROM retention_candidates c JOIN media_operations o ON o.id=c.operation_id", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?)
    }).await.unwrap()
}

// A due deletion that meets a busy manager stays scheduled, changes nothing upstream and
// runs on a later pass once the manager is idle.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_busy_manager_postpones_a_due_deletion_instead_of_blocking_it() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    make_due(&state).await;
    upstream.queued.lock().unwrap().push(1);
    let before = upstream.library_reads.load(Ordering::SeqCst);
    assert!(delete_locked(&state, &candidate, None, true).await.is_err());
    assert_eq!(
        upstream.library_reads.load(Ordering::SeqCst),
        before,
        "A retry that has to wait must not read the whole library"
    );
    let (waiting, operation, error) = deletion_states(&state).await;
    assert_eq!(
        (waiting.as_str(), operation.as_str()),
        ("pending", "pending")
    );
    assert!(error.unwrap().contains("queued"));
    assert_eq!(upstream.exclusions.load(Ordering::SeqCst), 0);
    assert!(file.exists());
    upstream.queued.lock().unwrap().clear();
    // A deletion relies on a recent read of the library, and only on a recent one.
    state
        .db
        .write("test.stale", |db| {
            db.execute("UPDATE manager_reconciliations SET checked_at=1", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let before = upstream.library_reads.load(Ordering::SeqCst);
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert_eq!(
        upstream.library_reads.load(Ordering::SeqCst) - before,
        1,
        "Only reconciliation reads the whole library; rechecks read the selected movie"
    );
    let (done, operation, _) = deletion_states(&state).await;
    assert_eq!(
        (done.as_str(), operation.as_str()),
        ("complete", "complete")
    );
    assert_eq!(upstream.exclusions.load(Ordering::SeqCst), 1);
    assert!(!file.exists());
    server.abort();
}

// A manager that can't be read, as during every server update, says nothing about the movie.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_manager_outage_keeps_a_scheduled_deletions_deadline() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    let schedule = || {
        state.db.read("test.schedule", |db| {
            Ok(db.query_row(
                "SELECT COUNT(*),MIN(id),MIN(due_at),MIN(state) FROM retention_candidates",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )?)
        })
    };
    let scheduled = schedule().await.unwrap();
    upstream.down.store(true, Ordering::SeqCst);
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 0);
    assert_eq!(
        bindings::tests::ownership(&state, "one").await,
        "unresolved"
    );
    assert_eq!(schedule().await.unwrap(), scheduled);
    make_due(&state).await;
    let (ready, waiting) = storage::run(&state.db).await.unwrap();
    assert!(ready.is_empty(), "Nothing runs on what can't be confirmed");
    assert_eq!(waiting.len(), 1);
    let (_, _, reason) = deletion_states(&state).await;
    assert_eq!(reason.as_deref(), Some(waiting[0].1));
    let (_, again) = storage::run(&state.db).await.unwrap();
    assert!(again.is_empty(), "A reason is only news once");
    assert!(file.exists());
    upstream.down.store(false, Ordering::SeqCst);
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 0);
    let (ready, _) = storage::run(&state.db).await.unwrap();
    assert_eq!(
        ready,
        std::slice::from_ref(&candidate),
        "It is already overdue"
    );
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(!file.exists());
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn only_a_download_of_the_same_movie_holds_its_deletion() {
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    make_due(&state).await;
    upstream.queued.lock().unwrap().push(1);
    assert!(delete_locked(&state, &candidate, None, true).await.is_err());
    assert!(file.exists());
    *upstream.queued.lock().unwrap() = vec![2];
    assert!(
        operations::ensure_idle(&state, "radarr").await.is_err(),
        "Stopping a manager still waits for its whole queue"
    );
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(!file.exists());
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn an_attempt_that_changed_nothing_runs_again() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    make_due(&state).await;
    // Idle until it has been marked as started, then too busy for as long as it waits.
    upstream.commands.lock().unwrap().extend([false, false]);
    upstream.busy.store(true, Ordering::SeqCst);
    assert!(delete_locked(&state, &candidate, None, true).await.is_err());
    let (waiting, operation, error) = deletion_states(&state).await;
    assert_eq!(
        (waiting.as_str(), operation.as_str()),
        ("pending", "pending")
    );
    assert!(error.unwrap().contains("busy"));
    assert_eq!(upstream.changes.load(Ordering::SeqCst), 0);
    assert!(file.exists());
    upstream.busy.store(false, Ordering::SeqCst);
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(!file.exists());
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_change_during_the_wait_for_a_manager_stops_the_deletion() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    make_due(&state).await;
    upstream.commands.lock().unwrap().extend([false, false]);
    upstream.busy.store(true, Ordering::SeqCst);
    let deletion = {
        let (state, candidate) = (state.clone(), candidate.clone());
        tokio::spawn(async move { delete_locked(&state, &candidate, None, true).await })
    };
    while upstream.command_reads.load(Ordering::SeqCst) < 3 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    change_fixture(&state, "INSERT INTO media_protection VALUES ('one',1)").await;
    upstream.busy.store(false, Ordering::SeqCst);
    assert!(deletion.await.unwrap().is_err());
    assert_eq!(upstream.changes.load(Ordering::SeqCst), 0);
    assert!(file.exists());
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_manager_that_stops_answering_cant_hold_a_deletion_past_the_wait() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, file, candidate, server) = scheduled_movie().await;
    make_due(&state).await;
    upstream.hung.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    assert!(delete_locked(&state, &candidate, None, true).await.is_err());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "A request alone may take 20 s"
    );
    let (waiting, operation, _) = deletion_states(&state).await;
    assert_eq!(
        (waiting.as_str(), operation.as_str()),
        ("pending", "pending")
    );
    assert!(file.exists());
    server.abort();
}

/// Two episodes of a season on a fake Sonarr.
async fn season_on_sonarr() -> (
    tempfile::TempDir,
    AppState,
    Arc<Upstream>,
    [std::path::PathBuf; 2],
    tokio::task::JoinHandle<()>,
) {
    let (temp, state, media, first) = media_library("shows/11.mkv").await;
    let second = first.with_file_name("12.mkv");
    std::fs::write(&second, "fixture shows/12.mkv").unwrap();
    let upstream = Arc::new(Upstream::default());
    let series = json!({"id":9,"tvdbId":42,"statistics":{"episodeFileCount":2},"seasons":[{"seasonNumber":1,"monitored":true},{"seasonNumber":2,"monitored":true}]});
    let episode = |id: i64, file: i64| json!({"id":id,"seriesId":9,"seasonNumber":1,"episodeFileId":file,"episodeFile":{"id":file,"path":format!("/media/shows/{file}.mkv")}});
    let episodes = json!([episode(101, 11), episode(102, 12)]);
    let (library, entity) = (series.clone(), series);
    let shows = first.parent().unwrap().to_owned();
    let (u1, u2, u3, u4, u5) = (
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
        upstream.clone(),
    );
    let stub = Router::new()
        .route(
            "/api/v3/series",
            get(move || {
                let library = library.clone();
                async move { Json(json!([library])) }
            }),
        )
        .route(
            "/api/v3/series/9",
            get(move || {
                let entity = entity.clone();
                async move { Json(entity) }
            })
            .put(move |Json(body): Json<Value>| {
                let u = u5.clone();
                async move {
                    if u.refuses_series_updates
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
                    }
                    u.series_updates
                        .lock()
                        .unwrap()
                        .push(body["seasons"].clone());
                    Ok(Json(json!(9)))
                }
            }),
        )
        .route(
            "/api/v3/episode",
            get(move || {
                let episodes = episodes.clone();
                async move { Json(episodes) }
            }),
        )
        .route(
            "/api/v3/episode/monitor",
            axum::routing::put(move || {
                let u = u1.clone();
                async move {
                    u.count(&u.changes);
                    Json(json!({}))
                }
            }),
        )
        .route(
            "/api/v3/episodefile/{id}",
            get(|Path(id): Path<i64>| async move {
                Json(json!({"id":id,"path":format!("/media/shows/{id}.mkv")}))
            })
            .delete(move |Path(id): Path<i64>| {
                let (u, shows) = (u2.clone(), shows.clone());
                async move {
                    u.count(&u.changes);
                    std::fs::remove_file(shows.join(format!("{id}.mkv"))).unwrap();
                    Json(json!({}))
                }
            }),
        )
        .route(
            "/api/v3/command",
            get(move || {
                let u = u3.clone();
                async move { u.commands().await }
            }),
        )
        .route(
            "/api/v3/queue",
            get(
                move |axum::extract::Query(query): axum::extract::Query<Vec<(String, String)>>| {
                    let u = u4.clone();
                    async move { u.queue("seriesIds", &query) }
                },
            ),
        );
    let server = bindings::tests::manager(&state, &media, "sonarr", stub).await;
    let describe = |file: &std::path::Path| {
        (
            file.to_string_lossy().to_string(),
            std::fs::metadata(file).unwrap().len() as i64,
            modified(file),
            thelxinoe_catalog::fingerprint(&mut std::fs::File::open(file).unwrap()).unwrap(),
        )
    };
    let files = [describe(&first), describe(&second)];
    let root = media.join("shows").to_string_lossy().to_string();
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('shows','Shows','shows',?1)",[root])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('show','shows','show','show','Show',1)",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('season','shows','season','show','season','Season',1,1)",[])?;
        for (episode, (path, size, stamp, hash)) in ["e1", "e2"].into_iter().zip(files) {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'shows','episode','season',?1,?1,1)",[episode])?;
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES (?1,'shows',?2,'first',?3,?4,?5,'{}','unmanaged',1)",params![episode,path,size,stamp,hash])?;
            db.execute("INSERT INTO media_sources VALUES (?1,?1)",[episode])?;
        }
        Ok(())
    }).await.unwrap();
    (temp, state, upstream, [first, second], server)
}

/// A season prepared for deletion by hand.
async fn prepared_season() -> (
    tempfile::TempDir,
    AppState,
    Arc<Upstream>,
    [std::path::PathBuf; 2],
    String,
    tokio::task::JoinHandle<()>,
) {
    let (temp, state, upstream, files, server) = season_on_sonarr().await;
    let operation = operations::prepare_operation(&state, None, "season".into(), "delete".into())
        .await
        .unwrap()
        .0["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (temp, state, upstream, files, operation, server)
}

/// A watched, finished season with its auto-delete scheduled and due.
async fn scheduled_season() -> (
    tempfile::TempDir,
    AppState,
    Arc<Upstream>,
    [std::path::PathBuf; 2],
    String,
    tokio::task::JoinHandle<()>,
) {
    let (temp, state, upstream, files, server) = season_on_sonarr().await;
    let refreshed = now();
    state.db.write("test.fixture", move |db| {
        db.execute("UPDATE media SET metadata=?1 WHERE id='show'",[json!({"refreshed_at":refreshed,"status":"Ended","seasons":[{"season_number":1,"episode_count":2}]}).to_string()])?;
        db.execute("INSERT INTO metadata_bindings VALUES ('show','sonarr','g','42',9,?1)",[refreshed])?;
        for (episode, manager_episode) in [("e1", 101), ("e2", 102)] {
            db.execute("INSERT INTO manager_episodes VALUES ('sonarr','g','42',?1,1,?2,?3,?4)",params![manager_episode,manager_episode-100,refreshed,json!({"air_date":"2000-01-01"}).to_string()])?;
            db.execute("INSERT INTO manager_episode_mappings VALUES (?1,'sonarr','g',?2,'confirmed')",params![episode,manager_episode])?;
            db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice',?1,1,1)",[episode])?;
        }
        db.execute("INSERT INTO settings(key,value) VALUES ('manager.retention.sonarr','true')",[])?;
        db.execute("UPDATE retention_policies SET enabled=1,trigger_users='[\"alice\"]' WHERE domain='shows'",[])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 1);
    make_due(&state).await;
    let candidate = state
        .db
        .read("test.candidate", |db| {
            Ok(
                db.query_row("SELECT id FROM retention_candidates", [], |r| {
                    r.get::<_, String>(0)
                })?,
            )
        })
        .await
        .unwrap();
    (temp, state, upstream, files, candidate, server)
}

async fn operation_state(state: &AppState, operation: &str) -> String {
    let operation = operation.to_owned();
    state
        .db
        .read("test.operation", move |db| {
            Ok(db.query_row(
                "SELECT state FROM media_operations WHERE id=?1",
                [operation],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap()
}

// Managers run a short command every minute, so one lands among a season's files sooner or
// later. Stopping there would leave part of the season deleted.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_season_deletion_waits_out_a_command_between_its_files() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, files, operation, server) = prepared_season().await;
    // The two checks before it starts and the first file's find it idle, the second's doesn't.
    upstream
        .commands
        .lock()
        .unwrap()
        .extend([false, false, false, true, true]);
    *upstream.queued.lock().unwrap() = vec![7];
    let _ = operations::execute_locked(&state, operation.clone(), None, false, Default::default())
        .await
        .unwrap();
    assert_eq!(operation_state(&state, &operation).await, "complete");
    assert_eq!(upstream.changes.load(Ordering::SeqCst), 4);
    assert!(files.iter().all(|file| !file.exists()));
    assert!(
        upstream.series_updates.lock().unwrap().is_empty(),
        "Only auto-delete unmonitors the season itself"
    );
    server.abort();
}

// Only that season: the series and its other seasons stay monitored.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn auto_deleting_a_season_unmonitors_that_season() {
    let (_temp, state, upstream, files, candidate, server) = scheduled_season().await;
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(files.iter().all(|file| !file.exists()));
    assert_eq!(
        *upstream.series_updates.lock().unwrap(),
        [json!([{"seasonNumber":1,"monitored":false},{"seasonNumber":2,"monitored":true}])]
    );
    let (done, operation, _) = deletion_states(&state).await;
    assert_eq!(
        (done.as_str(), operation.as_str()),
        ("complete", "complete")
    );
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_season_that_cant_be_unmonitored_is_still_a_completed_deletion() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, files, candidate, server) = scheduled_season().await;
    upstream
        .refuses_series_updates
        .store(true, Ordering::SeqCst);
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(files.iter().all(|file| !file.exists()));
    let (done, operation, _) = deletion_states(&state).await;
    assert_eq!(
        (done.as_str(), operation.as_str()),
        ("complete", "complete")
    );
    server.abort();
}

// Once something is gone, another attempt can't tell what the first did.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_season_deletion_stopped_midway_is_not_repeated() {
    use std::sync::atomic::Ordering;
    let (_temp, state, upstream, files, operation, server) = prepared_season().await;
    upstream
        .commands
        .lock()
        .unwrap()
        .extend([false, false, false]);
    upstream.busy.store(true, Ordering::SeqCst);
    let stopped =
        operations::execute_locked(&state, operation.clone(), None, false, Default::default())
            .await;
    assert!(stopped.is_err());
    assert_eq!(operation_state(&state, &operation).await, "uncertain");
    assert!(!files[0].exists());
    assert!(files[1].exists());
    upstream.busy.store(false, Ordering::SeqCst);
    let repeated =
        operations::execute_locked(&state, operation.clone(), None, false, Default::default())
            .await;
    assert!(repeated.is_err());
    assert!(files[1].exists());
    server.abort();
}

/// A deletion of the fixture movie, due long ago and captured with `targets`.
async fn scheduled_fixture(state: &AppState, targets: &'static str) -> String {
    state.db.write("test.scheduled", move |db| {
        db.execute("UPDATE media_files SET ownership='managed'", [])?;
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('bob','ret-movie',1,1)", [])?;
        let stamp = eligibility(db, "ret-movie")?.unwrap().stamp;
        db.execute("INSERT INTO media_operations VALUES ('operation',NULL,'ret-movie','delete','pending',?1,1,1,NULL)", [targets])?;
        db.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,trigger_user,state,eligible_at,due_at) VALUES ('candidate','ret-movie','operation',?1,'alice','pending',1,1234)", [&stamp])?;
        Ok(stamp)
    }).await.unwrap()
}

async fn schedule_of_fixture(state: &AppState) -> (String, i64) {
    state
        .db
        .read("test.schedule", |db| {
            Ok(db.query_row(
                "SELECT state,due_at FROM retention_candidates WHERE id='candidate'",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn change_fixture(state: &AppState, sql: &'static str) {
    state
        .db
        .write("test.change", move |db| {
            db.execute_batch(sql)?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn only_a_real_change_cancels_a_scheduled_deletion() {
    let (_temp, state, _) = movie().await;
    scheduled_fixture(&state, "[]").await;
    let kept_deadline = ("pending".to_owned(), 1234);
    for (hold, release) in [
        (
            "UPDATE media_files SET ownership='unresolved'",
            "UPDATE media_files SET ownership='managed'",
        ),
        (
            "INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) SELECT 'active','alice',id,'ret-movie','ret-file','first','','paused','direct','{}',10,strftime('%s','now'),strftime('%s','now') FROM sessions WHERE user_id='alice' LIMIT 1",
            "DELETE FROM playback_sessions",
        ),
    ] {
        change_fixture(&state, hold).await;
        assert!(
            storage::evaluate_all_write_media(&state.db)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(schedule_of_fixture(&state).await, kept_deadline, "{hold}");
        change_fixture(&state, release).await;
        assert!(
            storage::evaluate_all_write_media(&state.db)
                .await
                .unwrap()
                .is_empty(),
            "The scheduled deletion still stands"
        );
        assert_eq!(
            schedule_of_fixture(&state).await,
            kept_deadline,
            "{release}"
        );
    }
    // An outage doesn't hide Keep, and taking Keep back starts a new countdown.
    change_fixture(&state, "UPDATE media_files SET ownership='unresolved'").await;
    change_fixture(
        &state,
        "INSERT INTO media_protection VALUES ('ret-movie',1)",
    )
    .await;
    assert!(
        storage::evaluate_all_write_media(&state.db)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(schedule_of_fixture(&state).await.0, "cancelled");
    change_fixture(
        &state,
        "DELETE FROM media_protection; UPDATE media_files SET ownership='managed'",
    )
    .await;
    assert_eq!(
        storage::evaluate_all_write_media(&state.db)
            .await
            .unwrap()
            .len(),
        1,
        "A deletion taken back is scheduled afresh"
    );
}

#[tokio::test]
async fn a_scheduled_deletion_follows_a_replaced_managers_ids() {
    let (_temp, state, _) = movie().await;
    scheduled_fixture(&state, "[]").await;
    state
        .db
        .write("test.stale_claims", |db| {
            let mut stale = serde_json::to_value(operations::storage::load_targets(db, "ret-movie")?)?;
            stale[0]["claims"] = json!([{"service_id":"radarr","service_generation":"replaced","manager_file_id":11,"entity_id":1,"manager_path":"/media/movies/one.mp4","external_id":"603","members":[],"server_path":"one.mp4"}]);
            db.execute(
                "UPDATE media_operations SET targets=?1 WHERE id='operation'",
                [stale.to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        storage::evaluate_all_write_media(&state.db)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        schedule_of_fixture(&state).await,
        ("pending".to_owned(), 1234)
    );
    let followed = state
        .db
        .read("test.targets", |db| {
            let saved = db.query_row(
                "SELECT targets FROM media_operations WHERE id='operation'",
                [],
                |r| r.get::<_, String>(0),
            )?;
            Ok(serde_json::from_str::<Vec<operations::Target>>(&saved)?
                == operations::storage::load_targets(db, "ret-movie")?)
        })
        .await
        .unwrap();
    assert!(followed);
}
