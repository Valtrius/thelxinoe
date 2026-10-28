use super::*;
use crate::test_support::fixture;
use axum::{Router, extract::Query, routing::get};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Serves `stub` as an owned `kind` manager whose container shares the server's media mount.
async fn serve_manager(
    temp: &tempfile::TempDir,
    state: &mut AppState,
    kind: &'static str,
    generation: &'static str,
    stub: Router,
) -> tokio::task::JoinHandle<()> {
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
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
        .encrypt(&format!("manager:{kind}"), b"fixture-key")
        .unwrap();
    state.db.write("test.manager", move |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES (?1,'Fixture',?1,?2,?3,?4,?5,?6,'{}','1',1,1)",params![kind,container,port,generation,credential,media_source])?;
        crate::test_support::own(db)?;
        Ok(())
    }).await.unwrap();
    server
}

// A refresh asks Radarr for its one movie by TMDB ID instead of downloading the library.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_refresh_looks_up_its_title_by_external_id() {
    let (temp, mut state, _cookie) = fixture().await;
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
    let server = serve_manager(&temp, &mut state, "radarr", "g", stub).await;
    state.db.write("test.fixture", |db| {
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
    let server = serve_manager(&temp, &mut state, "sonarr", "first", stub).await;
    state.db.write("test.fixture", |db| {
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

// Auto-delete compares each season's episodes against the summary a refresh stores, so a
// season without one is never removed.
#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_show_refresh_stores_its_season_summary() {
    let (temp, mut state, _cookie) = fixture().await;
    let stub = Router::new()
        .route(
            "/api/v3/series",
            get(|| async {
                Json(json!([{"id":9,"tvdbId":42,"title":"Show",
                    "seasons":[{"seasonNumber":1,"monitored":true},{"seasonNumber":2,"monitored":true}]}]))
            }),
        )
        .route(
            "/api/v3/episode",
            get(|| async {
                Json(json!([
                    {"id":101,"seriesId":9,"seasonNumber":1,"episodeNumber":1,"airDate":"2024-02-01"},
                    {"id":102,"seriesId":9,"seasonNumber":1,"episodeNumber":2,"airDate":"2024-01-25"},
                    {"id":201,"seriesId":9,"seasonNumber":2,"episodeNumber":1}
                ]))
            }),
        );
    let server = serve_manager(&temp, &mut state, "sonarr", "g", stub).await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Shows','shows','/shows')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('5a0e','root','show','show','Show',1)",[])?;
        Ok(())
    }).await.unwrap();
    run(
        &state,
        &json!({"media_id":"5a0e","kind":"show","service_id":"sonarr",
            "service_generation":"g","external_id":"42"}),
    )
    .await
    .unwrap();
    let metadata = state
        .db
        .read("test.metadata", |db| {
            Ok(
                db.query_row("SELECT metadata FROM media WHERE id='5a0e'", [], |r| {
                    r.get::<_, String>(0)
                })?,
            )
        })
        .await
        .unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(
        metadata["seasons"],
        json!([
            {"season_number":1,"episode_count":2,"air_date":"2024-01-25"},
            {"season_number":2,"episode_count":1,"air_date":null}
        ])
    );
    server.abort();
}

#[test]
fn titles_keep_their_ratings_certification_and_studio() {
    let movie = normalize(
        "movie",
        &json!({"title":"Feature","certification":"PG-13","studio":"Walt Disney Pictures",
            "ratings":{"tmdb":{"value":6.665},"rottenTomatoes":{"value":30}}}),
    );
    assert_eq!(
        [
            &movie["certification"],
            &movie["studio"],
            &movie["vote_average"],
            &movie["critic_rating"]
        ],
        [
            &json!("PG-13"),
            &json!("Walt Disney Pictures"),
            &json!(6.665),
            &json!(30.0)
        ]
    );
    // Sonarr names the network that first aired a series.
    let show = normalize(
        "show",
        &json!({"title":"Show","certification":"TV-MA","network":"HBO","ratings":{"value":8.4}}),
    );
    assert_eq!(show["studio"], "HBO");
    assert_eq!(show["vote_average"], 8.4);
    let unrated = normalize(
        "movie",
        &json!({"title":"Plain","ratings":{"rottenTomatoes":{"value":0}}}),
    );
    assert_eq!(unrated["studio"], Value::Null);
    assert!(unrated.get("critic_rating").is_none());
    assert!(
        normalize("album", &json!({"title":"Album"}))
            .get("studio")
            .is_none()
    );
}
