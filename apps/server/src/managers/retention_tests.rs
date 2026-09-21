use super::*;
use crate::online::oauth::tests::fixture;

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
        let stub=Router::new()
            .route("/api/v3/episode",get(||async {Json(json!([{"id":37,"seriesId":9,"seasonNumber":1,"episodeNumber":2},{"id":80,"seriesId":9,"seasonNumber":2,"episodeNumber":1}]))}))
            .route("/api/v3/episode/monitor",axum::routing::put(move|Json(body):Json<Value>|{let count=recorded_monitors.clone();async move{assert_eq!(body,json!({"episodeIds":[37],"monitored":true}));count.fetch_add(1,Ordering::SeqCst);Json(json!({}))}}))
            .route("/api/v3/importlistexclusion",get(||async{Json(json!([{"id":5,"tvdbId":42}]))}))
            .route("/api/v3/importlistexclusion/5",axum::routing::delete(move||{let count=recorded_deletes.clone();async move{count.fetch_add(1,Ordering::SeqCst);Json(json!({}))}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
        state.db.call(move|db|{
            db.execute("INSERT INTO manager_services VALUES ('sonarr','Fixture','sonarr','container',8989,'g',X'00','[]','{}','1',1,1,NULL)",[])?;
            let targets=json!([{"id":"ret-file","generation":"first","path":"fixture","root":"fixture","size":0,"modified":"1","fingerprint":"1","ownership":"managed","claims":[{"service_id":"sonarr","service_generation":"g","manager_file_id":17,"entity_id":9,"manager_path":"fixture","external_id":"42","members":[37],"server_path":"fixture"}]}]);
            db.execute("INSERT INTO media_operations VALUES ('operation',NULL,'ret-movie','delete','complete',?1,1,1,NULL)",[targets.to_string()])?;
            db.execute("INSERT INTO retention_candidates VALUES ('candidate','ret-movie','operation','stamp','alice','complete',1,1,NULL)",[])?;
            db.execute("INSERT INTO retention_exclusions VALUES ('sonarr','42',5,?1)",[owned])?;
            Ok(())
        }).await.unwrap();
        let s = service(&state, "sonarr").await.unwrap();
        let connection = Connection {
            state: &state,
            base: format!("http://{address}"),
            key: "fixture-key".into(),
            kind: "sonarr".into(),
        };
        reacquire(&state, &s, &connection, "42", 9).await.unwrap();
        assert_eq!(monitors.load(Ordering::SeqCst), 1);
        assert_eq!(deletes.load(Ordering::SeqCst), usize::from(owned));
        state
            .db
            .call(|db| {
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
    let hash = Sha256::digest(b"generated retention fixture")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let stored = path.to_string_lossy().to_string();
    let root = directory
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    state.db.call(move|db| {
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
        .call(|db| {
            let original = eligibility(db, "ret-movie")?.unwrap();
            assert!(!original.automatic);
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
            db.execute("UPDATE playback_sessions SET state='stopped'",[])?;
            let before = eligibility(db, "ret-movie")?.unwrap().stamp;
            db.execute(
                "UPDATE media_files SET generation='replacement' WHERE id='ret-file'",
                [],
            )?;
            assert_ne!(before, eligibility(db, "ret-movie")?.unwrap().stamp);
            db.execute(
                "UPDATE media_files SET ownership='ambiguous' WHERE id='ret-file'",
                [],
            )?;
            assert!(eligibility(db, "ret-movie")?.is_none());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn automatic_deletion_requires_grace_and_root_optin_and_cannot_replay() {
    let (_temp, state, path) = movie().await;
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 1);
    assert_eq!(evaluate_all(&state, None).await.unwrap(), 0);
    let (candidate, operation) = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT id,operation_id FROM retention_candidates",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert!(
        revalidate_operation(&state, &operation, true)
            .await
            .is_err()
    );
    assert!(
        revalidate_operation(&state, &operation, false)
            .await
            .is_ok()
    );
    state
        .db
        .call(|db| {
            db.execute(
                "UPDATE library_roots SET automatic_unmanaged_deletion=1",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        revalidate_operation(&state, &operation, true)
            .await
            .is_err(),
        "Grace has not elapsed"
    );
    state
        .db
        .call(|db| {
            db.execute("UPDATE retention_candidates SET due_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(revalidate_operation(&state, &operation, true).await.is_ok());
    let _lease = state.media_operations.write().await;
    let _guard = state.managers.guard.lock().await;
    delete_locked(&state, &candidate, None, true).await.unwrap();
    assert!(!path.exists());
    assert!(delete_locked(&state, &candidate, None, true).await.is_err());
    state
        .db
        .call(|db| {
            assert_eq!(
                db.query_row("SELECT state FROM retention_candidates", [], |r| r
                    .get::<_, String>(0))?,
                "complete"
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn season_requires_complete_confirmed_aired_metadata_and_one_users_watched_set() {
    let (_temp, state, _) = movie().await;
    let refreshed = now();
    state.db.call(move |db| {
        db.execute("UPDATE library_roots SET kind='shows'",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at,metadata) VALUES ('show','ret-root','show','show','Show',1,?1)",[json!({"refreshed_at":refreshed,"status":"Ended","seasons":[{"season_number":1,"episode_count":2}]}).to_string()])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('season','ret-root','season','show','season','Season',1,1)",[])?;
        db.execute("INSERT INTO manager_services VALUES ('sonarr','Fixture Sonarr','sonarr','container-sonarr',8989,'g',X'00','','{}','1',1,1,NULL)",[])?;
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
        db.execute("UPDATE media SET metadata=json_set(metadata,'$.seasons[0].episode_count',2,'$.refreshed_at',0) WHERE id='show'",[])?;
        assert!(eligibility(db,"season")?.is_none(),"Stale manager metadata blocks retention");
        Ok(())
    }).await.unwrap();
}
