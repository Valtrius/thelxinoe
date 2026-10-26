use crate::test_support::{call, fixture};
use axum::{
    Json, Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::HashMap;

const KEY: &str = "fixture-api-key";

/// A Jellyfin that reads the same files under another mount, as its own container would.
fn jellyfin() -> Router {
    let authorized = |headers: &HeaderMap| {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains(&format!("Token=\"{KEY}\"")))
    };
    let episode = |id: &str, series: &str, name: &str, season: i64, number: i64, path: &str| {
        json!({"Id":id,"Name":format!("Episode {number}"),"Type":"Episode","SeriesId":series,"SeriesName":name,
            "ParentIndexNumber":season,"IndexNumber":number,"Path":path,"ProviderIds":{},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":false}})
    };
    let alice_played = vec![
        // The same file under Jellyfin's own mount.
        json!({"Id":"matrix","Name":"The Matrix","Type":"Movie","ProductionYear":1999,
            "Path":"/data/movies/The Matrix (1999)/The Matrix (1999).mkv","ProviderIds":{"Tmdb":"603"},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":true}}),
        // A file replaced since; Radarr's TMDB ID still names it.
        json!({"Id":"heat","Name":"Heat","Type":"Movie","ProductionYear":1995,
            "Path":"/data/movies/Heat (1995)/Heat (1995) 720p.mkv","ProviderIds":{"Tmdb":"949"},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":false}}),
        // Only the title matches.
        json!({"Id":"spider","Name":"Spider-Man: Into the Spider-Verse","Type":"Movie","ProductionYear":2018,
            "Path":"/elsewhere/spider.mkv","ProviderIds":{},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":false}}),
        json!({"Id":"dune","Name":"Dune: Part Two","Type":"Movie","ProductionYear":2024,
            "Path":"/data/movies/Dune Part Two (2024)/Dune Part Two (2024).mkv","ProviderIds":{},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":false}}),
        episode(
            "sev1",
            "severance",
            "Severance",
            1,
            1,
            "/data/tv/Severance/Season 1/Severance - S01E01.mkv",
        ),
        // Renamed since; the series is placed by its other files.
        episode(
            "sev2",
            "severance",
            "Severance",
            1,
            2,
            "/data/tv/Severance/Season 1/Severance - S01E02 - Good News.mkv",
        ),
        json!({"Id":"sev4","Name":"Episodes 4-5","Type":"Episode","SeriesId":"severance","SeriesName":"Severance",
            "ParentIndexNumber":1,"IndexNumber":4,"IndexNumberEnd":5,
            "Path":"/data/tv/Severance/Season 1/Severance - S01E04E05.mkv","ProviderIds":{},
            "UserData":{"Played":true,"PlaybackPositionTicks":0,"IsFavorite":false}}),
        episode("who1", "who", "Doctor Who", 1, 1, "/other/who-101.mkv"),
    ];
    let alice_resumable = vec![
        json!({"Id":"sev3","Name":"Episode 3","Type":"Episode","SeriesId":"severance",
        "SeriesName":"Severance","ParentIndexNumber":1,"IndexNumber":3,
        "Path":"/data/tv/Severance/Season 1/Severance - S01E03.mkv","ProviderIds":{},
        "UserData":{"Played":false,"PlaybackPositionTicks":6_000_000_000i64,"IsFavorite":false,
            "LastPlayedDate":"2024-05-03T20:11:43.1234567Z"}}),
    ];
    let alice_favorites = vec![
        alice_played[0].clone(),
        json!({"Id":"severance","Name":"Severance","Type":"Series","ProductionYear":2022,"ProviderIds":{},
            "UserData":{"Played":false,"PlaybackPositionTicks":0,"IsFavorite":true}}),
    ];
    let mut bob_matrix = alice_played[0].clone();
    bob_matrix["UserData"]["IsFavorite"] = json!(false);
    let bob_played = vec![bob_matrix];
    let bob_resumable = vec![
        json!({"Id":"heat","Name":"Heat","Type":"Movie","ProductionYear":1995,
        "Path":"/data/movies/Heat (1995)/Heat (1995) 720p.mkv","ProviderIds":{"Tmdb":"949"},
        "UserData":{"Played":false,"PlaybackPositionTicks":12_000_000_000i64,"IsFavorite":false,
            "LastPlayedDate":"2020-01-01T00:00:00Z"}}),
    ];
    let series = [
        json!({"Id":"severance","Name":"Severance","Type":"Series","ProductionYear":2022,"ProviderIds":{}}),
        json!({"Id":"who","Name":"Doctor Who","Type":"Series","ProductionYear":2005,"ProviderIds":{}}),
    ];
    let lists: HashMap<(&str, &str), Vec<Value>> = HashMap::from([
        (("u-alice", "IsPlayed"), alice_played),
        (("u-alice", "IsResumable"), alice_resumable),
        (("u-alice", "IsFavorite"), alice_favorites),
        (("u-bob", "IsPlayed"), bob_played),
        (("u-bob", "IsResumable"), bob_resumable),
    ]);
    Router::new()
        .route(
            "/jf/System/Info",
            get(move |headers: HeaderMap| async move {
                if !authorized(&headers) {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Ok(Json(json!({"ServerName":"Maison","Version":"10.11.3"})))
            }),
        )
        .route(
            "/jf/Users",
            get(move |headers: HeaderMap| async move {
                if !authorized(&headers) {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Ok(Json(json!([
                    {"Id":"u-alice","Name":"Alice","Policy":{"IsAdministrator":true}},
                    {"Id":"u-bob","Name":"bob","Policy":{}},
                    {"Id":"u-guest","Name":"Invité","Policy":{"IsDisabled":true}}
                ])))
            }),
        )
        .route(
            "/jf/Items",
            get(
                move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
                    if !authorized(&headers) {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    let items: Vec<Value> = if let Some(ids) = query.get("ids") {
                        let ids: Vec<&str> = ids.split(',').collect();
                        series
                            .iter()
                            .filter(|s| ids.contains(&s["Id"].as_str().unwrap()))
                            .cloned()
                            .collect()
                    } else {
                        let key = (query["userId"].as_str(), query["filters"].as_str());
                        lists
                            .iter()
                            .find(|((user, filter), _)| (*user, *filter) == key)
                            .map(|(_, items)| items.clone())
                            .unwrap_or_default()
                    };
                    let start: usize = query.get("startIndex").map_or(0, |v| v.parse().unwrap());
                    let limit: usize = query
                        .get("limit")
                        .map_or(usize::MAX, |v| v.parse().unwrap());
                    let total = items.len();
                    let page: Vec<Value> = items.into_iter().skip(start).take(limit).collect();
                    Ok(Json(json!({"Items":page,"TotalRecordCount":total})))
                },
            ),
        )
}

async fn catalog(state: &crate::AppState) {
    state
        .db
        .write("test.catalog", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies','/media/movies'),('tv','Shows','shows','/media/tv')", [])?;
            let probe = json!({"format":{"duration":"3600.0"}}).to_string();
            let file = |id: &str, root: &str, path: &str| -> rusqlite::Result<()> {
                db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,?2,?3,'g',1,'1',?1,?4,1)", params![id, root, path, probe])?;
                Ok(())
            };
            file("f-matrix", "movies", "/media/movies/The Matrix (1999)/The Matrix (1999).mkv")?;
            file("f-heat", "movies", "/media/movies/Heat (1995)/Heat (1995) 2160p.mkv")?;
            file("f-spider", "movies", "/media/movies/Spider-Man Into the Spider-Verse (2018)/Spider-Man Into the Spider-Verse (2018).mkv")?;
            file("f-sev1", "tv", "/media/tv/Severance/Season 1/Severance - S01E01.mkv")?;
            file("f-sev2", "tv", "/media/tv/Severance/Season 1/Severance - S01E02.mkv")?;
            file("f-sev3", "tv", "/media/tv/Severance/Season 1/Severance - S01E03.mkv")?;
            file("f-sev45", "tv", "/media/tv/Severance/Season 1/Severance - S01E04E05.mkv")?;
            file("f-who1", "tv", "/media/tv/Doctor Who (2005)/Season 1/Doctor Who (2005) - S01E01.mkv")?;
            db.execute_batch(r#"
                INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,year,metadata,created_at) VALUES
                 ('matrix','movies','movie',NULL,'the matrix:1999','The Matrix',NULL,1999,'{}',1),
                 ('heat','movies','movie',NULL,'heat:1995','Heat',NULL,1995,'{"tmdb_id":949}',1),
                 ('spider','movies','movie',NULL,'spider-man into the spider-verse:2018','Spider-Man Into the Spider-Verse',NULL,2018,'{}',1),
                 ('sev','tv','show',NULL,'severance','Severance',NULL,NULL,'{}',1),
                 ('sev-s1','tv','season','sev','severance:s1','Season 1',1,NULL,'{}',1),
                 ('sev-e1','tv','episode','sev-s1','severance:s1:e1','Severance · S01E01',1,NULL,'{}',1),
                 ('sev-e2','tv','episode','sev-s1','severance:s1:e2','Severance · S01E02',2,NULL,'{}',1),
                 ('sev-e3','tv','episode','sev-s1','severance:s1:e3','Severance · S01E03',3,NULL,'{}',1),
                 ('sev-e4','tv','episode','sev-s1','severance:s1:e4','Severance · S01E04',4,NULL,'{}',1),
                 ('sev-e5','tv','episode','sev-s1','severance:s1:e5','Severance · S01E05',5,NULL,'{}',1),
                 ('who','tv','show',NULL,'doctor who (2005)','Doctor Who (2005)',NULL,NULL,'{}',1),
                 ('who-s1','tv','season','who','doctor who (2005):s1','Season 1',1,NULL,'{}',1),
                 ('who-e1','tv','episode','who-s1','doctor who (2005):s1:e1','Doctor Who (2005) · S01E01',1,NULL,'{}',1);
                INSERT INTO media_sources(media_id,file_id) VALUES
                 ('matrix','f-matrix'),('heat','f-heat'),('spider','f-spider'),('sev-e1','f-sev1'),('sev-e2','f-sev2'),
                 ('sev-e3','f-sev3'),('sev-e4','f-sev45'),('sev-e5','f-sev45'),('who-e1','f-who1');
                -- Bob already watched The Matrix and resumed Heat here more recently than in Jellyfin.
                INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('bob','matrix',1,1);
                INSERT INTO edition_progress(user_id,media_id,edition,position,duration,updated_at) VALUES ('bob','heat','',300,3600,4102444800);
                -- Auto-delete waits for Alice on movies, and only The Matrix is a managed file.
                UPDATE media_files SET ownership='managed' WHERE id='f-matrix';
                UPDATE retention_policies SET enabled=1,trigger_users='["alice"]' WHERE domain='movies';
            "#)?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn watched(state: &crate::AppState, user: &'static str) -> Vec<String> {
    state
        .db
        .read("test.watched", move |db| {
            Ok(db
                .prepare("SELECT media_id FROM media_state WHERE user_id=?1 AND watched=1 ORDER BY media_id")?
                .query_map([user], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn an_import_adds_jellyfin_state_to_the_titles_it_finds() {
    let (_temp, state, cookie) = fixture().await;
    catalog(&state).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, jellyfin()).await.unwrap() });
    let url = format!("http://127.0.0.1:{port}/jf/");

    let (status, _, body) = call(
        &state,
        "/api/v1/admin/jellyfin-import/server",
        "PUT",
        json!({"url":url,"api_key":"wrong"}),
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("refused the API key")
    );
    let (status, _, body) = call(
        &state,
        "/api/v1/admin/jellyfin-import/server",
        "PUT",
        json!({"url":url,"api_key":KEY}),
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"url":format!("http://127.0.0.1:{port}/jf"),"name":"Maison","version":"10.11.3"})
    );

    let (status, _, body) = call(
        &state,
        "/api/v1/admin/jellyfin-import/accounts",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let accounts: Vec<(&str, &Value, u64, u64, u64)> = body["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap(),
                &a["suggested"],
                a["played"].as_u64().unwrap(),
                a["in_progress"].as_u64().unwrap(),
                a["favorites"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        accounts,
        [
            ("Alice", &json!("alice"), 8, 1, 2),
            ("bob", &json!("bob"), 1, 1, 0),
            ("Invité", &Value::Null, 0, 0, 0)
        ]
    );

    let request = json!({"accounts":[{"jellyfin_id":"u-alice","user_id":"alice"},{"jellyfin_id":"u-bob","user_id":"bob"}],
        "watched":true,"resume":true,"favorites":true});
    let (status, _, preview) = call(
        &state,
        "/api/v1/admin/jellyfin-import/preview",
        "POST",
        request.clone(),
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let counts = |report: &Value, index: usize| {
        let a = &report["accounts"][index];
        [
            "watched",
            "already_watched",
            "resume",
            "favorites",
            "not_found",
        ]
        .map(|k| a[k].as_u64().unwrap())
    };
    // Alice: The Matrix, Heat, Spider-Man, Severance 1, 2, 4 and 5, and Doctor Who 1.
    assert_eq!(counts(&preview, 0), [8, 0, 1, 2, 1]);
    // Bob watched The Matrix here already and resumed Heat here after Jellyfin.
    assert_eq!(counts(&preview, 1), [0, 1, 0, 0, 0]);
    assert_eq!(preview["retention"], 1);
    assert_eq!(
        preview["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| (
                g["key"].as_str().unwrap(),
                g["jellyfin"].as_str().unwrap(),
                g["title"].as_str().unwrap(),
                g["items"].as_u64().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "movie:spider",
                "Spider-Man: Into the Spider-Verse (2018)",
                "Spider-Man Into the Spider-Verse (2018)",
                1
            ),
            ("series:who", "Doctor Who (2005)", "Doctor Who (2005)", 1)
        ]
    );
    assert_eq!(preview["groups"][1]["accounts"][0]["watched"], 1);
    assert_eq!(
        preview["not_found"],
        json!([{"account":"Alice","title":"Dune: Part Two","year":2024,"episodes":null,"watched":1,"resume":0,"favorite":0,"position":null}])
    );
    // A preview changes nothing.
    assert!(watched(&state, "alice").await.is_empty());

    let mut request = request;
    request["excluded"] = json!(["series:who"]);
    let (status, _, report) = call(
        &state,
        "/api/v1/admin/jellyfin-import",
        "POST",
        request,
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(counts(&report, 0), [7, 0, 1, 2, 1]);
    assert_eq!(
        watched(&state, "alice").await,
        [
            "heat", "matrix", "sev-e1", "sev-e2", "sev-e4", "sev-e5", "spider"
        ]
    );
    assert_eq!(watched(&state, "bob").await, ["matrix"]);
    let progress = state
        .db
        .read("test.progress", |db| {
            Ok(db
                .prepare("SELECT user_id,media_id,position,updated_at FROM edition_progress ORDER BY user_id,media_id")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, i64>(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(
        progress,
        [
            ("alice".to_owned(), "sev-e3".to_owned(), 600.0, 1714767103),
            ("bob".to_owned(), "heat".to_owned(), 300.0, 4102444800)
        ]
    );
    let favorites = state
        .db
        .read("test.favorites", |db| {
            Ok(db
                .prepare("SELECT media_id FROM media_state WHERE user_id='alice' AND favorite=1 ORDER BY media_id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(favorites, ["matrix", "sev"]);

    let (_, _, overview) = call(
        &state,
        "/api/v1/admin/jellyfin-import",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(overview["server"]["name"], "Maison");
    assert_eq!(overview["last"]["accounts"][0]["watched"], 7);
    assert!(overview["last"].get("groups").is_none());

    let (status, _, _) = call(
        &state,
        "/api/v1/admin/jellyfin-import/server",
        "DELETE",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, overview) = call(
        &state,
        "/api/v1/admin/jellyfin-import",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert!(overview["server"].is_null());
    server.abort();
}
