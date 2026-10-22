use super::*;
use crate::test_support::fixture;
use axum::{Router, extract::Query, routing::get};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// A refresh asks Radarr for its one movie by TMDB ID instead of downloading the library.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_refresh_looks_up_its_title_by_external_id() {
    let (temp, mut state, _cookie) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    let queries = Arc::new(Mutex::new(Vec::<HashMap<String, String>>::new()));
    let seen = queries.clone();
    let stub =
        Router::new().route(
            "/api/v3/movie",
            get(move |Query(query): Query<HashMap<String, String>>| {
                seen.lock().unwrap().push(query);
                async move {
                    Json(json!([{"id":17,"tmdbId":603,"title":"Refreshed title","year":1999}]))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let container = "a".repeat(64);
    let evidence = json!({"id":container,"name":"localhost","running":true,
        "mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],
        "networks":[{"id":"network","address":"127.0.0.1"}]});
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
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES ('radarr','Fixture','radarr',?1,?2,'g',?3,?4,'{}','1',1,1)",params![container,port,credential,media_source])?;
        crate::test_support::own(db)?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Movies','movies','/movies')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('0a1b2c','root','movie','movie','Stale title',1)",[])?;
        Ok(())
    }).await.unwrap();
    run(
        &state,
        &json!({"media_id":"0a1b2c","kind":"movie","service_id":"radarr",
            "service_generation":"g","external_id":"603"}),
    )
    .await
    .unwrap();
    assert_eq!(
        queries.lock().unwrap().as_slice(),
        [HashMap::from([("tmdbId".to_owned(), "603".to_owned())])]
    );
    let refreshed = state
        .db
        .read("test.metadata", |db| {
            Ok(db.query_row(
                "SELECT manager_entity_id FROM metadata_bindings WHERE media_id='0a1b2c'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(refreshed, 17);
    server.abort();
}

// Saving a setting gives the service a new generation. The next refresh keeps the admin's
// episode mappings, a two-episode one included, and drops the old generation's episodes.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_new_generation_keeps_manual_episode_mappings() {
    let (temp, mut state, _cookie) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    let stub = Router::new()
        .route(
            "/api/v3/series",
            get(|| async { Json(json!([{"id":9,"tvdbId":42,"title":"Show"}])) }),
        )
        .route(
            "/api/v3/episode",
            get(|| async {
                Json(json!([
                    {"id":101,"seriesId":9,"seasonNumber":1,"episodeNumber":1,"title":"One"},
                    {"id":102,"seriesId":9,"seasonNumber":1,"episodeNumber":2,"title":"Two"}
                ]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let container = "b".repeat(64);
    let evidence = json!({"id":container,"name":"localhost","running":true,
        "mounts":[{"kind":"bind","source":"/physical","destination":"/media","writable":true}],
        "networks":[{"id":"network","address":"127.0.0.1"}]});
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
        .encrypt("manager:sonarr", b"fixture-key")
        .unwrap();
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES ('sonarr','Fixture','sonarr',?1,?2,'first',?3,?4,'{}','1',1,1)",params![container,port,credential,media_source])?;
        crate::test_support::own(db)?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Shows','shows','/shows')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('5a0e','root','show','show','Show',1)",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES ('5a01','root','season','5a0e','season','Season 1',1)",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES ('5a02','root','episode','5a01','episode','One and Two',1)",[])?;
        Ok(())
    }).await.unwrap();
    let refresh = |generation: &'static str| {
        json!({"media_id":"5a0e","kind":"show","service_id":"sonarr",
            "service_generation":generation,"external_id":"42"})
    };
    run(&state, &refresh("first")).await.unwrap();
    // The admin maps the file to both episodes.
    state.db.write("test.manual", |db| {
        for episode in [101, 102] {
            db.execute("INSERT INTO manager_episode_mappings VALUES ('5a02','sonarr','first',?1,'complex')",[episode])?;
        }
        db.execute("UPDATE manager_services SET generation='second' WHERE id='sonarr'",[])?;
        Ok(())
    }).await.unwrap();
    run(&state, &refresh("second")).await.unwrap();
    let (mappings, stale) = state
        .db
        .read("test.mappings", |db| {
            let mappings: Vec<(String, i64, String)> = db
                .prepare("SELECT service_generation,manager_episode_id,state FROM manager_episode_mappings WHERE media_id='5a02' ORDER BY manager_episode_id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let stale: i64 = db.query_row(
                "SELECT COUNT(*) FROM manager_episodes WHERE service_generation='first'",
                [],
                |r| r.get(0),
            )?;
            Ok((mappings, stale))
        })
        .await
        .unwrap();
    assert_eq!(
        mappings,
        [
            ("second".to_owned(), 101, "complex".to_owned()),
            ("second".to_owned(), 102, "complex".to_owned())
        ]
    );
    assert_eq!(stale, 0);
    server.abort();
}
