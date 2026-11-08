use super::*;
use crate::test_support::fixture;
use axum::extract::State as Stub;
use std::sync::{Arc, Mutex};

const ALBUM: &str = "21159e3f-172e-43f6-aa7d-8e06a81fea49";
const ARTIST: &str = "0d79fe8e-ba27-4859-bb8c-2f255f346853";

/// Lidarr as a request sees it. Its albums have no tracks until `loaded`.
#[derive(Default)]
struct Lidarr {
    artists: Vec<Value>,
    albums: Vec<Value>,
    loaded: bool,
    added: Option<Value>,
    calls: Vec<String>,
}
type Shared = Arc<Mutex<Lidarr>>;

impl Lidarr {
    fn album(&self, mut album: Value) -> Value {
        let duration = if self.loaded { 2_480_990 } else { 0 };
        album["releases"] = json!([{"monitored":true,"duration":duration}]);
        album
    }
}

fn stub(lidarr: Shared) -> Router {
    Router::new()
        .route(
            "/api/v1/album/lookup",
            get(|| async {
                Json(json!([{"title":"ARIRANG","foreignAlbumId":ALBUM,
                    "artist":{"artistName":"BTS","foreignArtistId":ARTIST}}]))
            }),
        )
        .route(
            "/api/v1/album",
            get(|Stub(l): Stub<Shared>| async move {
                let l = l.lock().unwrap();
                Json(json!(
                    l.albums
                        .iter()
                        .map(|a| l.album(a.clone()))
                        .collect::<Vec<_>>()
                ))
            })
            .post(
                |Stub(l): Stub<Shared>, Json(body): Json<Value>| async move {
                    let mut l = l.lock().unwrap();
                    l.calls.push("add album".into());
                    if !l.artists.iter().any(|a| a["foreignArtistId"] == ARTIST) {
                        let mut artist = body["artist"].clone();
                        artist["id"] = json!(5);
                        l.artists.push(artist);
                    }
                    let album = json!({"id":7,"foreignAlbumId":ALBUM,"artistId":5,
                    "monitored":body["monitored"],"anyReleaseOk":true});
                    l.albums.push(album.clone());
                    l.added = Some(body);
                    Json(l.album(album))
                },
            ),
        )
        .route(
            "/api/v1/album/{id}",
            axum::routing::put(
                |Stub(l): Stub<Shared>, Json(body): Json<Value>| async move {
                    l.lock().unwrap().calls.push("monitor album".into());
                    Json(body)
                },
            ),
        )
        .route(
            "/api/v1/artist",
            get(|Stub(l): Stub<Shared>| async move { Json(json!(l.lock().unwrap().artists)) })
                .post(
                    |Stub(l): Stub<Shared>, Json(mut body): Json<Value>| async move {
                        let mut l = l.lock().unwrap();
                        l.calls.push("add artist".into());
                        l.added = Some(body.clone());
                        body["id"] = json!(5);
                        l.artists.push(body.clone());
                        Json(body)
                    },
                ),
        )
        .route(
            "/api/v1/artist/lookup",
            get(|| async {
                Json(json!([{"artistName":"BTS","foreignArtistId":ARTIST,"images":[]}]))
            }),
        )
        .route(
            "/api/v1/artist/{id}",
            get(|Stub(l): Stub<Shared>| async move { Json(l.lock().unwrap().artists[0].clone()) })
                .put(
                    |Stub(l): Stub<Shared>, Json(body): Json<Value>| async move {
                        let mut l = l.lock().unwrap();
                        l.calls.push("monitor artist".into());
                        l.artists[0] = body.clone();
                        Json(body)
                    },
                ),
        )
        .route(
            "/api/v1/command",
            post(
                |Stub(l): Stub<Shared>, Json(body): Json<Value>| async move {
                    let call = format!("{} {}", body["name"].as_str().unwrap(), body["albumIds"]);
                    l.lock().unwrap().calls.push(call);
                    Json(json!({"id":1}))
                },
            ),
        )
        .with_state(lidarr)
}

/// An approved request for the album, its claimed job, and `lidarr` serving the manager.
async fn requested(
    lidarr: Shared,
) -> (
    tempfile::TempDir,
    AppState,
    tokio::task::JoinHandle<()>,
    thelxinoe_jobs::Job,
) {
    requested_target(lidarr, "item", ALBUM).await
}

/// An approved request for `external`, its claimed job, and `lidarr` serving the manager.
async fn requested_target(
    lidarr: Shared,
    target: &'static str,
    external: &'static str,
) -> (
    tempfile::TempDir,
    AppState,
    tokio::task::JoinHandle<()>,
    thelxinoe_jobs::Job,
) {
    let (temp, mut state, _) = fixture().await;
    let media = temp.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    Arc::get_mut(&mut state.config).unwrap().media = media.clone();
    let server =
        crate::managers::bindings::tests::manager(&state, &media, "lidarr", stub(lidarr)).await;
    let defaults = json!({"root_folder":"/media/music","quality_profile":1,
        "metadata_profile":1,"monitored":true});
    state.db.write("test.request", move |db| {
        db.execute("UPDATE manager_services SET defaults=?1 WHERE id='lidarr'", [defaults.to_string()])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at,target) VALUES ('request','alice','lidarr','g',?1,'ARIRANG','approved',1,1,?2)", [external, target])?;
        thelxinoe_jobs::enqueue_in(db, "manager.request", "request", &json!({"request_id":"request"}), 0)?;
        Ok(())
    }).await.unwrap();
    let job = thelxinoe_jobs::Queue(state.db.clone())
        .claim()
        .await
        .unwrap()
        .unwrap();
    (temp, state, server, job)
}

/// The request's state and album, and whether its job is queued to run again later.
async fn progress(state: &AppState, job: &str) -> (String, Option<i64>, bool) {
    let job = job.to_owned();
    state.db.read("test.progress", move |db| {
        Ok(db.query_row(
            "SELECT r.state,r.manager_id,j.state='queued' AND j.available_at>?2 FROM acquisition_requests r,jobs j WHERE r.id='request' AND j.id=?1",
            rusqlite::params![job, thelxinoe_core::now()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }).await.unwrap()
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn a_new_artist_monitors_only_the_requested_album_and_waits_for_its_tracks() {
    let lidarr = Shared::default();
    let (_temp, state, server, job) = requested(lidarr.clone()).await;
    assert!(
        !acquire(&state, &job).await.unwrap(),
        "Lidarr rejects every release of an album without tracks"
    );
    {
        let l = lidarr.lock().unwrap();
        let artist = &l.added.as_ref().unwrap()["artist"];
        assert_eq!(artist["monitored"], true);
        assert_eq!(artist["monitorNewItems"], "none");
        assert_eq!(
            artist["addOptions"],
            json!({"albumsToMonitor":[ALBUM],"searchForMissingAlbums":false})
        );
        assert_eq!(l.calls, ["add album"]);
    }
    assert_eq!(
        progress(&state, &job.id).await,
        ("adding".into(), Some(7), true)
    );
    lidarr.lock().unwrap().loaded = true;
    assert!(acquire(&state, &job).await.unwrap());
    assert_eq!(
        lidarr.lock().unwrap().calls,
        ["add album", "AlbumSearch [7]"]
    );
    assert_eq!(progress(&state, &job.id).await.0, "requested");
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn an_unmonitored_artist_is_monitored_before_the_add_and_a_slow_album_is_searched_at_last() {
    let lidarr = Shared::default();
    lidarr
        .lock()
        .unwrap()
        .artists
        .push(json!({"id":5,"foreignArtistId":ARTIST,"artistName":"BTS","monitored":false}));
    let (_temp, state, server, job) = requested(lidarr.clone()).await;
    let job = thelxinoe_jobs::Job {
        attempts: ALBUM_WAIT_RUNS,
        ..job
    };
    assert!(acquire(&state, &job).await.unwrap());
    {
        let l = lidarr.lock().unwrap();
        // The refresh that the add starts writes back the artist it read.
        assert_eq!(l.calls, ["monitor artist", "add album", "AlbumSearch [7]"]);
        assert_eq!(l.added.as_ref().unwrap()["artist"]["monitored"], true);
        assert_eq!(l.artists[0]["monitorNewItems"], "none");
    }
    assert_eq!(progress(&state, &job.id).await.0, "requested");
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn monitoring_a_new_artist_adds_it_for_its_future_albums_only() {
    let lidarr = Shared::default();
    let (_temp, state, server, job) = requested_target(lidarr.clone(), "artist", ARTIST).await;
    assert!(acquire(&state, &job).await.unwrap());
    {
        let l = lidarr.lock().unwrap();
        let artist = l.added.as_ref().unwrap();
        assert_eq!(artist["monitored"], true);
        assert_eq!(artist["monitorNewItems"], "all");
        assert_eq!(artist["rootFolderPath"], "/media/music");
        assert_eq!(
            artist["addOptions"],
            json!({"monitor":"future","searchForMissingAlbums":false})
        );
        assert_eq!(l.calls, ["add artist"]);
    }
    assert_eq!(
        progress(&state, &job.id).await,
        ("requested".into(), Some(5), false)
    );
    server.abort();
}

#[cfg_attr(windows, ignore = "needs the Linux server container environment")]
#[tokio::test]
async fn monitoring_an_artist_lidarr_holds_takes_on_its_new_albums() {
    let lidarr = Shared::default();
    lidarr
        .lock()
        .unwrap()
        .artists
        .push(json!({"id":5,"foreignArtistId":ARTIST,
        "artistName":"BTS","monitored":true,"monitorNewItems":"none"}));
    let (_temp, state, server, job) = requested_target(lidarr.clone(), "artist", ARTIST).await;
    assert!(acquire(&state, &job).await.unwrap());
    {
        let l = lidarr.lock().unwrap();
        assert_eq!(l.calls, ["monitor artist"]);
        assert_eq!(l.artists[0]["monitorNewItems"], "all");
    }
    assert_eq!(progress(&state, &job.id).await.0, "requested");
    server.abort();
}
