use super::*;
use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use std::sync::atomic::{AtomicUsize, Ordering};

const NOW: i64 = 1_800_000_000;
const PAST: &str = "2020-01-01T00:00:00Z";
const FUTURE: &str = "2035-01-01T00:00:00Z";
const FUTURE_AT: i64 = 2_051_222_400;
/// Release dates have no time of day, so they are kept at noon UTC.
const FUTURE_DAY: i64 = FUTURE_AT + 43_200;

fn inventory(kind: &str, titles: Value, episodes: Value, queue: Value) -> Inventory {
    let titles = serde_json::from_value::<Vec<Title>>(titles)
        .unwrap()
        .into_iter()
        .map(Title::tidy)
        .collect();
    let episodes = episodes
        .as_object()
        .into_iter()
        .flatten()
        .map(|(series, list)| {
            (
                series.parse().unwrap(),
                serde_json::from_value::<Vec<Episode>>(list.clone())
                    .unwrap()
                    .into_iter()
                    .map(Episode::tidy)
                    .collect(),
            )
        })
        .collect();
    // Each fixture inventory is read from the service "<kind>-1".
    Inventory::new(
        &format!("{kind}-1"),
        titles,
        episodes,
        serde_json::from_value(queue).unwrap(),
    )
}
fn snapshot(services: Vec<(&str, Inventory)>) -> Snapshot {
    Snapshot {
        services: services
            .into_iter()
            .map(|(kind, inventory)| (kind.to_owned(), Arc::new(inventory)))
            .collect(),
    }
}
fn shown(status: Status, admin: bool) -> Value {
    json!(status.for_viewer(admin))
}
fn found<'a>(snapshot: &'a Snapshot, kind: &str, title: i64) -> Found<'a> {
    let inventory = &snapshot.services[kind];
    Found {
        inventory,
        title: &inventory.titles[&title],
    }
}

fn radarr() -> Inventory {
    inventory(
        "radarr",
        json!([
            {"id":1,"tmdbId":101,"title":"Owned","monitored":true,"hasFile":true,"isAvailable":true,
             "images":[{"coverType":"poster","url":"/MediaCover/1/poster.jpg?lastWrite=1"},{"coverType":"fanart","url":"/MediaCover/1/fanart.jpg"},{"coverType":"banner","url":"/MediaCover/1/banner.jpg"},{"coverType":"poster","remoteUrl":"https://image.tmdb.org/x.jpg"},{"coverType":"clearlogo","url":"/MediaCover/1/clearlogo.png"}]},
            {"id":2,"tmdbId":102,"title":"Fetching","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":3,"tmdbId":103,"title":"Soon","monitored":true,"hasFile":false,"isAvailable":false,
             "inCinemas":PAST,"digitalRelease":FUTURE,"physicalRelease":"2036-01-01T00:00:00Z"},
            {"id":4,"tmdbId":104,"title":"Wanted","monitored":true,"hasFile":false,"isAvailable":true,"digitalRelease":PAST,
             "added":PAST,"genres":["Drama"],"ratings":{"imdb":{"value":7.1},"tmdb":{"value":6.5}}},
            {"id":5,"tmdbId":105,"title":"Ignored","monitored":false,"hasFile":false,"isAvailable":true},
            {"id":6,"tmdbId":106,"title":"Waiting","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":7,"tmdbId":0,"title":"No identity","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":8,"tmdbId":0,"title":"Blocked","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":9,"tmdbId":0,"title":"Shelved","monitored":false,"hasFile":false,"isAvailable":false,"digitalRelease":FUTURE}
        ]),
        json!({}),
        json!([
            {"id":50,"movieId":2,"status":"downloading","trackedDownloadState":"downloading","size":400.0,"sizeleft":100.0,"estimatedCompletionTime":FUTURE},
            {"id":51,"movieId":6,"status":"queued","trackedDownloadState":"downloading","size":800,"sizeleft":800},
            {"id":52,"movieId":4,"status":"failed","trackedDownloadState":"failed","size":10,"sizeleft":5},
            {"id":53,"movieId":8,"status":"completed","trackedDownloadState":"importBlocked","size":10,"sizeleft":0}
        ]),
    )
}

#[test]
fn queue_records_become_download_progress() {
    let record = |status: &str, tracked: &str, size: i64, left: i64| QueueItem {
        status: Some(status.into()),
        tracked_download_state: Some(tracked.into()),
        size,
        sizeleft: left,
        ..QueueItem::default()
    };
    let downloading = transfer(&[record("downloading", "downloading", 400, 100)]).unwrap();
    assert_eq!(downloading.state, Phase::Downloading);
    assert_eq!(downloading.progress, Some(0.75));
    // Importing counts as complete and outweighs nothing else in its bytes.
    let importing = transfer(&[
        record("completed", "importPending", 100, 0),
        record("downloading", "downloading", 100, 100),
    ])
    .unwrap();
    assert_eq!(importing.progress, Some(0.5));
    assert_eq!(
        transfer(&[record("completed", "importing", 0, 0)])
            .unwrap()
            .progress,
        Some(1.0)
    );
    for status in ["queued", "delay", "paused", "downloadClientUnavailable"] {
        let queued = transfer(&[record(status, "downloading", 10, 10)]).unwrap();
        assert_eq!(queued.state, Phase::Queued, "{status}");
        assert_eq!(queued.progress, None);
    }
    // Failures and warnings only count while their download is still running.
    assert!(transfer(&[record("failed", "failed", 10, 5)]).is_none());
    assert!(transfer(&[record("warning", "importBlocked", 10, 0)]).is_none());
    // A finished download waiting on, or blocked before, its import is importing.
    assert_eq!(
        transfer(&[record("completed", "importBlocked", 10, 0)])
            .unwrap()
            .progress,
        Some(1.0)
    );
    assert_eq!(
        transfer(&[record("warning", "downloading", 10, 5)])
            .unwrap()
            .state,
        Phase::Downloading
    );
    let mut timed = record("queued", "downloading", 10, 10);
    timed.estimated_completion_time = Some(FUTURE.into());
    assert_eq!(transfer(&[timed]).unwrap().eta, Some(FUTURE_AT));
}

#[test]
fn movies_follow_file_queue_release_and_monitoring_precedence() {
    let inventory = radarr();
    let movie =
        |id: i64, library: bool| movie_status(&inventory, &inventory.titles[&id], library, NOW);
    assert_eq!(
        shown(movie(1, false), true),
        json!({"state":"available","monitored":true})
    );
    assert_eq!(
        shown(movie(2, false), true),
        json!({"state":"downloading","progress":0.75,"eta":FUTURE_AT,"monitored":true})
    );
    // The earliest future release date, not the cinema date already past.
    assert_eq!(
        shown(movie(3, false), true),
        json!({"state":"upcoming","release":FUTURE_DAY,"monitored":true})
    );
    assert_eq!(
        shown(movie(4, false), true),
        json!({"state":"missing","monitored":true})
    );
    assert_eq!(
        shown(movie(5, false), true),
        json!({"state":"unmonitored","monitored":false})
    );
    assert_eq!(movie(6, false).state, Phase::Queued);
    assert_eq!(
        shown(movie(8, false), true),
        json!({"state":"downloading","progress":1.0,"monitored":true})
    );
    // An unmonitored title will not arrive, even before its release.
    assert_eq!(
        shown(movie(9, false), false),
        json!({"state":"unavailable","release":FUTURE_DAY})
    );
    // A playable library copy wins over everything Radarr reports.
    assert_eq!(movie(4, true).state, Phase::Available);
    assert_eq!(movie(3, true).release, Some(FUTURE_DAY));
    // Users never see administration: missing and unmonitored are just unavailable.
    assert_eq!(
        shown(movie(4, false), false),
        json!({"state":"unavailable"})
    );
    assert_eq!(
        shown(movie(5, false), false),
        json!({"state":"unavailable"})
    );
    assert_eq!(
        shown(movie(3, false), false),
        json!({"state":"upcoming","release":FUTURE_DAY})
    );
    // Only manager-local poster, fanart and logo paths are kept.
    let owned = &inventory.titles[&1];
    assert_eq!(
        owned.image("poster"),
        Some("/MediaCover/1/poster.jpg?lastWrite=1")
    );
    assert_eq!(
        owned.image("clearlogo"),
        Some("/MediaCover/1/clearlogo.png")
    );
    assert_eq!(owned.images.len(), 3);
    assert_eq!(inventory.titles[&7].tmdb_id, None);
}

fn sonarr() -> Inventory {
    inventory(
        "sonarr",
        json!([
            {"id":10,"tvdbId":201,"tmdbId":301,"title":"Series","monitored":true,"status":"continuing","ratings":{"votes":10,"value":8.0},
             "seasons":[{"seasonNumber":0,"monitored":false},{"seasonNumber":1,"monitored":true},{"seasonNumber":2,"monitored":false}],
             "images":[{"coverType":"fanart","url":"/MediaCover/10/fanart.jpg"}]},
            {"id":11,"tvdbId":202,"title":"Unwatched series","monitored":false,"status":"ended","seasons":[{"seasonNumber":1,"monitored":true}]}
        ]),
        json!({
            "10":[
                {"id":1001,"seasonNumber":1,"episodeNumber":1,"title":"Pilot","airDateUtc":PAST,"monitored":true,"hasFile":true,"runtime":45},
                {"id":1002,"seasonNumber":1,"episodeNumber":2,"title":"Second","airDateUtc":PAST,"monitored":true,"hasFile":false},
                {"id":1003,"seasonNumber":1,"episodeNumber":3,"title":"Third","airDateUtc":PAST,"monitored":true,"hasFile":false},
                {"id":2001,"seasonNumber":2,"episodeNumber":1,"title":"Next","airDateUtc":FUTURE,"monitored":true,"hasFile":false},
                {"id":2002,"seasonNumber":2,"episodeNumber":2,"title":"Unscheduled","monitored":true,"hasFile":false},
                {"id":1500,"seasonNumber":0,"episodeNumber":1,"title":"Special","airDateUtc":PAST,"monitored":false,"hasFile":false}
            ],
            "11":[
                {"id":3001,"seasonNumber":1,"episodeNumber":1,"airDateUtc":PAST,"monitored":true,"hasFile":false}
            ]
        }),
        json!([
            {"id":60,"seriesId":10,"episodeId":1003,"status":"downloading","trackedDownloadState":"downloading","size":1000,"sizeleft":250},
            {"id":61,"seriesId":10,"episodeId":1001,"status":"downloading","trackedDownloadState":"downloading","size":1000,"sizeleft":0}
        ]),
    )
}

#[test]
fn episodes_seasons_and_shows_aggregate_their_episodes() {
    let snapshot = snapshot(vec![("sonarr", sonarr())]);
    let series = found(&snapshot, "sonarr", 10);
    let episode = |id: i64, library: bool| {
        let episode = series.inventory.episodes[&10]
            .iter()
            .find(|e| e.id == id)
            .unwrap();
        episode_status(series.inventory, series.title, episode, library, NOW)
    };
    assert_eq!(episode(1001, false).state, Phase::Available);
    assert_eq!(
        shown(episode(1002, false), true),
        json!({"state":"missing","monitored":true})
    );
    assert_eq!(episode(1002, true).state, Phase::Available);
    assert_eq!(episode(1003, false).state, Phase::Downloading);
    assert_eq!(
        shown(episode(2001, false), false),
        json!({"state":"upcoming","release":FUTURE_AT})
    );
    assert_eq!(episode(2002, false).state, Phase::Upcoming);
    assert_eq!(
        shown(episode(1500, false), true),
        json!({"state":"unmonitored","monitored":false})
    );

    // An episode in the queue makes its season download; a file already present doesn't.
    assert_eq!(
        shown(season_status(series, 1, &HashSet::new(), NOW), true),
        json!({"state":"downloading","progress":0.75,"episodes":{"available":1,"total":3},"downloading":1,"monitored":true})
    );
    // An unmonitored season is not coming, even if its episodes have not aired.
    assert_eq!(
        shown(season_status(series, 2, &HashSet::new(), NOW), true),
        json!({"state":"unmonitored","release":FUTURE_AT,"episodes":{"available":0,"total":0},"monitored":false})
    );
    // An episode obtained before its air date counts as both aired and present.
    assert_eq!(
        shown(season_status(series, 2, &HashSet::from([2001]), NOW), true)["episodes"],
        json!({"available":1,"total":1})
    );
    // Specials stay out of the show; a library copy counts as available.
    assert_eq!(
        shown(show_status(series, &HashSet::from([1002]), NOW), false),
        json!({"state":"downloading","progress":0.75,"release":FUTURE_AT,"episodes":{"available":2,"total":3},"downloading":1})
    );
    let mut idle = sonarr();
    idle.queue.clear();
    let idle = snapshot_of(idle);
    let series = found(&idle, "sonarr", 10);
    assert_eq!(
        season_status(series, 1, &HashSet::new(), NOW).state,
        Phase::Partial
    );
    assert_eq!(
        season_status(series, 1, &HashSet::from([1002, 1003]), NOW).state,
        Phase::Available
    );
    assert_eq!(
        season_status(series, 0, &HashSet::new(), NOW).state,
        Phase::Unmonitored
    );
    let unwatched = found(&idle, "sonarr", 11);
    assert_eq!(
        shown(show_status(unwatched, &HashSet::new(), NOW), true),
        json!({"state":"unmonitored","episodes":{"available":0,"total":1},"monitored":false})
    );
    assert_eq!(
        shown(show_status(unwatched, &HashSet::new(), NOW), false),
        json!({"state":"unavailable","episodes":{"available":0,"total":1}})
    );
}

#[test]
fn episode_totals_leave_out_aired_episodes_that_are_not_monitored() {
    let snapshot = snapshot_of(inventory(
        "sonarr",
        json!([
            {"id":20,"tvdbId":220,"title":"Half wanted","monitored":true,
             "seasons":[{"seasonNumber":1,"monitored":true},{"seasonNumber":2,"monitored":false}]},
            {"id":21,"tvdbId":221,"title":"Future only","monitored":true,"status":"ended",
             "seasons":[{"seasonNumber":1,"monitored":true}]}
        ]),
        json!({
            "20":[
                {"id":1,"seasonNumber":1,"episodeNumber":1,"airDateUtc":PAST,"monitored":true,"hasFile":true},
                {"id":2,"seasonNumber":1,"episodeNumber":2,"airDateUtc":PAST,"monitored":true,"hasFile":false},
                {"id":3,"seasonNumber":2,"episodeNumber":1,"airDateUtc":PAST,"monitored":false,"hasFile":true},
                {"id":4,"seasonNumber":2,"episodeNumber":2,"airDateUtc":PAST,"monitored":false,"hasFile":false}
            ],
            "21":[
                {"id":5,"seasonNumber":1,"episodeNumber":1,"airDateUtc":PAST,"monitored":false,"hasFile":false}
            ]
        }),
        json!([]),
    ));
    let series = found(&snapshot, "sonarr", 20);
    // An unmonitored episode counts once it is obtained.
    assert_eq!(
        shown(show_status(series, &HashSet::new(), NOW), true),
        json!({"state":"partial","episodes":{"available":2,"total":3},"monitored":true})
    );
    assert_eq!(
        shown(show_status(series, &HashSet::from([2]), NOW), true),
        json!({"state":"available","episodes":{"available":3,"total":3},"monitored":true})
    );
    assert_eq!(
        shown(season_status(series, 2, &HashSet::new(), NOW), true),
        json!({"state":"available","episodes":{"available":1,"total":1},"monitored":false})
    );
    // Aired episodes that are not wanted do not make a monitored show upcoming.
    assert_eq!(
        shown(
            show_status(found(&snapshot, "sonarr", 21), &HashSet::new(), NOW),
            true
        ),
        json!({"state":"unmonitored","episodes":{"available":0,"total":0},"monitored":true})
    );
}
fn snapshot_of(inventory: Inventory) -> Snapshot {
    snapshot(vec![("sonarr", inventory)])
}

#[test]
fn catalog_links_prefer_bindings_then_provider_identities() {
    let snapshot = snapshot(vec![("radarr", radarr()), ("sonarr", sonarr())]);
    let link = |kind: &str| Link {
        id: "item".into(),
        kind: kind.into(),
        ..Link::default()
    };
    let bound = Link {
        service_id: Some("radarr-1".into()),
        entity_id: Some(4),
        external_id: Some("104".into()),
        tmdb_id: Some(101),
        ..link("movie")
    };
    assert_eq!(find(&snapshot, "radarr", &bound).unwrap().title.id, 4);
    // A binding whose manager identity changed falls back to the provider identity.
    let stale = Link {
        external_id: Some("999".into()),
        ..bound.clone()
    };
    assert_eq!(find(&snapshot, "radarr", &stale).unwrap().title.id, 1);
    assert!(find(&snapshot, "radarr", &link("movie")).is_none());
    let season = Link {
        tvdb_id: Some(201),
        season: Some(1),
        ..link("season")
    };
    assert_eq!(
        link_status(&snapshot, &season, &[], NOW).unwrap().state,
        Phase::Downloading
    );
    // Episodes use a confirmed mapping first, else their season and episode numbers.
    let mapped = Link {
        tmdb_id: Some(301),
        season: Some(1),
        episode: Some(3),
        mapped: vec![("sonarr-1".into(), 1002)],
        ..link("episode")
    };
    assert_eq!(
        link_status(&snapshot, &mapped, &[], NOW).unwrap().state,
        Phase::Missing
    );
    let numbered = Link {
        mapped: vec![("other".into(), 1002)],
        ..mapped.clone()
    };
    assert_eq!(
        link_status(&snapshot, &numbered, &[], NOW).unwrap().state,
        Phase::Downloading
    );
    let unknown = Link {
        mapped: Vec::new(),
        episode: Some(40),
        ..mapped
    };
    assert!(link_status(&snapshot, &unknown, &[], NOW).is_none());
    assert!(link_status(&snapshot, &link("track"), &[], NOW).is_none());
}

#[test]
fn seasons_merge_library_and_sonarr_episodes_with_specials_last() {
    let snapshot = snapshot(vec![("sonarr", sonarr())]);
    let episode = |id: &str, number: i64, available: bool, watched: bool| LibraryEpisode {
        id: id.into(),
        number: Some(number),
        title: format!("Library {id}"),
        available,
        watched,
        ..LibraryEpisode::default()
    };
    let library = vec![
        LibrarySeason {
            id: "specials".into(),
            number: Some(0),
            title: "Specials".into(),
            episodes: vec![episode("sp1", 1, false, false)],
        },
        LibrarySeason {
            id: "season1".into(),
            number: Some(1),
            title: "Season 1".into(),
            episodes: vec![
                LibraryEpisode {
                    mapped: vec![("sonarr-1".into(), 1001)],
                    position: Some(12.5),
                    duration: Some(2700.0),
                    ..episode("e1", 1, true, true)
                },
                episode("e2", 2, true, true),
                episode("e9", 9, true, false),
            ],
        },
        LibrarySeason {
            id: "season5".into(),
            number: Some(5),
            title: "Season 5".into(),
            episodes: Vec::new(),
        },
    ];
    let value = seasons_value(Some(found(&snapshot, "sonarr", 10)), &library, true, NOW);
    let seasons = value["seasons"].as_array().unwrap();
    assert_eq!(
        seasons
            .iter()
            .map(|s| s["number"].clone())
            .collect::<Vec<_>>(),
        [json!(1), json!(2), json!(0)]
    );
    let first = &seasons[0];
    assert_eq!(first["id"], "season1");
    assert_eq!(first["title"], "Season 1");
    assert_eq!(first["watched"], false);
    // e2 is playable here though Sonarr has no file; the third episode is downloading.
    assert_eq!(first["status"]["state"], "downloading");
    assert_eq!(
        first["status"]["episodes"],
        json!({"available":2,"total":3})
    );
    let episodes = first["episodes"].as_array().unwrap();
    assert_eq!(
        episodes.iter().map(|e| e["id"].clone()).collect::<Vec<_>>(),
        [json!("e1"), json!("e2"), Value::Null, json!("e9")]
    );
    assert_eq!(episodes[0]["title"], "Pilot");
    assert_eq!(episodes[0]["runtime"], 45);
    assert_eq!(episodes[0]["position"], 12.5);
    assert_eq!(episodes[0]["duration"], 2700.0);
    assert_eq!(episodes[0]["air_date"], 1_577_836_800);
    assert_eq!(episodes[1]["status"]["state"], "available");
    assert_eq!(episodes[2]["title"], "Third");
    assert_eq!(episodes[2]["available"], false);
    assert_eq!(episodes[2]["position"], Value::Null);
    assert_eq!(episodes[2]["status"]["state"], "downloading");
    // A library-only episode has no Sonarr status.
    assert_eq!(episodes[3]["title"], "Library e9");
    assert_eq!(episodes[3]["status"], Value::Null);
    let second = &seasons[1];
    assert_eq!(second["id"], Value::Null);
    assert_eq!(second["title"], "Season 2");
    assert_eq!(second["watched"], false);
    assert_eq!(second["episodes"].as_array().unwrap().len(), 2);
    let specials = &seasons[2];
    assert_eq!(specials["title"], "Specials");
    assert_eq!(specials["episodes"][0]["id"], "sp1");
    assert_eq!(specials["episodes"][0]["status"]["state"], "unmonitored");
    // Users get the collapsed states, and a library without Sonarr has no status.
    let user = seasons_value(Some(found(&snapshot, "sonarr", 10)), &library, false, NOW);
    assert_eq!(
        user["seasons"][2]["episodes"][0]["status"],
        json!({"state":"unavailable"})
    );
    assert!(user["seasons"][0]["status"].get("monitored").is_none());
    let local = seasons_value(None, &library, false, NOW);
    assert_eq!(local["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(local["seasons"][0]["status"], Value::Null);
    assert_eq!(local["seasons"][1]["title"], "Specials");
}

#[test]
fn manager_titles_without_catalog_items_become_virtual_items() {
    let snapshot = snapshot(vec![("radarr", radarr()), ("sonarr", sonarr())]);
    let linked = Linked {
        entities: HashSet::from([("radarr-1".to_owned(), 1)]),
        tmdb: HashSet::from([102]),
        tvdb: HashSet::new(),
    };
    // Wanted or not, every title with an identity is listed.
    let movies = virtual_items(&snapshot, "movie", &linked, false, NOW, "g");
    assert_eq!(
        movies
            .iter()
            .map(|(m, _)| m["id"].clone())
            .collect::<Vec<_>>(),
        [
            json!("tmdb:103"),
            json!("tmdb:104"),
            json!("tmdb:105"),
            json!("tmdb:106")
        ]
    );
    assert_eq!(
        movies[1],
        (
            json!({"id":"tmdb:104","kind":"movie","title":"Wanted","year":null,"available":false,"virtual":true,"artwork_url":null,"status":{"state":"unavailable"},"watched":false}),
            Facts {
                added: Some(1_577_836_800),
                rating: Some(6.5),
                genres: vec!["Drama".into()],
                released: Some(1_577_880_000),
                ..Facts::default()
            }
        )
    );
    let admin = virtual_items(&snapshot, "movie", &linked, true, NOW, "g");
    assert_eq!(admin[2].0["status"]["state"], "unmonitored");
    assert_eq!(admin[3].0["status"]["state"], "queued");
    assert_eq!(admin[3].0["status"]["monitored"], true);
    // A title whose file the library has not found yet is listed too.
    let unscanned = virtual_items(&snapshot, "movie", &Linked::default(), true, NOW, "g");
    assert_eq!(unscanned[0].0["id"], "tmdb:101");
    assert_eq!(unscanned[0].0["status"]["state"], "available");
    let shows = virtual_items(&snapshot, "show", &Linked::default(), true, NOW, "g");
    assert_eq!(
        shows
            .iter()
            .map(|(s, _)| (
                s["id"].clone(),
                s["ended"].clone(),
                s["status"]["state"].clone()
            ))
            .collect::<Vec<_>>(),
        [
            (json!("tmdb:301"), json!(false), json!("downloading")),
            (json!("tvdb:202"), json!(true), json!("unmonitored"))
        ]
    );
    assert_eq!(shows[0].1.rating, Some(8.0));
    let tvdb_only = inventory(
        "sonarr",
        json!([{"id":12,"tvdbId":203,"title":"Only TVDB","monitored":true,"images":[{"coverType":"poster","url":"/MediaCover/12/poster.jpg"}]}]),
        json!({"12":[{"id":1,"seasonNumber":1,"episodeNumber":1,"airDateUtc":FUTURE,"monitored":true}]}),
        json!([]),
    );
    let tvdb_only = snapshot_of(tvdb_only);
    let shows = virtual_items(&tvdb_only, "show", &Linked::default(), false, NOW, "g");
    assert_eq!(shows[0].0["id"], "tvdb:203");
    assert_eq!(
        shows[0].0["artwork_url"],
        "/api/v1/catalog/titles/sonarr-1/12/artwork/poster?grant=g"
    );
    assert_eq!(shows[0].0["status"]["state"], "upcoming");
    let linked = Linked {
        tvdb: HashSet::from([203]),
        ..Linked::default()
    };
    assert!(virtual_items(&tvdb_only, "show", &linked, false, NOW, "g").is_empty());
}

#[test]
fn series_end_when_sonarr_or_tmdb_says_so() {
    for status in ["ended", "Ended", "Canceled", "cancelled"] {
        assert!(ended(Some(status)), "{status}");
    }
    for status in [
        Some("continuing"),
        Some("Returning Series"),
        Some("upcoming"),
        None,
    ] {
        assert!(!ended(status), "{status:?}");
    }
}

#[test]
fn references_and_fingerprints() {
    assert_eq!(Ref::parse("tmdb:603"), Some(Ref::Tmdb(603)));
    assert_eq!(Ref::parse("tvdb:7"), Some(Ref::Tvdb(7)));
    for invalid in [
        "tmdb:0",
        "tmdb:",
        "imdb:1",
        "tmdb:-1",
        "tmdb:1a",
        "603",
        "tmdb:1234567890123",
    ] {
        assert_eq!(Ref::parse(invalid), None, "{invalid}");
    }
    let one = snapshot_of(sonarr());
    let mut changed = sonarr();
    changed.queue[0].sizeleft = 100;
    // Progress within the same five-percent step announces nothing.
    let mut nudged = sonarr();
    nudged.queue[0].sizeleft = 249;
    assert_eq!(one.fingerprint(), snapshot_of(nudged).fingerprint());
    assert_eq!(one.fingerprint(), snapshot_of(sonarr()).fingerprint());
    assert_ne!(one.fingerprint(), snapshot_of(changed).fingerprint());
    assert_ne!(one.fingerprint(), Snapshot::default().fingerprint());
}

#[tokio::test]
async fn polling_reads_queue_pages_and_only_changed_series_episodes() {
    let (_temp, state, _) = fixture().await;
    let titles = Arc::new(AtomicUsize::new(0));
    let episodes = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let queued = Arc::new(AtomicUsize::new(260));
    let series = Arc::new(std::sync::Mutex::new(json!([
        {"id":10,"tvdbId":201,"title":"One","monitored":true,"statistics":{"episodeCount":1,"episodeFileCount":0,"totalEpisodeCount":1}},
        {"id":11,"tvdbId":202,"title":"Two","monitored":true,"statistics":{"episodeCount":1,"episodeFileCount":0,"totalEpisodeCount":1}}
    ])));
    let (title_calls, episode_calls, queue_size, series_list) = (
        titles.clone(),
        episodes.clone(),
        queued.clone(),
        series.clone(),
    );
    let stub = Router::new()
        .route(
            "/api/v3/queue",
            get(
                move |axum::extract::Query(query): axum::extract::Query<
                    HashMap<String, String>,
                >| {
                    let total = queue_size.load(Ordering::SeqCst);
                    async move {
                        assert_eq!(query["includeUnknownSeriesItems"], "false");
                        let page = query["page"].parse::<usize>().unwrap();
                        let size = query["pageSize"].parse::<usize>().unwrap();
                        let records = ((page - 1) * size..(page * size).min(total))
                            .map(|id| json!({"id":id,"seriesId":10,"episodeId":1,"status":"queued","size":1,"sizeleft":1}))
                            .collect::<Vec<_>>();
                        Json(json!({"page":page,"pageSize":size,"totalRecords":total,"records":records}))
                    }
                },
            ),
        )
        .route(
            "/api/v3/series",
            get(move || {
                title_calls.fetch_add(1, Ordering::SeqCst);
                let list = series_list.lock().unwrap().clone();
                async move { Json(list) }
            }),
        )
        .route(
            "/api/v3/episode",
            get(
                move |axum::extract::Query(query): axum::extract::Query<
                    HashMap<String, String>,
                >| {
                    let id = query["seriesId"].clone();
                    episode_calls.lock().unwrap().push(id.clone());
                    async move {
                        Json(json!([{"id":id.parse::<i64>().unwrap()*100,"seasonNumber":1,"episodeNumber":1,"airDateUtc":PAST,"monitored":true}]))
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let c = Connection::validation(
        &state,
        format!("http://{address}"),
        String::new(),
        "fixture".into(),
        "sonarr".into(),
    );
    let mut tracker = Tracker::default();
    let first = poll(&c, None, &mut tracker, false).await.unwrap();
    assert_eq!(first.queue.len(), 260);
    assert_eq!(first.titles.len(), 2);
    assert_eq!(first.episodes[&11][0].id, 1100);
    assert_eq!(titles.load(Ordering::SeqCst), 1);
    assert_eq!(*episodes.lock().unwrap(), ["10", "11"]);
    // An unchanged queue reads nothing else until titles are due.
    let second = poll(&c, Some(&first), &mut tracker, false).await.unwrap();
    assert_eq!(titles.load(Ordering::SeqCst), 1);
    assert_eq!(second.titles.len(), 2);
    // A nudge rereads titles, but only the series whose signature changed.
    series.lock().unwrap()[1]["statistics"]["episodeFileCount"] = json!(1);
    let third = poll(&c, Some(&second), &mut tracker, true).await.unwrap();
    assert_eq!(titles.load(Ordering::SeqCst), 2);
    assert_eq!(*episodes.lock().unwrap(), ["10", "11", "11"]);
    // A queue change suggests an import, so titles are reread without a nudge.
    queued.store(3, Ordering::SeqCst);
    let fourth = poll(&c, Some(&third), &mut tracker, false).await.unwrap();
    assert_eq!(fourth.queue.len(), 3);
    assert_eq!(titles.load(Ordering::SeqCst), 3);
    assert_eq!(episodes.lock().unwrap().len(), 3);
    server.abort();
    // A failing manager is an error the worker answers by keeping the last snapshot.
    let unreachable = Connection::validation(
        &state,
        format!("http://{address}"),
        String::new(),
        "fixture".into(),
        "sonarr".into(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let before = tracker.queue.clone();
    assert!(
        poll(&unreachable, Some(&fourth), &mut tracker, true)
            .await
            .is_err()
    );
    assert_eq!(tracker.queue, before);
}

async fn api_fixture() -> (tempfile::TempDir, AppState, String, String) {
    let (temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute_batch("UPDATE users SET role='admin' WHERE id='bob';
INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies','/media/movies'),('shows','Shows','shows','/media/shows');
INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at,operational) VALUES ('radarr-1','Radarr','radarr','c1',7878,'g',X'00','/media','1',1,1),('sonarr-1','Sonarr','sonarr','c2',8989,'g',X'00','/media','1',1,1);
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,metadata,created_at) VALUES
 ('movie-bound','movies','movie',NULL,'a','beta file',NULL,'{\"title\":\"Beta\",\"tmdb_id\":101}',1),
 ('movie-matched','movies','movie',NULL,'b','delta',NULL,'{\"title\":\"delta\",\"tmdb_id\":104}',1),
 ('movie-plain','movies','movie',NULL,'c','Epsilon',NULL,'{}',1),
 ('show','shows','show',NULL,'s','Show',NULL,'{\"name\":\"Show\",\"tvdb_id\":201,\"tmdb_id\":301}',1),
 ('season1','shows','season','show','s1','Season 1',1,'{}',1),
 ('specials','shows','season','show','s0','Specials',0,'{}',1),
 ('e1','shows','episode','season1','e1','Show · S01E01',1,'{}',1),
 ('e2','shows','episode','season1','e2','Show · S01E02',2,'{}',1),
 ('sp1','shows','episode','specials','sp1','Show · S00E01',1,'{}',1);
INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('f1','shows','/media/shows/e1.mkv','g',1,'m','f1','{}',1),('f2','movies','/media/movies/delta.mkv','g',1,'m','f2','{}',1);
INSERT INTO media_sources VALUES ('e1','f1'),('movie-matched','f2');
INSERT INTO metadata_bindings(media_id,service_id,service_generation,external_id,manager_entity_id) VALUES ('movie-bound','radarr-1','g','101',1),('show','sonarr-1','g','201',10);
INSERT INTO manager_episodes VALUES ('sonarr-1','g','201',1001,1,1,1,'{}');
INSERT INTO manager_episode_mappings VALUES ('e1','sonarr-1','g',1001,'confirmed');
INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','e1',1,1);
INSERT INTO edition_progress VALUES ('alice','e2','',30,60,1);")?;
        Ok(())
    }).await.unwrap();
    let bob =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    // The movie bound as Radarr entity 1 is missing upstream; the matched one is playable.
    let movies = inventory(
        "radarr",
        json!([
            {"id":1,"tmdbId":101,"title":"Beta","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":2,"tmdbId":102,"title":"alpha wanted","year":2024,"monitored":true,"isAvailable":true,
             "images":[{"coverType":"poster","url":"/MediaCover/2/poster.jpg"}]},
            {"id":3,"tmdbId":103,"title":"Gamma","monitored":true,"isAvailable":false,"digitalRelease":FUTURE},
            {"id":4,"tmdbId":104,"title":"delta","monitored":true,"hasFile":false,"isAvailable":true},
            {"id":5,"tmdbId":105,"title":"Zeta","monitored":false,"isAvailable":true},
            {"id":6,"tmdbId":106,"title":"Omega","monitored":true,"hasFile":true},
            {"id":7,"tmdbId":107,"title":"Queued one","monitored":true,"isAvailable":true}
        ]),
        json!({}),
        json!([{"id":1,"movieId":7,"status":"queued","trackedDownloadState":"downloading","size":10,"sizeleft":10}]),
    );
    let mut shows = sonarr();
    shows.queue.clear();
    replace(
        &state,
        snapshot(vec![("radarr", movies), ("sonarr", shows)]),
    );
    (temp, state, alice, format!("thelxinoe_session={bob}"))
}

#[tokio::test]
async fn top_level_lists_filter_and_sort_before_they_are_cut() {
    let (_temp, state, alice, _) = api_fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute_batch("UPDATE media SET metadata=json_set(metadata,'$.vote_average',7.5,'$.genres',json('[{\"name\":\"Drama\"}]')) WHERE id='movie-bound';
UPDATE media SET metadata=json_set(metadata,'$.status','ended') WHERE id='show';
INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','movie-matched',1,1);
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES ('movie-started','movies','movie',NULL,'d','Started',2);
INSERT INTO edition_progress VALUES ('alice','movie-started','',30,60,1);")?;
        Ok(())
    }).await.unwrap();
    let movies = inventory(
        "radarr",
        json!([
            {"id":2,"tmdbId":102,"title":"alpha wanted","monitored":true,"isAvailable":true,"added":"2024-01-01T00:00:00Z"},
            {"id":3,"tmdbId":103,"title":"Gamma","monitored":true,"isAvailable":false,"digitalRelease":FUTURE,
             "added":"2025-01-01T00:00:00Z","genres":["Drama"]},
            {"id":7,"tmdbId":107,"title":"Queued one","monitored":true,"isAvailable":true,"ratings":{"tmdb":{"value":8.0}}}
        ]),
        json!({}),
        json!([{"id":1,"movieId":7,"status":"queued","trackedDownloadState":"downloading","size":10,"sizeleft":10}]),
    );
    replace(
        &state,
        snapshot(vec![("radarr", movies), ("sonarr", sonarr())]),
    );
    let titles = |query: &'static str| {
        let (state, alice) = (state.clone(), alice.clone());
        async move {
            let (status, _, body) = call(
                &state,
                &format!("/api/v1/catalog?{query}"),
                "GET",
                Value::Null,
                &alice,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{query}: {body}");
            let titles = body["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["title"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            (titles, body)
        }
    };
    let (all, body) = titles("kind=movie").await;
    assert_eq!(
        all,
        [
            "alpha wanted",
            "Beta",
            "delta",
            "Epsilon",
            "Gamma",
            "Queued one",
            "Started"
        ]
    );
    assert_eq!(body["total"], 7);
    assert_eq!(body["genres"], json!(["Drama"]));
    assert_eq!(
        body["counts"],
        json!({"watch":{"in_progress":1,"unwatched":5,"watched":1},
            "availability":{"downloaded":1,"downloading":1,"not_downloaded":4,"upcoming":1}})
    );
    for (query, expected) in [
        ("kind=movie&watch=watched", vec!["delta"]),
        (
            "kind=movie&watch=in_progress,watched",
            vec!["delta", "Started"],
        ),
        ("kind=movie&availability=downloaded", vec!["delta"]),
        (
            "kind=movie&availability=downloading,upcoming",
            vec!["Gamma", "Queued one"],
        ),
        (
            "kind=movie&availability=not_downloaded&q=E",
            vec!["alpha wanted", "Beta", "Epsilon", "Started"],
        ),
        ("kind=movie&genre=Drama", vec!["Beta", "Gamma"]),
        (
            "kind=movie&sort=title&order=desc",
            vec![
                "Started",
                "Queued one",
                "Gamma",
                "Epsilon",
                "delta",
                "Beta",
                "alpha wanted",
            ],
        ),
        // Titles without the value come last either way; ties keep the title order.
        (
            "kind=movie&sort=added&order=desc",
            vec![
                "Gamma",
                "alpha wanted",
                "Started",
                "Beta",
                "delta",
                "Epsilon",
                "Queued one",
            ],
        ),
        (
            "kind=movie&sort=added",
            vec![
                "Beta",
                "delta",
                "Epsilon",
                "Started",
                "alpha wanted",
                "Gamma",
                "Queued one",
            ],
        ),
        (
            "kind=movie&sort=rating&order=desc",
            vec![
                "Queued one",
                "Beta",
                "alpha wanted",
                "delta",
                "Epsilon",
                "Gamma",
                "Started",
            ],
        ),
    ] {
        assert_eq!(titles(query).await.0, expected, "{query}");
    }
    // Counts follow the search, not the other filters.
    let (_, body) = titles("kind=movie&q=E&watch=watched").await;
    assert_eq!(
        body["counts"]["watch"],
        json!({"in_progress":1,"unwatched":4,"watched":1})
    );
    let (shows, body) = titles("kind=show").await;
    assert_eq!(shows, ["Show", "Unwatched series"]);
    assert_eq!(body["items"][0]["ended"], true);
    assert_eq!(body["counts"]["series"], json!({"continuing":0,"ended":2}));
    assert_eq!(body["counts"]["watch"]["in_progress"], 1);
    assert!(titles("kind=show&series=continuing").await.0.is_empty());
    assert_eq!(
        titles("kind=show&series=ended&watch=in_progress").await.0,
        ["Show"]
    );
    for invalid in [
        "kind=movie&watch=seen",
        "kind=movie&availability=",
        "kind=movie&sort=length",
        "kind=show&series=airing",
    ] {
        let (status, ..) = call(
            &state,
            &format!("/api/v1/catalog?{invalid}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    }
    let detail = call(&state, "/api/v1/catalog/show", "GET", Value::Null, &alice).await;
    assert_eq!(detail.2["ended"], true);
    // Discover marks what the library lists: its items and the manager-only titles.
    let shows = library_ids(&state, "show").await.unwrap();
    assert!([201, 202].iter().all(|id| shows.tvdb.contains(id)));
    assert!(shows.tmdb.contains(&301));
    let movies = library_ids(&state, "movie").await.unwrap();
    assert!(
        [101, 102, 104, 107]
            .iter()
            .all(|id| movies.tmdb.contains(id))
    );
}

#[tokio::test]
async fn top_level_lists_sort_by_files_releases_and_airings() {
    let (_temp, state, alice, _) = api_fixture().await;
    // A missing file weighs nothing, and a file holding two episodes counts once.
    state.db.write("test.fixture", |db| {
        db.execute_batch("UPDATE media SET metadata=json_set(metadata,'$.runtime',120) WHERE id='movie-bound';
UPDATE media SET metadata=json_set(metadata,'$.release_date','2022-03-04T00:00:00Z') WHERE id='movie-matched';
UPDATE media_files SET size=5000 WHERE id='f2';
INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at,present) VALUES
 ('f3','movies','/media/movies/epsilon.mkv','g',9000,'m','f3','{}',1,0),
 ('f4','shows','/media/shows/e1-e2.mkv','g',30,'m','f4','{}',1,1);
INSERT INTO media_sources VALUES ('movie-plain','f3'),('e1','f4'),('e2','f4');")?;
        Ok(())
    }).await.unwrap();
    let movies = inventory(
        "radarr",
        json!([
            {"id":2,"tmdbId":102,"title":"alpha wanted","monitored":true,"isAvailable":true,"sizeOnDisk":1000,"inCinemas":PAST},
            {"id":3,"tmdbId":103,"title":"Gamma","monitored":true,"isAvailable":false,"digitalRelease":FUTURE,
             "runtime":95,"statistics":{"sizeOnDisk":3000}},
            {"id":7,"tmdbId":107,"title":"Queued one","monitored":true,"isAvailable":true,"runtime":0}
        ]),
        json!({}),
        json!([]),
    );
    let mut shows = sonarr();
    let titles = Arc::make_mut(&mut shows.titles);
    let series = titles.get_mut(&10).unwrap();
    series.previous_airing = Some("2025-01-01T00:00:00Z".into());
    series.next_airing = Some(FUTURE.into());
    let unwatched = titles.get_mut(&11).unwrap();
    unwatched.previous_airing = Some(PAST.into());
    unwatched.statistics = Some(Statistics {
        episode_file_count: 3,
        size_on_disk: 7000,
        ..Statistics::default()
    });
    replace(
        &state,
        snapshot(vec![("radarr", movies), ("sonarr", shows)]),
    );
    let list = |query: &'static str| {
        let (state, alice) = (state.clone(), alice.clone());
        async move {
            let (status, _, body) = call(
                &state,
                &format!("/api/v1/catalog?{query}"),
                "GET",
                Value::Null,
                &alice,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{query}: {body}");
            body["items"].as_array().unwrap().clone()
        }
    };
    for (query, expected) in [
        (
            "kind=movie&sort=size&order=desc",
            vec![
                "delta",
                "Gamma",
                "alpha wanted",
                "Beta",
                "Epsilon",
                "Queued one",
            ],
        ),
        (
            "kind=movie&sort=size",
            vec![
                "alpha wanted",
                "Gamma",
                "delta",
                "Beta",
                "Epsilon",
                "Queued one",
            ],
        ),
        (
            "kind=movie&sort=runtime&order=desc",
            vec![
                "Beta",
                "Gamma",
                "alpha wanted",
                "delta",
                "Epsilon",
                "Queued one",
            ],
        ),
        (
            "kind=movie&sort=released",
            vec![
                "alpha wanted",
                "delta",
                "Gamma",
                "Beta",
                "Epsilon",
                "Queued one",
            ],
        ),
        (
            "kind=show&sort=episodes&order=desc",
            vec!["Unwatched series", "Show"],
        ),
        (
            "kind=show&sort=size&order=desc",
            vec!["Unwatched series", "Show"],
        ),
        (
            "kind=show&sort=last_aired",
            vec!["Unwatched series", "Show"],
        ),
        (
            "kind=show&sort=next_airing&order=desc",
            vec!["Show", "Unwatched series"],
        ),
    ] {
        let titles = list(query)
            .await
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(titles, expected, "{query}");
    }
    // Cards carry what they can be sorted by, so they show the value they are.
    let movies = list("kind=movie&sort=size&order=desc").await;
    assert_eq!(movies[0]["size"], 5000);
    assert_eq!(movies[0]["released"], 1_646_395_200);
    assert_eq!(movies[0]["added"], 1);
    assert!(movies[0].get("runtime").is_none());
    assert_eq!(movies[1]["released"], FUTURE_DAY);
    let shows = list("kind=show").await;
    assert_eq!(
        [
            &shows[0]["size"],
            &shows[0]["episodes"],
            &shows[0]["last_aired"],
            &shows[0]["next_airing"]
        ],
        [
            &json!(31),
            &json!(2),
            &json!(1_735_689_600),
            &json!(FUTURE_AT)
        ]
    );
    assert_eq!(shows[1]["size"], 7000);
    assert_eq!(shows[1]["episodes"], 3);
}

#[tokio::test]
async fn browse_detail_seasons_and_lookup_expose_cached_status() {
    let (_temp, state, alice, bob) = api_fixture().await;
    let get = |path: String, cookie: String| {
        let state = state.clone();
        async move { call(&state, &path, "GET", Value::Null, &cookie).await }
    };
    let movies = get("/api/v1/catalog?kind=movie".into(), alice.clone()).await;
    assert_eq!(movies.0, StatusCode::OK, "{}", movies.2);
    let items = movies.2["items"].as_array().unwrap();
    assert_eq!(
        items.iter().map(|i| i["title"].clone()).collect::<Vec<_>>(),
        [
            "alpha wanted",
            "Beta",
            "delta",
            "Epsilon",
            "Gamma",
            "Omega",
            "Queued one",
            "Zeta"
        ]
    );
    assert_eq!(items[0]["id"], "tmdb:102");
    assert_eq!(items[0]["virtual"], true);
    assert_eq!(items[0]["year"], 2024);
    assert!(
        items[0]["artwork_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/catalog/titles/radarr-1/2/artwork/poster?grant=")
    );
    assert_eq!(items[0]["status"], json!({"state":"unavailable"}));
    assert_eq!(items[1]["id"], "movie-bound");
    assert!(items[1].get("virtual").is_none());
    assert_eq!(items[1]["status"], json!({"state":"unavailable"}));
    assert_eq!(items[2]["status"], json!({"state":"available"}));
    assert_eq!(items[3]["status"], Value::Null);
    assert_eq!(items[4]["status"]["state"], "upcoming");
    assert_eq!(items[5]["status"]["state"], "available");
    assert_eq!(items[6]["status"]["state"], "queued");
    assert_eq!(items[7]["status"], json!({"state":"unavailable"}));
    let admin = get("/api/v1/catalog?kind=movie&q=A".into(), bob.clone()).await;
    let items = admin.2["items"].as_array().unwrap();
    assert_eq!(
        items.iter().map(|i| i["title"].clone()).collect::<Vec<_>>(),
        ["alpha wanted", "Beta", "delta", "Gamma", "Omega", "Zeta"]
    );
    assert_eq!(
        items[1]["status"],
        json!({"state":"missing","monitored":true})
    );
    // Lists below the top level never mix in manager-only titles.
    let episodes = get("/api/v1/catalog?parent=season1".into(), alice.clone()).await;
    assert_eq!(episodes.2["items"].as_array().unwrap().len(), 2);
    assert_eq!(episodes.2["items"][0]["status"]["state"], "available");
    assert_eq!(
        episodes.2["items"][1]["status"],
        json!({"state":"unavailable"})
    );
    let shows = get("/api/v1/catalog?kind=show".into(), bob.clone()).await;
    let items = shows.2["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["id"], "tvdb:202");
    assert_eq!(items[1]["status"]["state"], "unmonitored");
    assert_eq!(items[0]["status"]["state"], "partial");
    assert_eq!(
        items[0]["status"]["episodes"],
        json!({"available":1,"total":3})
    );
    // Cards carry their series' landscape art; titles without a manager have none.
    assert!(
        items[0]["backdrop_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant=")
    );
    let unlinked = get("/api/v1/catalog?kind=movie&q=Epsilon".into(), bob.clone()).await;
    assert_eq!(unlinked.2["items"][0]["title"], "Epsilon");
    assert_eq!(unlinked.2["items"][0]["backdrop_url"], Value::Null);
    // Home shelves are landscape, so their cards get the same backdrops.
    let saved = call(
        &state,
        "/api/v1/catalog/e2/state",
        "PUT",
        json!({"watch_later":true}),
        &alice,
    )
    .await;
    assert_eq!(saved.0, StatusCode::OK, "{}", saved.2);
    let home = get("/api/v1/me/home".into(), alice.clone()).await;
    assert_eq!(home.0, StatusCode::OK, "{}", home.2);
    // Saved episodes and seasons name their series.
    assert!(home.2["watch_later"][0]["show_title"].is_string());
    assert!(
        home.2["watch_later"][0]["backdrop_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant=")
    );

    // Detail adds artwork, backdrop and status; only administrators see Keep.
    let detail = get("/api/v1/catalog/show".into(), alice.clone()).await;
    assert_eq!(detail.0, StatusCode::OK);
    assert!(detail.2.get("keep").is_none());
    assert!(detail.2.get("overrides").is_none());
    assert_eq!(detail.2["artwork_url"], Value::Null);
    assert_eq!(detail.2["status"]["state"], "partial");
    let backdrop = detail.2["backdrop_url"].as_str().unwrap().to_owned();
    assert!(backdrop.starts_with("/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant="));
    let episode = get("/api/v1/catalog/e2".into(), alice.clone()).await;
    assert_eq!(episode.2["status"], json!({"state":"unavailable"}));
    // Seasons and episodes show their series' backdrop.
    assert!(
        episode.2["backdrop_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant=")
    );
    let plain = get("/api/v1/catalog/movie-plain".into(), bob.clone()).await;
    assert_eq!(plain.2["status"], Value::Null);
    assert_eq!(plain.2["backdrop_url"], Value::Null);
    assert_eq!(plain.2["keep"], false);
    assert_eq!(plain.2["kept_by"], Value::Null);
    assert_eq!(plain.2["kept_within"], 0);
    // Keep on a series covers its seasons and episodes, and blocks the series' deletion
    // when only an episode is kept.
    state
        .db
        .write("test.keep", |db| {
            db.execute(
                "INSERT INTO media_protection VALUES ('show',1),('e2',1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let episode = get("/api/v1/catalog/e1".into(), bob.clone()).await;
    assert_eq!(episode.2["keep"], false);
    assert_eq!(episode.2["kept_by"], "show");
    let season = get("/api/v1/catalog/season1".into(), bob.clone()).await;
    assert_eq!(season.2["kept_by"], "show");
    assert_eq!(season.2["kept_within"], 1);
    let show = get("/api/v1/catalog/show".into(), bob.clone()).await;
    assert_eq!(show.2["keep"], true);
    assert_eq!(show.2["kept_by"], Value::Null);
    assert_eq!(show.2["kept_within"], 1);
    assert!(
        get("/api/v1/catalog/e1".into(), alice.clone())
            .await
            .2
            .get("kept_by")
            .is_none()
    );
    state
        .db
        .write("test.keep", |db| {
            db.execute("DELETE FROM media_protection", [])?;
            Ok(())
        })
        .await
        .unwrap();
    state
        .db
        .write("test.fixture", |db| {
            db.execute("INSERT INTO media_protection VALUES ('movie-plain',1)", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        get("/api/v1/catalog/movie-plain".into(), bob.clone())
            .await
            .2["keep"],
        true
    );
    assert!(
        get("/api/v1/catalog/movie-plain".into(), alice.clone())
            .await
            .2
            .get("keep")
            .is_none()
    );

    // Seasons merge the library with Sonarr, for the caller's own progress.
    let seasons = get("/api/v1/catalog/show/seasons".into(), alice.clone()).await;
    assert_eq!(seasons.0, StatusCode::OK, "{}", seasons.2);
    let list = seasons.2["seasons"].as_array().unwrap();
    assert_eq!(
        list.iter().map(|s| s["number"].clone()).collect::<Vec<_>>(),
        [json!(1), json!(2), json!(0)]
    );
    assert_eq!(list[0]["id"], "season1");
    assert_eq!(list[0]["watched"], false);
    let first = &list[0]["episodes"];
    assert_eq!(first[0]["id"], "e1");
    assert_eq!(first[0]["title"], "Pilot");
    assert_eq!(first[0]["watched"], true);
    assert_eq!(first[1]["position"], 30.0);
    assert_eq!(first[1]["duration"], 60.0);
    assert_eq!(first[1]["status"], json!({"state":"unavailable"}));
    assert_eq!(first[2]["id"], Value::Null);
    assert_eq!(list[2]["id"], "specials");
    let bob_seasons = get("/api/v1/catalog/show/seasons".into(), bob.clone()).await;
    assert_eq!(bob_seasons.2["seasons"][0]["episodes"][0]["watched"], false);
    assert_eq!(bob_seasons.2["seasons"][0]["status"]["monitored"], true);
    let by_reference = get("/api/v1/catalog/tvdb:201/seasons".into(), alice.clone()).await;
    assert_eq!(by_reference.2, seasons.2);
    let sonarr_only = get("/api/v1/catalog/tvdb:202/seasons".into(), alice.clone()).await;
    assert_eq!(sonarr_only.0, StatusCode::OK);
    assert_eq!(
        sonarr_only.2["seasons"][0]["episodes"][0]["id"],
        Value::Null
    );
    for missing in ["tvdb:999", "movie-plain", "nothing"] {
        assert_eq!(
            get(format!("/api/v1/catalog/{missing}/seasons"), alice.clone())
                .await
                .0,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }

    // Lookup resolves provider references to the library and manager titles.
    let found = get(
        "/api/v1/catalog/lookup?kind=movie&ref=tmdb:101".into(),
        alice.clone(),
    )
    .await;
    assert_eq!(found.0, StatusCode::OK, "{}", found.2);
    assert_eq!(found.2["media_id"], "movie-bound");
    assert_eq!(found.2["title"]["title"], "Beta");
    assert_eq!(found.2["title"]["tvdb_id"], Value::Null);
    assert_eq!(found.2["status"], json!({"state":"unavailable"}));
    let wanted = get(
        "/api/v1/catalog/lookup?kind=movie&ref=tmdb:102".into(),
        bob.clone(),
    )
    .await;
    assert_eq!(wanted.2["media_id"], Value::Null);
    assert_eq!(wanted.2["title"]["year"], 2024);
    assert!(wanted.2["title"]["poster_url"].is_string());
    assert_eq!(wanted.2["title"]["backdrop_url"], Value::Null);
    assert_eq!(
        wanted.2["status"],
        json!({"state":"missing","monitored":true})
    );
    let show = get(
        "/api/v1/catalog/lookup?kind=show&ref=tvdb:201".into(),
        alice.clone(),
    )
    .await;
    assert_eq!(show.2["media_id"], "show");
    assert_eq!(show.2["title"]["tvdb_id"], 201);
    assert_eq!(show.2["title"]["tmdb_id"], 301);
    let unknown = get(
        "/api/v1/catalog/lookup?kind=show&ref=tmdb:1".into(),
        alice.clone(),
    )
    .await;
    assert_eq!(unknown.0, StatusCode::OK);
    assert_eq!(
        unknown.2,
        json!({"media_id":null,"title":null,"status":null})
    );
    for invalid in [
        "kind=movie&ref=tvdb:201",
        "kind=album&ref=tmdb:1",
        "kind=movie&ref=tmdb:x",
    ] {
        assert_eq!(
            get(format!("/api/v1/catalog/lookup?{invalid}"), alice.clone())
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{invalid}"
        );
    }
}

#[tokio::test]
async fn manager_artwork_is_proxied_from_the_cache_with_a_grant() {
    let (_temp, state, alice, _) = api_fixture().await;
    let detail = call(&state, "/api/v1/catalog/show", "GET", Value::Null, &alice).await;
    let url = detail.2["backdrop_url"].as_str().unwrap().to_owned();
    // The manager is unreachable and nothing is cached yet.
    assert_eq!(
        call(&state, &url, "GET", Value::Null, "").await.0,
        StatusCode::NOT_FOUND
    );
    let dir = state.config.cache.join("artwork");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("title-sonarr-1-10-fanart"),
        b"\xff\xd8\xff\xe0fixture",
    )
    .unwrap();
    std::fs::write(
        dir.join("title-sonarr-1-10-fanart.source"),
        "/MediaCover/10/fanart.jpg",
    )
    .unwrap();
    let served = call(&state, &url, "GET", Value::Null, "").await;
    assert_eq!(served.0, StatusCode::OK);
    assert_eq!(served.1["content-type"], "image/jpeg");
    assert_eq!(served.1["cache-control"], "private, max-age=86400");
    let grant = url.split("grant=").nth(1).unwrap();
    for path in [
        "/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant=wrong".to_owned(),
        format!("/api/v1/catalog/titles/sonarr-1/10/artwork/banner?grant={grant}"),
        format!("/api/v1/catalog/titles/sonarr-1/11/artwork/fanart?grant={grant}"),
        format!("/api/v1/catalog/titles/..%2Fx/10/artwork/fanart?grant={grant}"),
        format!("/api/v1/catalog/titles/sonarr-1/-1/artwork/fanart?grant={grant}"),
    ] {
        let status = call(&state, &path, "GET", Value::Null, "").await.0;
        assert!(
            matches!(status, StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED),
            "{path}: {status}"
        );
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/catalog/titles/sonarr-1/10/artwork/fanart?grant=wrong",
            "GET",
            Value::Null,
            ""
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    // Cleanup keeps artwork its titles still have and removes everything else.
    for name in [
        "title-radarr-1-2-poster",
        "title-radarr-1-2-fanart",
        "title-radarr-1-99-poster",
        "title-removed-7-poster",
        "title-removed-7-poster.source",
        "title-sonarr-1-10-fanart.abc.tmp",
        "show",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    assert_eq!(
        prune_artwork(&state, &super::snapshot(&state))
            .await
            .unwrap(),
        4
    );
    let mut left = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(
        left,
        [
            "show",
            "title-radarr-1-2-poster",
            "title-sonarr-1-10-fanart",
            "title-sonarr-1-10-fanart.abc.tmp",
            "title-sonarr-1-10-fanart.source"
        ]
    );
    // A recent download in progress is kept; a service read later keeps its art.
    assert!(artwork_wanted(
        "title-radarr-2-5-poster",
        &Snapshot::default(),
        &HashSet::from(["radarr-2".to_owned()])
    ));
    assert!(!artwork_wanted(
        "title-radarr-2-5-poster",
        &Snapshot::default(),
        &HashSet::new()
    ));
    assert!(artwork_wanted(
        "title-odd",
        &Snapshot::default(),
        &HashSet::new()
    ));
}

#[tokio::test]
async fn jellyfin_items_show_their_title_backdrop_and_logo() {
    let (_temp, state, _, _) = api_fixture().await;
    let shows = inventory(
        "sonarr",
        json!([{"id":10,"tvdbId":201,"title":"Series","monitored":true,"images":[
            {"coverType":"fanart","url":"/MediaCover/10/fanart.jpg"},
            {"coverType":"clearlogo","url":"/MediaCover/10/clearlogo.png"}]}]),
        json!({}),
        json!([]),
    );
    replace(&state, snapshot(vec![("sonarr", shows)]));
    let token = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "jellyfin".into(),
        "TV".into(),
    )
    .await
    .unwrap();
    // Listed, as the fixture's files do not exist for a single item's media sources.
    let item = |id: &str| {
        let (state, path) = (state.clone(), format!("/Items?Ids={id}&api_key={token}"));
        async move {
            let (status, _, page) = call(&state, &path, "GET", Value::Null, "").await;
            assert_eq!(status, StatusCode::OK, "{page}");
            page["Items"][0].clone()
        }
    };
    let show = item("show").await;
    let grant = show["BackdropImageTags"][0].as_str().unwrap().to_owned();
    assert_eq!(grant.len(), 64);
    assert_eq!(show["ImageTags"]["Logo"], grant);
    // Episodes and seasons point at their series' images.
    for id in ["e1", "season1"] {
        let child = item(id).await;
        assert_eq!(child["BackdropImageTags"], json!([]));
        assert_eq!(child["ParentBackdropItemId"], "show");
        assert_eq!(child["ParentBackdropImageTags"], json!([grant]));
        assert_eq!(child["ParentLogoItemId"], "show");
        assert_eq!(child["ParentLogoImageTag"], grant);
    }
    let plain = item("movie-plain").await;
    assert_eq!(plain["BackdropImageTags"], json!([]));
    assert!(plain["ImageTags"].get("Logo").is_none());
    assert!(plain.get("ParentBackdropItemId").is_none());

    let image = |path: String| {
        let state = state.clone();
        async move {
            let (status, headers, _) = call(&state, &path, "GET", Value::Null, "").await;
            (status, headers)
        }
    };
    // Nothing is cached and the manager is unreachable.
    assert_eq!(
        image(format!("/Items/show/Images/Backdrop?tag={grant}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let dir = state.config.cache.join("artwork");
    std::fs::create_dir_all(&dir).unwrap();
    for (kind, bytes) in [
        ("fanart.jpg", &b"\xff\xd8\xff\xe0fixture"[..]),
        ("clearlogo.png", &b"\x89PNG\r\n\x1a\nfixture"[..]),
    ] {
        let (name, _) = kind.split_once('.').unwrap();
        std::fs::write(dir.join(format!("title-sonarr-1-10-{name}")), bytes).unwrap();
        std::fs::write(
            dir.join(format!("title-sonarr-1-10-{name}.source")),
            format!("/MediaCover/10/{kind}"),
        )
        .unwrap();
    }
    for (path, kind) in [
        (
            format!("/Items/show/Images/Backdrop?tag={grant}"),
            "image/jpeg",
        ),
        (
            format!("/Items/show/Images/Backdrop/0?tag={grant}"),
            "image/jpeg",
        ),
        (
            format!("/Items/e1/Images/Backdrop?api_key={token}"),
            "image/jpeg",
        ),
        (format!("/Items/show/Images/Logo?tag={grant}"), "image/png"),
    ] {
        let (status, headers) = image(path.clone()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(headers["content-type"], kind, "{path}");
    }
    for path in [
        format!("/Items/show/Images/Backdrop/1?tag={grant}"),
        format!("/Items/show/Images/Banner?tag={grant}"),
        format!("/Items/movie-plain/Images/Backdrop?tag={grant}"),
        "/Items/show/Images/Backdrop".to_owned(),
    ] {
        let status = image(path.clone()).await.0;
        assert!(
            matches!(status, StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED),
            "{path}: {status}"
        );
    }
    // Cleanup keeps the logo the series still has.
    prune_artwork(&state, &super::snapshot(&state))
        .await
        .unwrap();
    assert!(dir.join("title-sonarr-1-10-clearlogo").is_file());
}
