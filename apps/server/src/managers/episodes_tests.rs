use super::*;
use crate::managers::status::SeriesEpisode;
use axum::http::StatusCode;
use std::sync::{Arc, Mutex};

fn series() -> Series {
    let episode = |id: i64, season: i64, missing: bool| SeriesEpisode {
        id,
        season,
        missing,
    };
    Series {
        service: "sonarr".into(),
        id: 9,
        seasons: vec![0, 1, 2],
        episodes: vec![
            episode(11, 1, false),
            episode(12, 1, true),
            episode(13, 1, true),
            episode(21, 2, false),
        ],
    }
}
fn input(value: Value) -> Search {
    serde_json::from_value(value).unwrap()
}

#[test]
fn searches_name_episodes_of_the_series_or_one_of_its_seasons() {
    let series = series();
    let command = |value: Value| command(&series, &input(value));
    assert_eq!(
        command(json!({"episodes":[12,11],"monitor":true})).unwrap(),
        Command::Episodes(vec![12, 11])
    );
    assert_eq!(command(json!({"season":2})).unwrap(), Command::Season(2));
    // A missing search names exactly the season's missing episodes.
    assert_eq!(
        command(json!({"season":1,"missing":true})).unwrap(),
        Command::Episodes(vec![12, 13])
    );
    for (value, status) in [
        (json!({"season":2,"missing":true}), StatusCode::CONFLICT),
        (json!({"season":5}), StatusCode::CONFLICT),
        (json!({"episodes":[99]}), StatusCode::CONFLICT),
        (json!({"episodes":[11,11]}), StatusCode::BAD_REQUEST),
        (json!({}), StatusCode::BAD_REQUEST),
        (json!({"episodes":[11],"season":1}), StatusCode::BAD_REQUEST),
        (
            json!({"episodes":[11],"missing":true}),
            StatusCode::BAD_REQUEST,
        ),
        (json!({"season":1,"monitor":true}), StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(command(value.clone()).unwrap_err().0, status, "{value}");
    }
    let many = (1..=EPISODES as i64 + 1).collect::<Vec<_>>();
    assert_eq!(
        episodes(&series, &many).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn release_searches_target_one_episode_or_a_season() {
    let series = series();
    let query = |episode, season| release_query(&series, &ReleaseTarget { episode, season });
    assert_eq!(
        query(Some(13), None).unwrap(),
        [("episodeId", "13".to_owned())]
    );
    assert_eq!(
        query(None, Some(0)).unwrap(),
        [
            ("seriesId", "9".to_owned()),
            ("seasonNumber", "0".to_owned())
        ]
    );
    assert!(query(None, None).is_err());
    assert!(query(Some(13), Some(1)).is_err());
    assert!(query(Some(99), None).is_err());
    assert!(query(None, Some(7)).is_err());
}

#[test]
fn releases_keep_what_the_dialog_compares_and_why_sonarr_rejects_them() {
    let approved = release(&json!({
        "guid":"a","indexerId":3,"indexer":"Indexer","title":"Show.S01E02.1080p",
        "size":2_000_000_000u64,"protocol":"torrent","seeders":12,"leechers":3,
        "quality":{"quality":{"name":"WEBDL-1080p"}},"customFormatScore":250,
        "publishDate":"2026-01-01T00:00:00Z","approved":true,"rejections":[]
    }));
    assert_eq!(
        approved,
        json!({"guid":"a","indexer_id":3,"indexer":"Indexer","title":"Show.S01E02.1080p",
            "size":2_000_000_000u64,"protocol":"torrent","quality":"WEBDL-1080p","score":250,
            "seeders":12,"leechers":3,"published":1_767_225_600,"full_season":false,
            "approved":true,"rejections":[]})
    );
    let rejected = release(&json!({
        "guid":"b","indexerId":3,"fullSeason":true,"approved":true,
        "rejections":["Not wanted in profile",{"reason":"Season pack"},{"message":"Too old"},7]
    }));
    assert_eq!(rejected["full_season"], true);
    // A release with any rejection is never shown as approved.
    assert_eq!(rejected["approved"], false);
    assert_eq!(
        rejected["rejections"],
        json!(["Not wanted in profile", "Season pack", "Too old"])
    );
}

#[tokio::test]
async fn searches_monitor_first_and_send_sonarr_one_command() {
    let (_temp, state, _) = crate::test_support::fixture().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let monitored = calls.clone();
    let commanded = calls.clone();
    let stub = Router::new()
        .route(
            "/api/v3/episode/monitor",
            axum::routing::put(move |Json(body): Json<Value>| {
                monitored.lock().unwrap().push(("monitor", body));
                async { Json(json!([])) }
            }),
        )
        .route(
            "/api/v3/command",
            post(move |Json(body): Json<Value>| {
                commanded.lock().unwrap().push(("command", body));
                async { Json(json!({"id":1})) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('sonarr','Fixture','sonarr','container',8989,'g',X'00','[]','{}','1',1,1,NULL)",[])?;
        crate::test_support::own(db)?;
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
    let series = series();
    run(&connection, &series, &Command::Episodes(vec![12]), true)
        .await
        .unwrap();
    run(&connection, &series, &Command::Season(2), false)
        .await
        .unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ("monitor", json!({"episodeIds":[12],"monitored":true})),
            ("command", json!({"name":"EpisodeSearch","episodeIds":[12]})),
            (
                "command",
                json!({"name":"SeasonSearch","seriesId":9,"seasonNumber":2})
            ),
        ]
    );
    server.abort();
}
