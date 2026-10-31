use super::*;
use crate::test_support::{call, fixture};
use axum::extract::RawQuery;
use axum::http::StatusCode;
use std::sync::Mutex;

/// What the Radarr stub lists, and the changes it was asked to make, in order.
#[derive(Default)]
struct Radarr {
    movies: Vec<Value>,
    files: Vec<Value>,
    /// Where each movie file is on the test's disk.
    disk: Vec<(i64, std::path::PathBuf)>,
    exclusions: Vec<Value>,
    changes: Vec<String>,
}
type Shared = Arc<Mutex<Radarr>>;

async fn list_movies(State(r): State<Shared>) -> Json<Value> {
    Json(json!(r.lock().unwrap().movies))
}
async fn get_movie(
    State(r): State<Shared>,
    Path(id): Path<i64>,
) -> std::result::Result<Json<Value>, StatusCode> {
    let r = r.lock().unwrap();
    let movie = r.movies.iter().find(|m| m["id"] == id);
    movie.cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}
async fn put_movie(
    State(r): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut r = r.lock().unwrap();
    r.changes
        .push(format!("PUT movie/{id} monitored={}", body["monitored"]));
    for movie in r.movies.iter_mut().filter(|m| m["id"] == id) {
        movie["monitored"] = body["monitored"].clone();
    }
    Json(body)
}
async fn delete_movie(
    State(r): State<Shared>,
    Path(id): Path<i64>,
    RawQuery(query): RawQuery,
) -> Json<Value> {
    let mut r = r.lock().unwrap();
    r.changes
        .push(format!("DELETE movie/{id}?{}", query.unwrap_or_default()));
    r.movies.retain(|m| m["id"] != id);
    Json(json!({}))
}
async fn list_files(State(r): State<Shared>) -> Json<Value> {
    Json(json!(r.lock().unwrap().files))
}
async fn get_file(
    State(r): State<Shared>,
    Path(id): Path<i64>,
) -> std::result::Result<Json<Value>, StatusCode> {
    let r = r.lock().unwrap();
    let file = r.files.iter().find(|f| f["id"] == id);
    file.cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}
async fn delete_file(State(r): State<Shared>, Path(id): Path<i64>) -> Json<Value> {
    let mut r = r.lock().unwrap();
    r.changes.push(format!("DELETE moviefile/{id}"));
    for (_, path) in r.disk.iter().filter(|(file, _)| *file == id) {
        std::fs::remove_file(path).unwrap();
    }
    r.files.retain(|f| f["id"] != id);
    for movie in &mut r.movies {
        if movie["movieFile"]["id"] == id {
            movie["hasFile"] = json!(false);
            movie.as_object_mut().unwrap().remove("movieFile");
        }
    }
    Json(json!({}))
}
async fn list_exclusions(State(r): State<Shared>) -> Json<Value> {
    Json(json!(r.lock().unwrap().exclusions))
}
async fn add_exclusion(State(r): State<Shared>, Json(mut body): Json<Value>) -> Json<Value> {
    let mut r = r.lock().unwrap();
    r.changes
        .push(format!("POST exclusions {}", body["tmdbId"]));
    body["id"] = json!(r.exclusions.len() + 1);
    r.exclusions.push(body.clone());
    Json(body)
}

/// A Radarr stub on a local port.
async fn serve(radarr: Shared) -> (u16, tokio::task::JoinHandle<()>) {
    let stub = Router::new()
        .route("/api/v3/movie", get(list_movies))
        .route(
            "/api/v3/movie/{id}",
            get(get_movie).put(put_movie).delete(delete_movie),
        )
        .route("/api/v3/moviefile", get(list_files))
        .route("/api/v3/moviefile/{id}", get(get_file).delete(delete_file))
        .route("/api/v3/command", get(|| async { Json(json!([])) }))
        .route(
            "/api/v3/queue",
            get(|| async { Json(json!({"totalRecords":0,"records":[]})) }),
        )
        .route(
            "/api/v3/exclusions",
            get(list_exclusions).post(add_exclusion),
        )
        .with_state(radarr);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    (port, server)
}

#[tokio::test]
async fn removal_confirms_the_movie_and_excludes_it_once() {
    let (_temp, state, _) = fixture().await;
    let radarr = Shared::default();
    radarr.lock().unwrap().movies =
        vec![json!({"id":1,"tmdbId":603,"title":"One","year":1999,"hasFile":true})];
    let (port, server) = serve(radarr.clone()).await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('radarr','Fixture','radarr','container',7878,'g',X'00','[]','{}','1',1,1,NULL)",[])?;
        crate::test_support::own(db)?;
        Ok(())
    }).await.unwrap();
    let s = service(&state, "radarr").await.unwrap();
    let c = Connection {
        grant: ownership::Grant {
            scope: ownership::Scope::Operational,
            service: s.id.clone(),
            generation: s.generation.clone(),
            container: s.container.clone(),
        },
        ..Connection::validation(
            &state,
            format!("http://127.0.0.1:{port}"),
            String::new(),
            "fixture-key".into(),
            "radarr".into(),
        )
    };
    assert_eq!(
        super::super::retention::ensure_exclusion(&c, 1, "603")
            .await
            .unwrap(),
        (1, true)
    );
    assert_eq!(
        super::super::retention::ensure_exclusion(&c, 1, "603")
            .await
            .unwrap(),
        (1, false),
        "An excluded movie is not excluded twice"
    );
    assert_eq!(
        remove(&c, 1, "604").await.unwrap_err().0,
        StatusCode::CONFLICT,
        "Another movie now has this id"
    );
    assert_eq!(
        remove(&c, 1, "603").await.unwrap_err().0,
        StatusCode::CONFLICT,
        "Radarr would leave its file behind"
    );
    radarr.lock().unwrap().movies[0]["hasFile"] = json!(false);
    remove(&c, 1, "603").await.unwrap();
    assert_eq!(
        radarr.lock().unwrap().changes,
        [
            "POST exclusions 603",
            "DELETE movie/1?deleteFiles=false&addImportExclusion=false"
        ]
    );
    server.abort();
}

/// A library movie with one file, which a Radarr stub lists and manages.
struct Library {
    _temp: tempfile::TempDir,
    state: AppState,
    cookie: String,
    radarr: Shared,
    file: std::path::PathBuf,
    server: tokio::task::JoinHandle<()>,
}

async fn library() -> Library {
    let (temp, mut state, cookie) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(media.join("movies")).unwrap();
    let file = media.join("movies/one.mp4");
    std::fs::write(&file, b"fixture movie").unwrap();
    let media = media.canonicalize().unwrap();
    let file = file.canonicalize().unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    let radarr = Shared::default();
    {
        let mut r = radarr.lock().unwrap();
        let movie_file = json!({"id":11,"movieId":1,"path":"/media/movies/one.mp4"});
        r.movies = vec![
            json!({"id":1,"tmdbId":603,"title":"One","year":1999,"monitored":true,"hasFile":true,"movieFile":movie_file}),
        ];
        r.files = vec![movie_file];
        r.disk = vec![(11, file.clone())];
    }
    let (port, server) = serve(radarr.clone()).await;
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
    let meta = std::fs::metadata(&file).unwrap();
    let modified = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let hash = thelxinoe_catalog::fingerprint(&mut std::fs::File::open(&file).unwrap()).unwrap();
    let stored = file.to_string_lossy().to_string();
    let root = media.join("movies").to_string_lossy().to_string();
    state.db.write("test.fixture", move |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES ('radarr','Fixture','radarr',?1,?2,'g',?3,?4,'{}','1',1,1)",params![container,port,credential,media_source])?;
        crate::test_support::own(db)?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies',?1)",[root])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES ('one','movies','movie','one','One','{\"tmdb_id\":603}',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,ownership,scanned_at) VALUES ('one','movies',?1,'first',?2,?3,?4,'{}','unmanaged',1)",params![stored,meta.len() as i64,modified,hash])?;
        db.execute("INSERT INTO media_sources VALUES ('one','one')",[])?;
        Ok(())
    }).await.unwrap();
    super::super::status::reread(&state).await;
    Library {
        _temp: temp,
        state,
        cookie,
        radarr,
        file,
        server,
    }
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn deleted_or_unmonitored_movies_can_leave_radarr_and_its_lists() {
    let Library {
        _temp,
        state,
        cookie,
        radarr,
        file,
        server,
    } = library().await;
    let removal = call(
        &state,
        "/api/v1/admin/movies/one",
        "DELETE",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(removal.0, StatusCode::CONFLICT, "{}", removal.2);
    assert!(removal.2.to_string().contains("still has files"));
    assert!(file.exists());

    // Deleting the files can also exclude the movie and remove it from Radarr.
    let prepared = call(
        &state,
        "/api/v1/admin/media/operations",
        "POST",
        json!({"media_id":"one","action":"delete"}),
        &cookie,
    )
    .await;
    assert_eq!(prepared.0, StatusCode::OK, "{}", prepared.2);
    let execute = format!(
        "/api/v1/admin/media/operations/{}/execute",
        prepared.2["id"].as_str().unwrap()
    );
    let deleted = call(
        &state,
        &execute,
        "POST",
        json!({"remove":true,"exclude":true}),
        &cookie,
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.2);
    assert!(!file.exists());
    {
        let mut r = radarr.lock().unwrap();
        assert!(r.movies.is_empty());
        assert_eq!(
            r.changes,
            [
                "PUT movie/1 monitored=false",
                "DELETE moviefile/11",
                "POST exclusions 603",
                "DELETE movie/1?deleteFiles=false&addImportExclusion=false",
            ]
        );
        r.changes.clear();
        // Added back without its file, and left unmonitored.
        r.movies = vec![
            json!({"id":2,"tmdbId":603,"title":"One","year":1999,"monitored":false,"hasFile":false}),
            json!({"id":3,"tmdbId":604,"title":"Two","year":2001,"monitored":false,"hasFile":false}),
        ];
    }
    super::super::status::reread(&state).await;

    call(
        &state,
        "/api/v1/admin/media/one/keep",
        "PUT",
        json!({"keep":true}),
        &cookie,
    )
    .await;
    let kept = call(
        &state,
        "/api/v1/admin/movies/one?exclude=true",
        "DELETE",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(kept.0, StatusCode::CONFLICT, "{}", kept.2);
    call(
        &state,
        "/api/v1/admin/media/one/keep",
        "PUT",
        json!({"keep":false}),
        &cookie,
    )
    .await;
    // The library movie, and one only Radarr lists, are removed without their files.
    for path in [
        "/api/v1/admin/movies/one?exclude=true",
        "/api/v1/admin/movies/tmdb:604",
    ] {
        let removed = call(&state, path, "DELETE", Value::Null, &cookie).await;
        assert_eq!(removed.0, StatusCode::OK, "{path}: {}", removed.2);
    }
    {
        let r = radarr.lock().unwrap();
        assert!(r.movies.is_empty());
        assert_eq!(
            r.changes,
            [
                "DELETE movie/2?deleteFiles=false&addImportExclusion=false",
                "DELETE movie/3?deleteFiles=false&addImportExclusion=false",
            ],
            "603 was already excluded, and 604 was not to be"
        );
    }
    let audit = state
        .db
        .read("test.audit", |db| {
            Ok(db
                .prepare("SELECT action,target FROM audit WHERE action LIKE 'movie.%' ORDER BY id")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(
        audit,
        [
            ("movie.remove.exclude".to_owned(), "one".to_owned()),
            ("movie.remove.exclude".to_owned(), "one".to_owned()),
            ("movie.remove".to_owned(), "tmdb:604".to_owned()),
        ]
    );
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn auto_delete_removes_and_excludes_movies_as_its_policy_says() {
    let Library {
        _temp,
        state,
        cookie,
        radarr,
        file,
        server,
    } = library().await;
    state.db.write("test.watched", |db| {
        db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','one',1,1)",[])?;
        db.execute("INSERT INTO settings(key,value) VALUES ('manager.retention.radarr','true')",[])?;
        Ok(())
    }).await.unwrap();
    let policy = |exclude: bool, remove: bool| {
        json!({"enabled":true,"grace_seconds":86400,"exclude_specials":true,"trigger_users":["alice"],
            "exclude_from_lists":exclude,"remove_from_manager":remove})
    };
    let save = async |domain: &str, policy: Value| {
        let path = format!("/api/v1/admin/retention/policy/{domain}");
        let saved = call(&state, &path, "POST", policy, &cookie).await;
        assert_eq!(saved.0, StatusCode::OK, "{}", saved.2);
    };
    let retention = async || {
        call(
            &state,
            "/api/v1/admin/retention",
            "GET",
            Value::Null,
            &cookie,
        )
        .await
        .2
    };
    save("movies", policy(true, false)).await;
    let evaluated = call(
        &state,
        "/api/v1/admin/retention/evaluate",
        "POST",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(evaluated.0, StatusCode::OK, "{}", evaluated.2);
    save("movies", policy(false, true)).await;
    save("shows", policy(true, true)).await;
    let listed = retention().await;
    let options = |domain: &str| {
        let policy = listed["policies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["domain"] == domain)
            .unwrap();
        (
            policy["exclude_from_lists"].clone(),
            policy["remove_from_manager"].clone(),
        )
    };
    assert_eq!(options("movies"), (json!(false), json!(true)));
    assert_eq!(
        options("shows"),
        (json!(false), json!(false)),
        "A deleted season is only unmonitored"
    );
    let item = &listed["items"][0];
    assert_eq!(
        item["state"], "pending",
        "What a deletion does upstream keeps its countdown"
    );
    let path = format!(
        "/api/v1/admin/retention/{}/delete",
        item["id"].as_str().unwrap()
    );
    let deleted = call(&state, &path, "POST", Value::Null, &cookie).await;
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.2);
    assert!(!file.exists());
    assert_eq!(
        radarr.lock().unwrap().changes,
        [
            "PUT movie/1 monitored=false",
            "DELETE moviefile/11",
            "DELETE movie/1?deleteFiles=false&addImportExclusion=false",
        ]
    );
    let records = state
        .db
        .read("test.records", |db| {
            let exclusions =
                db.query_row("SELECT COUNT(*) FROM retention_exclusions", [], |r| {
                    r.get::<_, i64>(0)
                })?;
            let audit = db.query_row(
                "SELECT action FROM audit WHERE action LIKE 'movie.%'",
                [],
                |r| r.get::<_, String>(0),
            )?;
            Ok((exclusions, audit))
        })
        .await
        .unwrap();
    assert_eq!(records, (0, "movie.remove".to_owned()));
    server.abort();
}
