use super::*;
use crate::online::{
    Runtime,
    oauth::{Credential, tests::fixture},
};
use crate::test_support::call;
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode, header},
    routing::get,
};
use std::collections::HashMap;
use std::sync::Arc;
const CHANNEL: &str = "UCaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "UCbbbbbbbbbbbbbbbbbbbbbb";
const VIDEO: &str = "abcdefghijk";

#[tokio::test]
async fn connected_youtube_avatar_is_backfilled_without_subscribing_to_own_channel() {
    let (_temp, mut state, cookie) = fixture().await;
    let generation = connect(&state, "alice").await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE online_accounts SET external_id=?1 WHERE user_id='alice'",
                [CHANNEL],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = requests.clone();
    let server = stub(&mut state, axum::Router::new().route("/channels", get(move |Query(query): Query<HashMap<String,String>>| {
        let count = count.clone();
        async move {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(query["id"], CHANNEL);
            assert!(query["part"].contains("snippet"));
            Json(json!({"items":[{"id":CHANNEL,"snippet":{"title":"Alice","thumbnails":{"medium":{"url":"https://yt3.googleusercontent.com/alice.jpg"}}}}]}))
        }
    }))).await;
    for _ in 0..2 {
        step(
            &state,
            Turn {
                user: "alice".into(),
                generation: generation.clone(),
                cursor: Cursor {
                    phase: "channels".into(),
                    ..Default::default()
                },
                failures: 0,
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
    let account = call(
        &state,
        "/api/v1/online/youtube",
        "GET",
        Value::Null,
        &cookie,
    )
    .await
    .2;
    assert_eq!(
        account["account"]["avatar_url"],
        "https://yt3.googleusercontent.com/alice.jpg"
    );
    assert_eq!(account["account"]["display_name"], "Alice");
    state
        .db
        .write("test.fixture", |db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM youtube_subscriptions", [], |row| row
                    .get::<_, i64>(
                    0
                ))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    call(
        &state,
        "/api/v1/online/youtube",
        "DELETE",
        Value::Null,
        &cookie,
    )
    .await;
    assert!(
        call(
            &state,
            "/api/v1/online/youtube",
            "GET",
            Value::Null,
            &cookie
        )
        .await
        .2["account"]["avatar_url"]
            .is_null()
    );
    server.abort();
}

pub(crate) async fn connect(state: &AppState, user: &str) -> String {
    let encrypted = state
        .secrets
        .encrypt(
            &format!("online:youtube:{user}"),
            &serde_json::to_vec(&Credential {
                access_token: format!("{user}-test-token"),
                refresh_token: Some("refresh-fixture".into()),
                scope: crate::online::oauth::SCOPE.into(),
            })
            .unwrap(),
        )
        .unwrap();
    let owner = user.to_owned();
    let generation = id();
    let copy = generation.clone();
    state.db.write("test.fixture", move|db|{
        db.execute("INSERT INTO online_accounts(user_id,provider,generation,status,credential,expires_at,updated_at) VALUES (?1,'youtube',?2,'connected',?3,?4,?5)",params![owner,copy,encrypted,now()+3600,now()])?;
        db.execute("INSERT INTO youtube_sync(user_id,generation) VALUES (?1,?2)",params![owner,copy])?;Ok(())
    }).await.unwrap();
    generation
}
pub(crate) async fn stub(
    state: &mut AppState,
    router: axum::Router,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    state.online = Arc::new(Runtime {
        http: reqwest::Client::new(),
        authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
        token: format!("http://{address}/token"),
        api: format!("http://{address}"),
        slots: tokio::sync::Semaphore::new(4),
        artwork: tokio::sync::Semaphore::new(16),
        thumbnails: crate::online::thumbnails::Cache::default(),
        refresh: tokio::sync::Mutex::new(()),
        extraction: tokio::sync::Semaphore::new(2),
        streamlink: super::super::streamlink_worker::Pool::default(),
        youtube_worker: super::super::youtube_worker::Pool::default(),
        network: crate::online::network::Runtime::default(),
        streams: crate::online::streams::Runtime::default(),
        twitch: crate::online::twitch::Runtime::default(),
        kick: crate::online::kick::Runtime::default(),
    });
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() })
}
#[tokio::test]
async fn fair_sync_resumes_pages_and_preserves_private_state() {
    let (_temp, mut state, session) = fixture().await;
    connect(&state, "alice").await;
    connect(&state, "bob").await;
    state.db.write("test.fixture", |db|{db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES ('alice',?1,'Previous subscription','previous',1)",[OTHER])?;Ok(())}).await.unwrap();
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let record = requests.clone();
    let router=axum::Router::new().route("/{endpoint}",get(move|Path(endpoint):Path<String>,Query(query):Query<HashMap<String,String>>,headers:HeaderMap|{
        let record=record.clone();async move{
            let alice=headers[header::AUTHORIZATION]=="Bearer alice-test-token";
            record.lock().await.push(format!("{}:{endpoint}:{}",if alice{"alice"}else{"bob"},query.get("pageToken").map(String::as_str).unwrap_or("")));
            Json(match endpoint.as_str(){
                "subscriptions" if alice&&query.get("pageToken").is_none_or(String::is_empty)=>json!({"items":[{"snippet":{"resourceId":{"channelId":CHANNEL},"title":"Channel"}}],"nextPageToken":"page-two"}),
                "subscriptions" if alice=>{assert_eq!(query["pageToken"],"page-two");json!({"items":[]})},
                "subscriptions"=>json!({"items":[{"snippet":{"resourceId":{"channelId":CHANNEL},"title":"Channel"}}]}),
                "channels"=>json!({"items":[{"id":CHANNEL,"contentDetails":{"relatedPlaylists":{"uploads":"UUaaaaaaaaaaaaaaaaaaaaaa"}}}]}),
                "playlistItems"=>json!({"items":[{"contentDetails":{"videoId":VIDEO,"videoPublishedAt":"2026-09-19T00:00:00Z"}}]}),
                "videos"=>json!({"items":[{"id":VIDEO,"snippet":{"channelId":CHANNEL,"title":if alice{"Alice-only metadata"}else{"Bob-only metadata"},"channelTitle":"Channel","publishedAt":"2026-09-19T00:00:00Z","liveBroadcastContent":"none"},"contentDetails":{"duration":"PT20M"},"status":{"privacyStatus":if alice{"private"}else{"public"}},"liveStreamingDetails":{"actualEndTime":"2026-09-19T00:20:00Z"}}]}),
                _=>panic!("Unexpected endpoint"),
            })
        }
    }));
    let server = stub(&mut state, router).await;
    assert!(tick(&state).await.unwrap());
    assert!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT active FROM youtube_subscriptions WHERE user_id='alice' AND channel_id=?1",
                [OTHER],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap()
    );
    assert!(tick(&state).await.unwrap());
    assert_eq!(
        &requests.lock().await[..2],
        ["alice:subscriptions:", "bob:subscriptions:"]
    );
    // A process restart recreates runtime state but resumes the saved page.
    let mut restarted = AppState::open((*state.config).clone()).await.unwrap();
    restarted.online = state.online.clone();
    for _ in 0..40 {
        if !tick(&restarted).await.unwrap() {
            break;
        }
    }
    let completed = restarted
        .db
        .write("test.fixture", |db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM youtube_sync WHERE last_complete IS NOT NULL",
                [],
                |r| r.get::<_, u32>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(completed, 2);
    let requests = requests.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|s| s.as_str() == "alice:subscriptions:")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|s| s.as_str() == "alice:subscriptions:page-two")
            .count(),
        1
    );
    assert_eq!(quota::status(&restarted).await.unwrap()["used"], 9);
    let mine = call(
        &restarted,
        "/api/v1/online/youtube/feed",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(mine.0, StatusCode::OK);
    assert_eq!(mine.2["items"][0]["title"], "Alice-only metadata");
    assert_eq!(mine.2["items"][0]["broadcast"], "replay");
    assert!(mine.2["items"][0].get("artwork_url").is_none());
    assert_eq!(
        call(
            &restarted,
            "/api/v1/online/youtube/watchlist",
            "POST",
            json!({"url":format!("https://youtu.be/{VIDEO}")}),
            &session
        )
        .await
        .0,
        StatusCode::OK
    );
    let bob = crate::test_support::issue_session(
        &restarted.db,
        "bob".into(),
        "web".into(),
        "test".into(),
    )
    .await
    .unwrap();
    let bob = format!("thelxinoe_session={bob}");
    let other = call(
        &restarted,
        "/api/v1/online/youtube/feed?watchlist=true",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(other.2["total"], 0);
    call(
        &restarted,
        "/api/v1/online/youtube",
        "DELETE",
        Value::Null,
        &session,
    )
    .await;
    let retained = call(
        &restarted,
        "/api/v1/online/youtube/feed?watchlist=true",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(retained.2["total"], 1);
    call(
        &restarted,
        "/api/v1/online/youtube/data",
        "DELETE",
        Value::Null,
        &session,
    )
    .await;
    let removed = call(
        &restarted,
        "/api/v1/online/youtube/feed?watchlist=true",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(removed.2["total"], 0);
    let other = call(
        &restarted,
        "/api/v1/online/youtube/feed",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(other.2["items"][0]["title"], "Bob-only metadata");
    server.abort();
}
const NEW: &str = "bcdefghijkl";
const NEXT: &str = "cdefghijklm";
async fn upload_sync(
    state: &mut AppState,
    pages: HashMap<&'static str, Vec<&'static str>>,
) -> (
    Arc<tokio::sync::Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,uploads,snapshot,active) VALUES ('alice',?1,'Channel','UUaaaaaaaaaaaaaaaaaaaaaa','current',1)", [CHANNEL])?;
        Ok(())
    }).await.unwrap();
    let published = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let record = requests.clone();
    let router = axum::Router::new().route("/{endpoint}", get(move |Path(endpoint): Path<String>, Query(query): Query<HashMap<String, String>>| {
        let record = record.clone();
        let pages = pages.clone();
        let published = published.clone();
        async move {
            Json(match endpoint.as_str() {
                "playlistItems" => {
                    let token = query.get("pageToken").cloned().unwrap_or_default();
                    record.lock().await.push(format!("playlistItems:{token}"));
                    let items: Vec<Value> = pages[token.as_str()].iter().map(|id| json!({"contentDetails":{"videoId":id,"videoPublishedAt":published}})).collect();
                    json!({"items":items,"nextPageToken":if token.is_empty() {"page-two"} else {""}})
                }
                "videos" => {
                    record.lock().await.push(format!("videos:{}", query["id"]));
                    let items: Vec<Value> = query["id"].split(',').map(|id| json!({"id":id,"snippet":{"channelId":CHANNEL,"title":"Upload","channelTitle":"Channel","publishedAt":published,"liveBroadcastContent":"none"},"contentDetails":{"duration":"PT20M"},"status":{"privacyStatus":"public"}})).collect();
                    json!({"items":items})
                }
                _ => panic!("Unexpected endpoint"),
            })
        }
    }));
    let server = stub(state, router).await;
    (requests, server)
}
async fn sync_uploads(state: &AppState) {
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE youtube_sync SET cursor='{\"phase\":\"uploads\"}',next_run=0",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    for _ in 0..20 {
        if !tick(state).await.unwrap() {
            break;
        }
    }
    let error = state
        .db
        .read("test.fixture", |db| {
            Ok(db.query_row("SELECT error FROM youtube_sync", [], |r| {
                r.get::<_, Option<String>>(0)
            })?)
        })
        .await
        .unwrap();
    assert_eq!(error, None);
}
#[tokio::test]
async fn known_uploads_skip_metadata_and_older_pages_once_backfilled() {
    let (_temp, mut state, _session) = fixture().await;
    connect(&state, "alice").await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,published_at,metadata_at) VALUES ('alice',?1,?2,'Opened by URL',?3,?3)", params![VIDEO, CHANNEL, now()])?;
        Ok(())
    }).await.unwrap();
    let (requests, server) = upload_sync(
        &mut state,
        HashMap::from([("", vec![NEW, VIDEO]), ("page-two", vec![NEXT])]),
    )
    .await;
    // A tail known from elsewhere does not end the first backfill.
    sync_uploads(&state).await;
    assert_eq!(
        std::mem::take(&mut *requests.lock().await),
        [
            "playlistItems:".to_owned(),
            format!("videos:{NEW},{VIDEO}"),
            "playlistItems:page-two".to_owned(),
            format!("videos:{NEXT}"),
        ]
    );
    sync_uploads(&state).await;
    assert_eq!(
        std::mem::take(&mut *requests.lock().await),
        ["playlistItems:"]
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE youtube_videos SET metadata_at=?1 WHERE video_id=?2",
                params![now() - 86401, VIDEO],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    sync_uploads(&state).await;
    assert_eq!(
        std::mem::take(&mut *requests.lock().await),
        ["playlistItems:".to_owned(), format!("videos:{NEW},{VIDEO}")]
    );
    assert_eq!(quota::status(&state).await.unwrap()["used"], 7);
    server.abort();
}
#[tokio::test]
async fn stored_uploads_past_the_first_page_refresh_daily() {
    const OLD: &str = "lmnopqrstuv";
    let (_temp, mut state, _session) = fixture().await;
    connect(&state, "alice").await;
    let (requests, server) = upload_sync(&mut state, HashMap::from([("", vec![VIDEO])])).await;
    state.db.write("test.fixture", |db| {
        db.execute("UPDATE youtube_subscriptions SET backfilled=1", [])?;
        for (video, published, metadata) in [(VIDEO, now(), now()), (NEXT, now() - 86400, now() - 86401), (OLD, now() - 120 * 86400, now() - 86401)] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,published_at,metadata_at) VALUES ('alice',?1,?2,'Stored',?3,?4)", params![video, CHANNEL, published, metadata])?;
        }
        Ok(())
    }).await.unwrap();
    sync_uploads(&state).await;
    assert_eq!(
        *requests.lock().await,
        ["playlistItems:".to_owned(), format!("videos:{NEXT}")]
    );
    let titles = state
        .db
        .read("test.fixture", |db| {
            Ok(db
                .prepare("SELECT title FROM youtube_videos ORDER BY video_id")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(titles, ["Stored", "Upload", "Stored"]);
    server.abort();
}
#[tokio::test]
async fn stale_video_cleanup_keeps_requested_downloads() {
    const OLD: &str = "lmnopqrstuv";
    let (_temp, state, _session) = fixture().await;
    let generation = connect(&state, "alice").await;
    state.db.write("test.fixture", |db| {
        let long_ago = now() - 120 * 86400;
        for video in [VIDEO, OLD] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title,published_at,metadata_at) VALUES ('alice',?1,'Old video',?2,?2)", params![video, long_ago])?;
        }
        db.execute("INSERT INTO youtube_download_requests(user_id,video_id,requested_at) VALUES ('alice',?1,?2)", params![VIDEO, now()])?;
        Ok(())
    }).await.unwrap();
    let turn = Turn {
        user: "alice".into(),
        generation,
        cursor: Cursor::default(),
        failures: 0,
    };
    save(
        &state,
        turn,
        false,
        storage::Page::Videos {
            ids: vec![],
            rows: vec![],
            backfilled: None,
        },
    )
    .await
    .unwrap();
    let (videos, requests) = state
        .db
        .read("test.fixture", |db| {
            let videos: Vec<String> = db
                .prepare("SELECT video_id FROM youtube_videos WHERE user_id='alice'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let requests: i64 =
                db.query_row("SELECT COUNT(*) FROM youtube_download_requests", [], |r| {
                    r.get(0)
                })?;
            Ok((videos, requests))
        })
        .await
        .unwrap();
    assert_eq!(videos, [VIDEO]);
    assert_eq!(requests, 1);
}

#[tokio::test]
async fn a_late_provider_reply_cannot_restore_deleted_data() {
    let (_temp, mut state, session) = fixture().await;
    connect(&state, "alice").await;
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let s = started.clone();
    let r = release.clone();
    let server=stub(&mut state,axum::Router::new().route("/subscriptions",get(move||{let s=s.clone();let r=r.clone();async move{s.notify_one();r.notified().await;Json(json!({"items":[{"snippet":{"resourceId":{"channelId":CHANNEL},"title":"Stale subscription"}}]}))}}))).await;
    let worker = state.clone();
    let task = tokio::spawn(async move { tick(&worker).await });
    started.notified().await;
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/data",
            "DELETE",
            Value::Null,
            &session
        )
        .await
        .0,
        StatusCode::OK
    );
    release.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM youtube_subscriptions",
                [],
                |r| r.get::<_, u32>(0)
            )?))
            .await
            .unwrap(),
        0
    );
    server.abort();
}
#[tokio::test]
async fn direct_url_playback_resolves_metadata_without_saving_a_watchlist_item() {
    let (_temp, mut state, session) = fixture().await;
    connect(&state, "alice").await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE youtube_sync SET next_run=?1", [now() + 1800])?;
            Ok(())
        })
        .await
        .unwrap();
    let server = stub(&mut state, axum::Router::new().route("/videos", get(|Query(query): Query<HashMap<String, String>>| async move {
        assert_eq!(query["id"], VIDEO);
        Json(json!({"items":[{"id":VIDEO,"snippet":{"channelId":CHANNEL,"title":"Direct URL title","channelTitle":"Channel","publishedAt":"2026-09-19T00:00:00Z","liveBroadcastContent":"none"},"contentDetails":{"duration":"PT20M"},"status":{"privacyStatus":"public"}}]}))
    }))).await;
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/resolve",
            "POST",
            json!({"video_id":VIDEO}),
            &session
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(tick(&state).await.unwrap());
    let (title, private_rows, saved) = state.db.write("test.fixture", |db| {
        Ok((
            db.query_row("SELECT title FROM youtube_videos WHERE user_id='alice' AND video_id=?1 AND metadata_at>0", [VIDEO], |r| r.get::<_, String>(0))?,
            db.query_row("SELECT COUNT(*) FROM youtube_videos WHERE user_id='bob'", [], |r| r.get::<_, u32>(0))?,
            db.query_row("SELECT COUNT(*) FROM youtube_watchlist_items", [], |r| r.get::<_, u32>(0))?,
        ))
    }).await.unwrap();
    assert_eq!(title, "Direct URL title");
    assert_eq!(private_rows, 0);
    assert_eq!(saved, 0);
    server.abort();
}

#[test]
fn untrusted_urls_and_durations_are_bounded() {
    assert_eq!(duration("P1DT2H3M4.5S"), Some(93785));
    for bad in [
        "bad",
        "P1YT2H",
        "PTNaNS",
        "PT-1S",
        "PT5M2H",
        "PT99999999999999H",
    ] {
        assert_eq!(duration(bad), None);
    }
    for good in [
        "https://youtu.be/abcdefghijk?t=12",
        "https://www.youtube.com/shorts/abcdefghijk",
        "https://youtube.com/watch?v=abcdefghijk&list=ignored",
        "abcdefghijk",
    ] {
        assert_eq!(crate::online::feed::video_id(good).as_deref(), Some(VIDEO));
    }
    for bad in [
        "https://youtube.com.evil.test/watch?v=abcdefghijk",
        "https://user@youtube.com/watch?v=abcdefghijk",
        "http://127.0.0.1/a",
        "file:///x",
        "https://youtu.be/a/b",
        "https://youtube.com/playlist?list=abcdefghijk",
    ] {
        assert!(crate::online::feed::video_id(bad).is_none());
    }
}
