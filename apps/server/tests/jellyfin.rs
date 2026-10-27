#[path = "../../../tests/helpers/auth-session.rs"]
mod auth_session;
#[path = "../../../tests/helpers/server-config.rs"]
mod configuration;
#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use thelxinoe_server::{AppState, config::Config, router};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(configuration::config(temp.path()))
        .await
        .unwrap();
    let hash = thelxinoe_auth::password_hash("a long test-only password".into(), None)
        .await
        .unwrap();
    state
        .db
        .write("test.fixture", move |db| {
            for name in ["alice", "bob"] {
                db.execute(
                    "INSERT INTO users(id,username,password_hash,role,created_at) VALUES (?1,?1,?2,'user',1)",
                    rusqlite::params![name, hash],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let token = auth_session::issue_session(
        &state.db,
        "alice".into(),
        "device".into(),
        "Browser test".into(),
    )
    .await
    .unwrap();
    (temp, state, token)
}
async fn send(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Bytes) {
    let mut req = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "media.test")
        .header("content-type", "application/json")
        .header("x-thelxinoe-client", "1");
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    let response = router(state.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes)
}
async fn call(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let (status, _, bytes) = send(state, path, method, body, headers).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
const DEVICE: &str = "MediaBrowser Client=\"Test%20client\", Device=\"Living room, TV\", DeviceId=\"tv-one\", Version=\"1\"";
async fn login(state: &AppState, user: &str, device: &str) -> String {
    let (status, login) = call(
        state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":user,"Pw":"a long test-only password"}),
        &[("Authorization", device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    login["AccessToken"].as_str().unwrap().to_owned()
}
fn run(program: &str, args: &[&str]) -> Vec<u8> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
/// Codec summaries of a file, such as `h264 160x90` or `aac 2ch`.
fn streams(path: &Path) -> Vec<String> {
    let probe: Value = serde_json::from_slice(&run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_streams",
            "-of",
            "json",
            path.to_str().unwrap(),
        ],
    ))
    .unwrap();
    probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| match s["channels"].as_i64() {
            Some(channels) => format!("{} {channels}ch", s["codec_name"].as_str().unwrap()),
            None => format!(
                "{} {}x{}",
                s["codec_name"].as_str().unwrap(),
                s["width"],
                s["height"]
            ),
        })
        .collect()
}
/// A scanned movie with AAC and AC-3 audio tracks, keyframes every five
/// seconds, and a music track. Both are tiny and generated on the runner.
async fn local_library(root: &Path, state: &AppState) -> (PathBuf, PathBuf) {
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    let movies = root.join("media/Movies");
    let album = root.join("media/Music/Artist/Album");
    std::fs::create_dir_all(&movies).unwrap();
    std::fs::create_dir_all(&album).unwrap();
    let movie = movies.join("Feature (2021).mkv");
    let track = album.join("01 Song.m4a");
    run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=s=160x90:r=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "18",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "1:a",
            "-c:v",
            "libx264",
            "-g",
            "120",
            "-sc_threshold",
            "0",
            "-c:a:0",
            "aac",
            "-c:a:1",
            "ac3",
            movie.to_str().unwrap(),
        ],
    );
    run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000",
            "-t",
            "12",
            "-c:a",
            "aac",
            "-metadata",
            "title=Song",
            "-metadata",
            "artist=Artist",
            "-metadata",
            "album=Album",
            "-metadata",
            "track=1",
            "-movflags",
            "+faststart",
            track.to_str().unwrap(),
        ],
    );
    let paths = [movies, root.join("media/Music")].map(|p| p.to_string_lossy().into_owned());
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies',?1),('music','Music','music',?2)", paths)?;
        Ok(())
    }).await.unwrap();
    for root in thelxinoe_catalog::roots(&state.db).await.unwrap() {
        thelxinoe_catalog::scan(&state.db, root, state.tools.runtime.media().unwrap())
            .await
            .unwrap();
    }
    (movie, track)
}
async fn grants(state: &AppState) -> i64 {
    state
        .db
        .read("test.fixture", |db| {
            Ok(db.query_row("SELECT COUNT(*) FROM playback_grants", [], |r| r.get(0))?)
        })
        .await
        .unwrap()
}
async fn playback_state(state: &AppState, id: &str) -> (String, String) {
    let id = id.to_owned();
    state
        .db
        .read("test.fixture", move |db| {
            Ok(db.query_row(
                "SELECT mode,state FROM playback_sessions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
}
async fn playbacks(state: &AppState, user: &str, media: &str) -> Vec<(String, String)> {
    let (user, media) = (user.to_owned(), media.to_owned());
    state
        .db
        .read("test.fixture", move |db| {
            Ok(db
                .prepare("SELECT id,state FROM playback_sessions WHERE user_id=?1 AND media_id=?2 ORDER BY created_at,rowid")?
                .query_map([user, media], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap()
}
/// Fetches a VOD playlist and its first segment with only the URL's grant.
/// Returns the segment durations and the streams of that first segment.
async fn vod(state: &AppState, url: &str, segment: &Path) -> (Vec<String>, Vec<String>) {
    let (status, headers, body) = send(state, url, "GET", Value::Null, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.apple.mpegurl"
    );
    let playlist = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD\n") && playlist.ends_with("#EXT-X-ENDLIST\n"),
        "{playlist}"
    );
    let durations = playlist
        .lines()
        .filter_map(|line| line.strip_prefix("#EXTINF:"))
        .map(str::to_owned)
        .collect();
    let first = playlist
        .lines()
        .find(|line| line.starts_with("segment-"))
        .unwrap();
    assert!(first.contains(".ts?grant="), "{first}");
    let base = url.split_once("index.m3u8").unwrap().0;
    let (status, _, body) = send(state, &format!("{base}{first}"), "GET", Value::Null, &[]).await;
    assert_eq!(status, StatusCode::OK);
    std::fs::write(segment, &body).unwrap();
    (durations, streams(segment))
}
async fn position(state: &AppState, item: &str, headers: &[(&str, &str)]) -> Value {
    let (status, item) = call(
        state,
        &format!("/Items/{item}"),
        "GET",
        Value::Null,
        headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{item}");
    item["UserData"]["PlaybackPositionTicks"].clone()
}

#[tokio::test]
async fn watchlist_library_skips_a_single_list_and_restores_folders_per_user() {
    let (_temp, state, token) = fixture().await;
    let bearer = format!("Bearer {token}");
    let web = [("Authorization", bearer.as_str())];
    let (status, lists) = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &web,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let default_list = lists[0]["id"].as_i64().unwrap();
    state.db.write("test.fixture", move |db| {
        for (video, title, published) in [("aaaaaaaaaaa", "Alpha", 100), ("bbbbbbbbbbb", "Bravo", 200), ("ccccccccccc", "Feed only", 300)] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration,published_at) VALUES('alice',?1,'channel',?2,'public',120,?3)", rusqlite::params![video,title,published])?;
        }
        db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES('alice','channel','Channel','s',1)", [])?;
        for (video, position) in [("bbbbbbbbbbb", 0), ("aaaaaaaaaaa", 1)] {
            db.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES('alice',?1,?2,?3,1)", rusqlite::params![default_list,video,position])?;
        }
        db.execute("INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES('mix','alice','Music mix',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let mut tokens = Vec::new();
    for user in ["alice", "bob"] {
        let (status, login) = call(
            &state,
            "/Users/AuthenticateByName",
            "POST",
            json!({"Username":user,"Pw":"a long test-only password"}),
            &[("Authorization", DEVICE)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        tokens.push(login["AccessToken"].as_str().unwrap().to_owned());
    }
    let alice = [("X-Emby-Token", tokens[0].as_str())];
    let bob = [("X-Emby-Token", tokens[1].as_str())];
    let (_, views) = call(&state, "/UserViews", "GET", Value::Null, &alice).await;
    let roots = views["Items"].as_array().unwrap();
    let root = roots
        .iter()
        .find(|item| item["Name"] == "Watchlists")
        .expect("YouTube watchlists have their own library")
        .clone();
    let root_id = root["Id"].as_str().unwrap();
    for path in [
        format!("/Items/{root_id}"),
        format!("/Users/alice/Items/{root_id}"),
    ] {
        let (status, item) = call(&state, &path, "GET", Value::Null, &alice).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item, root);
        assert_eq!(item["DisplayPreferencesId"], root_id);
    }
    let path = format!("/Items?ParentId={root_id}");
    let (status, single) = call(&state, &path, "GET", Value::Null, &alice).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(single["TotalRecordCount"], 2);
    assert_eq!(single["Items"][0]["Name"], "Bravo");
    assert_eq!(single["Items"][1]["Name"], "Alpha");
    assert!(
        single["Items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["Type"] == "Video" && item["IsFolder"] == false)
    );
    let video_id = single["Items"][0]["Id"].as_str().unwrap();
    let music_root = roots
        .iter()
        .find(|item| item["Name"] == "Playlists")
        .unwrap()["Id"]
        .as_str()
        .unwrap();
    let (_, music) = call(
        &state,
        &format!("/Items?ParentId={music_root}"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(music["TotalRecordCount"], 1);
    assert_eq!(music["Items"][0]["Name"], "Music mix");
    let (_, page) = call(
        &state,
        &format!("{path}&StartIndex=1&Limit=1&IncludeItemTypes=Video"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(page["TotalRecordCount"], 2);
    assert_eq!(page["StartIndex"], 1);
    assert_eq!(page["Items"].as_array().unwrap().len(), 1);
    assert_eq!(page["Items"][0]["Name"], "Alpha");
    for filter in [
        "SearchTerm=Feed",
        "IsFolder=true",
        "IncludeItemTypes=Playlist",
        "MediaTypes=Audio",
    ] {
        let (_, empty) = call(
            &state,
            &format!("{path}&{filter}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(empty["TotalRecordCount"], 0, "{filter}: {empty}");
    }
    let (_, empty) = call(&state, &path, "GET", Value::Null, &bob).await;
    assert_eq!(empty["TotalRecordCount"], 0);
    let (_, other_list) = call(
        &state,
        "/Playlists",
        "POST",
        json!({"Name":"Second list","Ids":[video_id]}),
        &alice,
    )
    .await;
    let second_id = other_list["Id"].as_str().unwrap();
    let (_, multiple) = call(&state, &path, "GET", Value::Null, &alice).await;
    assert_eq!(multiple["TotalRecordCount"], 2);
    assert!(
        multiple["Items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["Type"] == "Playlist" && item["IsFolder"] == true)
    );
    assert!(
        multiple["Items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["Id"] == second_id)
    );
    let (_, opened) = call(
        &state,
        &format!("/Items?ParentId={second_id}"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(opened["Items"][0]["Id"], video_id);
    assert_eq!(
        call(
            &state,
            &format!("/Items/{second_id}"),
            "GET",
            Value::Null,
            &bob
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&state, &path, "GET", Value::Null, &bob).await.1["TotalRecordCount"],
        0
    );
    let (_, recursive) = call(
        &state,
        &format!("{path}&Recursive=true&IncludeItemTypes=Video"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(recursive["TotalRecordCount"], 2);
    assert!(
        recursive["Items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["Type"] == "Video")
    );
    assert_eq!(
        call(
            &state,
            &format!("{path}&IncludeItemTypes=Video"),
            "GET",
            Value::Null,
            &alice
        )
        .await
        .1["TotalRecordCount"],
        0
    );
    let (_, web_lists) = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &web,
    )
    .await;
    let second = web_lists
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Second list")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/online/youtube/watchlists/{second}"),
            "DELETE",
            Value::Null,
            &web
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, restored) = call(&state, &path, "GET", Value::Null, &alice).await;
    assert_eq!(restored["Items"][0]["Id"], video_id);
    assert_eq!(restored["TotalRecordCount"], 2);
    let (_, refreshed) = call(&state, "/UserViews", "GET", Value::Null, &alice).await;
    assert!(
        refreshed["Items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == &root)
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "INSERT INTO settings(key,value) VALUES('online.youtube.enabled','false')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (_, disabled) = call(&state, "/UserViews", "GET", Value::Null, &alice).await;
    assert!(
        !disabled["Items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["Id"] == root_id)
    );
    assert_eq!(
        call(&state, &path, "GET", Value::Null, &alice).await.0,
        StatusCode::NOT_FOUND
    );
    let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("jellyfin-watchlists-result.json"), serde_json::to_vec_pretty(&json!({
        "singleWatchlist": {"videos": single["Items"].as_array().unwrap().iter().map(|item| &item["Name"]).collect::<Vec<_>>(), "total": single["TotalRecordCount"]},
        "multipleWatchlists": {"lists": multiple["Items"].as_array().unwrap().iter().map(|item| &item["Name"]).collect::<Vec<_>>(), "total": multiple["TotalRecordCount"]},
        "musicPlaylists": music["TotalRecordCount"],
        "recursiveVideos": recursive["TotalRecordCount"],
        "restoredVideos": restored["TotalRecordCount"],
        "perUserIsolation": true, "stableLibraryAndVideoIds": true, "providerDisableHonored": true
    })).unwrap()).unwrap();
}

#[tokio::test]
async fn youtube_genres_filter_videos_shorts_and_broadcasts() {
    let (_temp, state, _) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES('alice','channel','Channel','s',1)", [])?;
        for (video, title, broadcast, short) in [("aaaaaaaaaaa", "Video", "none", Some(0)), ("bbbbbbbbbbb", "Unchecked", "none", None), ("ccccccccccc", "Short", "none", Some(1)), ("ddddddddddd", "Live", "live", Some(0)), ("eeeeeeeeeee", "Replay", "replay", Some(1)), ("fffffffffff", "Upcoming", "upcoming", Some(0))] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration,broadcast,is_short) VALUES('alice',?1,'channel',?2,'public',120,?3,?4)", rusqlite::params![video,title,broadcast,short])?;
        }
        db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES('alice','t','t','Streamer','Title','Category',1,'2026-10-14T10:00:00Z','s',1)", [])?;
        Ok(())
    }).await.unwrap();
    let token = login(&state, "alice", DEVICE).await;
    let auth = [("X-Emby-Token", token.as_str())];
    let root = "d7df0f21caa645a09798267150600001";
    let (status, genres) = call(
        &state,
        &format!("/Genres?ParentId={root}"),
        "GET",
        Value::Null,
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let genre = |name: &str| {
        genres["Items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["Name"] == name)
            .unwrap()["Id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let names = |page: &Value| {
        let mut names = page["Items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["Name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    let (_, all) = call(
        &state,
        &format!("/Items?ParentId={root}"),
        "GET",
        Value::Null,
        &auth,
    )
    .await;
    let short = all["Items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["Name"] == "Short")
        .unwrap();
    assert_eq!(short["Genres"], json!(["Shorts"]));
    assert_eq!(short["GenreItems"][0]["Id"], genre("Shorts"));
    for (selected, expected) in [
        (vec!["Videos"], vec!["Unchecked", "Video"]),
        (vec!["Shorts"], vec!["Short"]),
        (vec!["Live", "Live replays"], vec!["Live", "Replay"]),
    ] {
        let ids = selected
            .iter()
            .map(|name| genre(name))
            .collect::<Vec<_>>()
            .join(",");
        let (status, page) = call(
            &state,
            &format!("/Items?ParentId={root}&GenreIds={ids}"),
            "GET",
            Value::Null,
            &auth,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(names(&page), expected, "{selected:?}");
        assert_eq!(page["TotalRecordCount"], expected.len());
    }
    let (_, twitch) = call(
        &state,
        "/Genres?ParentId=d7df0f21caa645a09798267150600002",
        "GET",
        Value::Null,
        &auth,
    )
    .await;
    assert_eq!(twitch["TotalRecordCount"], 0);
}

#[tokio::test]
async fn twitch_community_rating_sorts_by_viewers_and_describes_the_stream() {
    let (_temp, state, _) = fixture().await;
    state.db.write("test.fixture", |db| {
        for (channel, name, viewers) in [("a", "Charlie", 12_345), ("b", "Bravo", 1_250), ("c", "Alpha", 12_345), ("z", "Zulu", 20)] {
            db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES('alice',?1,?1,?2,'Building a factory','Satisfactory',?3,'2026-10-14T10:00:00Z','s',1)",rusqlite::params![channel,name,viewers])?;
        }
        Ok(())
    }).await.unwrap();
    let (status, login) = call(
        &state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":"alice","Pw":"a long test-only password"}),
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let auth = [("X-Emby-Token", login["AccessToken"].as_str().unwrap())];
    let (_, views) = call(&state, "/UserViews", "GET", Value::Null, &auth).await;
    let root = views["Items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["Name"] == "Twitch")
        .unwrap()["Id"]
        .as_str()
        .unwrap();
    for (sort, direction, expected) in [
        (
            "CommunityRating",
            "Descending",
            ["Charlie", "Alpha", "Bravo", "Zulu"],
        ),
        (
            "CommunityRating",
            "Ascending",
            ["Zulu", "Bravo", "Charlie", "Alpha"],
        ),
        (
            "CommunityRating,SortName",
            "Descending,Ascending",
            ["Alpha", "Charlie", "Bravo", "Zulu"],
        ),
        (
            "CommunityRating,SortName",
            "Ascending,Ascending",
            ["Zulu", "Bravo", "Alpha", "Charlie"],
        ),
        (
            "CommunityRating,SortName",
            "Descending&SortOrder=Ascending",
            ["Alpha", "Charlie", "Bravo", "Zulu"],
        ),
        (
            "CommunityRating,SortName",
            "Descending",
            ["Charlie", "Alpha", "Bravo", "Zulu"],
        ),
        (
            "CommunityRating,SortName",
            "Ascending",
            ["Zulu", "Bravo", "Alpha", "Charlie"],
        ),
        (
            "SortName,CommunityRating",
            "Ascending,Descending",
            ["Alpha", "Bravo", "Charlie", "Zulu"],
        ),
        (
            "SortName",
            "Ascending",
            ["Alpha", "Bravo", "Charlie", "Zulu"],
        ),
    ] {
        let mut names = Vec::new();
        for start in [0, 2] {
            let (status, page) = call(
                &state,
                &format!("/Items?ParentId={root}&IncludeItemTypes=Video&SortBy={sort}&SortOrder={direction}&StartIndex={start}&Limit=2"),
                "GET",
                Value::Null,
                &auth,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["TotalRecordCount"], 4);
            for item in page["Items"].as_array().unwrap() {
                if item["Name"] == "Alpha" {
                    assert_eq!(
                        item["Overview"],
                        "12.3k viewers · Satisfactory · Building a factory"
                    );
                }
                if item["Name"] == "Bravo" {
                    assert_eq!(
                        item["Overview"],
                        "1.3k viewers · Satisfactory · Building a factory"
                    );
                }
                names.push(item["Name"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(names, expected, "{sort} {direction}");
    }
    for (viewers, compact) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1k"),
        (999_950, "1m"),
        (1_000_000, "1m"),
    ] {
        state
            .db
            .write("test.fixture", move |db| {
                db.execute(
                    "UPDATE twitch_streams SET viewers=?1 WHERE user_id='alice' AND channel_id='c'",
                    [viewers],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (status, page) = call(
            &state,
            &format!("/Items?ParentId={root}&SearchTerm=Alpha"),
            "GET",
            Value::Null,
            &auth,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            page["Items"][0]["Overview"],
            format!("{compact} viewers · Satisfactory · Building a factory")
        );
    }
}

#[tokio::test]
async fn online_libraries_and_watchlists_are_private_stable_and_use_existing_settings() {
    let (_temp, state, first_party) = fixture().await;
    state.db.write("test.fixture", |db| {
        for user in ["alice","bob"] {
            db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES(?1,'channel','Channel','s',1)",[user])?;
            for video in ["aaaaaaaaaaa","bbbbbbbbbbb"] {
                db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration,published_at) VALUES(?1,?2,'channel',?2,'public',120,100)",rusqlite::params![user,video])?;
            }
            db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES(?1,'123','channel','Live channel','Live','Game',10,'2026-09-20T10:00:00Z','s',1)",[user])?;
            db.execute("INSERT INTO kick_channels(user_id,slug,generation,live,title) VALUES(?1,'channel','g',1,'Kick live')",[user])?;
        }
        Ok(())
    }).await.unwrap();
    let mut tokens = Vec::new();
    for user in ["alice", "bob"] {
        let (status, login) = call(
            &state,
            "/Users/AuthenticateByName",
            "POST",
            json!({"Username":user,"Pw":"a long test-only password"}),
            &[("Authorization", DEVICE)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        tokens.push(login["AccessToken"].as_str().unwrap().to_owned());
    }
    let auth = [("X-Emby-Token", tokens[0].as_str())];
    let bob = [("X-Emby-Token", tokens[1].as_str())];
    let views = call(&state, "/UserViews", "GET", Value::Null, &auth).await;
    assert_eq!(views.0, StatusCode::OK);
    let roots = views.1["Items"].as_array().unwrap();
    let mut video_id = String::new();
    for (name, count) in [("YouTube", 2), ("Twitch", 1), ("Kick", 1)] {
        let root = roots.iter().find(|v| v["Name"] == name).unwrap()["Id"]
            .as_str()
            .unwrap();
        let path = format!("/Items?ParentId={root}&IncludeItemTypes=Video&Limit=1");
        let (status, page) = call(&state, &path, "GET", Value::Null, &auth).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], count);
        let id = page["Items"][0]["Id"].as_str().unwrap().to_owned();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        assert_eq!(page["Items"][0]["LocationType"], "Remote");
        assert_eq!(page["Items"][0]["MediaSources"][0]["Id"], id);
        assert_eq!(
            page["Items"][0]["MediaSources"][0]["MediaStreams"][0]["IsInterlaced"],
            false
        );
        assert_eq!(
            call(&state, &path, "GET", Value::Null, &auth).await.1["Items"][0]["Id"],
            id
        );
        assert_eq!(
            call(&state, &format!("/Items/{id}"), "GET", Value::Null, &bob)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                &state,
                &format!("/Items/{id}/PlaybackInfo"),
                "POST",
                json!({}),
                &bob
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        if name == "YouTube" {
            video_id = id;
        }
    }
    let create = json!({"Name":"TV list","Ids":[video_id],"Users":[{"UserId":"alice","CanEdit":true}],"IsPublic":false});
    let (status, list) = call(&state, "/Playlists", "POST", create.clone(), &auth).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let list = list["Id"].as_str().unwrap();
    let foreign = call(
        &state,
        &format!("/Items?ParentId={list}&IncludeItemTypes=Video"),
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert!(
        foreign.0 == StatusCode::NOT_FOUND
            || foreign.1["Items"].as_array().is_some_and(Vec::is_empty)
    );
    // Wholphin edits membership, preserving the user's existing list policy.
    let list_key = list.to_owned();
    state.db.write("test.fixture", move|db|{db.execute("UPDATE youtube_watchlists SET auto_download=1,auto_remove_watched=1,sort_mode='date',sort_direction='desc' WHERE id=(SELECT watchlist_id FROM compat_online_items WHERE id=?1)",[list_key])?;Ok(())}).await.unwrap();
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Users/alice"),
            "GET",
            Value::Null,
            &auth
        )
        .await
        .1["CanEdit"],
        true
    );
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Users/alice"),
            "GET",
            Value::Null,
            &bob
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            &format!("/Items?ParentId={list}"),
            "GET",
            Value::Null,
            &auth
        )
        .await
        .1["Items"][0]["Id"],
        video_id
    );
    assert_eq!(
        call(
            &state,
            "/Items?IncludeItemTypes=Playlist&MediaTypes=Video&SearchTerm=TV",
            "GET",
            Value::Null,
            &auth
        )
        .await
        .1["TotalRecordCount"],
        1
    );
    assert_eq!(
        call(
            &state,
            "/Items?IncludeItemTypes=Playlist&MediaTypes=Video&SearchTerm=TV",
            "GET",
            Value::Null,
            &bob
        )
        .await
        .1["TotalRecordCount"],
        0
    );
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Items?EntryIds={video_id}"),
            "DELETE",
            Value::Null,
            &bob
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Items?EntryIds={video_id}"),
            "DELETE",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Items?Ids={video_id}"),
            "POST",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let bearer = format!("Bearer {first_party}");
    let web = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await;
    let saved = web
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "TV list")
        .unwrap();
    assert_eq!(saved["autoDownload"], true);
    assert_eq!(saved["autoRemoveWatched"], true);
    assert_eq!(saved["sortMode"], "date");
    assert_eq!(saved["sortDirection"], "desc");
    assert_eq!(saved["items"].as_array().unwrap().len(), 1);
    let second = call(
        &state,
        "/Items?IncludeItemTypes=Video&SearchTerm=bbbbbbbbbbb",
        "GET",
        Value::Null,
        &auth,
    )
    .await
    .1["Items"][0]["Id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        call(
            &state,
            &format!("/Playlists/{list}/Items?Ids={second}"),
            "POST",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let movement = format!("/Playlists/{list}/Items/{second}/Move/0");
    assert_eq!(
        call(&state, &movement, "POST", Value::Null, &bob).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&state, &movement, "POST", Value::Null, &auth).await.0,
        StatusCode::NO_CONTENT
    );
    let reordered = call(
        &state,
        &format!("/Items?ParentId={list}"),
        "GET",
        Value::Null,
        &auth,
    )
    .await
    .1;
    assert_eq!(reordered["Items"][0]["Id"], second);
    assert_eq!(reordered["Items"][1]["Id"], video_id);
    let key = list.to_owned();
    assert!(state.db.write("test.fixture", move|db|Ok(db.query_row("SELECT auto_download=1 AND auto_remove_watched=1 AND sort_mode='manual' AND sort_direction='asc' FROM youtube_watchlists WHERE id=(SELECT watchlist_id FROM compat_online_items WHERE id=?1)",[key],|r|r.get::<_,bool>(0))?)).await.unwrap());
    assert!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT COUNT(*) FROM events WHERE user_id='alice' AND kind='youtube.changed'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap()
            > 0
    );
    let mut public = create;
    public["IsPublic"] = json!(true);
    assert_eq!(
        call(&state, "/Playlists", "POST", public, &auth).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            &format!("/UserFavoriteItems/{video_id}"),
            "POST",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/UserPlayedItems/{video_id}"),
            "POST",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::OK
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute("DELETE FROM youtube_videos WHERE user_id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            &format!("/Items/{video_id}"),
            "GET",
            Value::Null,
            &auth
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(state.db.write("test.fixture", |db|Ok(db.query_row("SELECT COUNT(*) FROM compat_online_items WHERE user_id='alice' AND youtube_video_id IS NOT NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
}

#[tokio::test]
async fn diagnostics_exclude_compatibility_credentials_and_untrusted_errors() {
    let (_temp, state, first_party) = fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, login) = call(
        &state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":"alice","Pw":"a long test-only password"}),
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = login["AccessToken"].as_str().unwrap();
    let query = format!("/Users/Me?api_key={token}");
    assert_eq!(
        call(&state, &query, "GET", Value::Null, &[]).await.0,
        StatusCode::OK
    );
    let leaked_error = format!("untrusted-error-sentinel: {query}");
    state.db.write("test.fixture", move |db| {
        db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,error,created_at) VALUES ('redaction','fixture','redaction','{}','failed',1,?1,1)", [&leaked_error])?;
        db.execute("INSERT INTO audit(action,target,created_at) VALUES ('fixture',?1,1)", [&leaked_error])?;
        Ok(())
    }).await.unwrap();
    let bearer = format!("Bearer {first_party}");
    let (status, bundle) = call(
        &state,
        "/api/v1/admin/diagnostics",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bundle["database_ok"], true);
    assert_eq!(bundle["jobs"], json!([{"state":"failed","count":1}]));
    let serialized = bundle.to_string();
    for secret in [
        token,
        first_party.as_str(),
        "untrusted-error-sentinel",
        "api_key",
        "a long test-only password",
    ] {
        assert!(!serialized.contains(secret));
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/diagnostics",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn catalog_paging_playlist_order_and_private_state_match_first_party() {
    let (_temp, state, first_party) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/music')",[])?;
        for (id,kind,parent) in [("artist","artist",None),("album","album",Some("artist")),("one","track",Some("album")),("two","track",Some("album"))] {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'music',?2,?3,?1,?1,1)",rusqlite::params![id,kind,parent])?;
        }
        db.execute("INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES ('mix','alice','Shared mix',1,1)",[])?;
        for (position,media) in ["two","one","two"].iter().enumerate() {
            db.execute("INSERT INTO playlist_items VALUES ('mix',?1,?2)",rusqlite::params![position as i64,media])?;
        }
        Ok(())
    }).await.unwrap();
    let mut tokens = Vec::new();
    for name in ["alice", "bob"] {
        let (status, login) = call(
            &state,
            "/Users/AuthenticateByName",
            "POST",
            json!({"Username":name,"Pw":"a long test-only password"}),
            &[("Authorization", DEVICE)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        tokens.push(login["AccessToken"].as_str().unwrap().to_owned());
    }
    let alice = [("X-Emby-Token", tokens[0].as_str())];
    let bob = [("X-Emby-Token", tokens[1].as_str())];
    for query in [
        "IsFolder=false&Limit=0",
        "IncludeItemTypes=Audio&StartIndex=100",
        "ArtistIds=artist&IsFolder=false&Limit=0",
    ] {
        let (status, page) = call(
            &state,
            &format!("/Items?{query}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 2);
        assert_eq!(page["Items"], json!([]));
    }
    for path in ["/Playlists/mix/Items", "/Items?ParentId=mix"] {
        let (status, page) = call(&state, path, "GET", Value::Null, &bob).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 3);
        let rows = page["Items"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|v| v["Id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["two", "one", "two"]
        );
        assert_ne!(rows[0]["PlaylistItemId"], rows[2]["PlaylistItemId"]);
    }
    let (status, page) = call(
        &state,
        "/Playlists/mix/Items?StartIndex=2&Limit=1",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["Items"].as_array().unwrap().len(), 1);
    assert_eq!(page["Items"][0]["Id"], "two");
    for id in ["mix", "one"] {
        let (status, data) = call(
            &state,
            &format!("/UserFavoriteItems/{id}"),
            "POST",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{data}");
        assert_eq!(data["IsFavorite"], true);
        let (_, other) = call(&state, &format!("/Items/{id}"), "GET", Value::Null, &bob).await;
        assert_eq!(other["UserData"]["IsFavorite"], false);
    }
    let (status, data) = call(&state, "/UserPlayedItems/one", "POST", Value::Null, &alice).await;
    assert_eq!(status, StatusCode::OK, "{data}");
    assert_eq!(data["Played"], true);
    for (headers, total) in [(&alice, 1), (&bob, 0)] {
        let (status, page) = call(
            &state,
            "/Items?IncludeItemTypes=Playlist&Filters=IsFavorite&StartIndex=100",
            "GET",
            Value::Null,
            headers,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["TotalRecordCount"], total);
        assert_eq!(page["Items"], json!([]));
    }
    state
        .db
        .write("test.fixture", |db| {
            for index in 0..100 {
                db.execute(
                    "INSERT INTO compat_preferences VALUES ('alice','test',?1,'{}')",
                    [index.to_string()],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/DisplayPreferences/0?client=test",
            "POST",
            json!({"SortBy":"DateCreated"}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/DisplayPreferences/new?client=test",
            "POST",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, saved) = call(
        &state,
        "/api/v1/catalog/one/state",
        "GET",
        Value::Null,
        &[("Authorization", &format!("Bearer {first_party}"))],
    )
    .await;
    assert_eq!(saved["favorite"], true);
    assert_eq!(saved["watched"], true);
    assert_eq!(
        call(
            &state,
            "/Users/bob/FavoriteItems/one",
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, data) = call(
        &state,
        "/UserFavoriteItems/one",
        "DELETE",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["IsFavorite"], false);
}
#[tokio::test]
async fn compatibility_credentials_are_private_revocable_and_transport_scoped() {
    let (_temp, state, first_party) = fixture().await;
    let (status, login) = call(
        &state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":"alice","Pw":"a long test-only password"}),
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token = login["AccessToken"].as_str().unwrap();
    for header in ["X-Emby-Token", "X-MediaBrowser-Token"] {
        assert_eq!(
            call(&state, "/Users/Me", "GET", Value::Null, &[(header, token)])
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}&ApiKey={}", "f".repeat(64)),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}&UserId=bob"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/Users/bob/Items",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let bearer = format!("Bearer {token}");
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/Users/Me",
            "GET",
            Value::Null,
            &[("X-Emby-Token", &first_party)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let session = login["SessionInfo"]["Id"].as_str().unwrap();
    let bearer = format!("Bearer {first_party}");
    let sessions = call(
        &state,
        "/api/v1/auth/sessions",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await
    .1;
    assert!(
        sessions["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session && s["name"].as_str().unwrap().contains("Test client"))
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/auth/sessions/{session}"),
            "DELETE",
            Value::Null,
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/Users/Me",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn quick_connect_requires_review_and_is_single_use() {
    let (_temp, state, first_party) = fixture().await;
    let (status, pending) = call(
        &state,
        "/QuickConnect/Initiate",
        "POST",
        Value::Null,
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    let secret = pending["Secret"].as_str().unwrap();
    let code = pending["Code"].clone();
    assert_eq!(
        call(
            &state,
            "/Users/AuthenticateWithQuickConnect",
            "POST",
            json!({"Secret":secret}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let bearer = format!("Bearer {first_party}");
    let (status, review) = call(
        &state,
        "/api/v1/auth/quick-connect/inspect",
        "POST",
        json!({"code":code}),
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(review["device"], "Living room, TV");
    assert_eq!(review["client"], "Test client");
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code,"confirmation":review["confirmation"]}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/QuickConnect/Connect?secret={secret}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .1["Authenticated"],
        true
    );
    let (status, session) = call(
        &state,
        "/Users/AuthenticateWithQuickConnect",
        "POST",
        json!({"Secret":secret}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["User"]["Name"], "alice");
    assert_eq!(
        call(
            &state,
            "/Users/AuthenticateWithQuickConnect",
            "POST",
            json!({"Secret":secret}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code,"confirmation":review["confirmation"]}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        // A used approval is gone, not a reason to sign the approving browser out.
        StatusCode::GONE
    );
}

#[tokio::test]
async fn downloaded_watchlist_video_plays_through_jellyfin_urls_and_stops() {
    let (temp, state, first_party) = fixture().await;
    let (movie, _) = local_library(temp.path(), &state).await;
    let video = "download001";
    let root = state.config.cache.join("youtube");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("download.mkv");
    std::fs::copy(movie, &path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut probe: Value = serde_json::from_slice(&run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
            path.to_str().unwrap(),
        ],
    ))
    .unwrap();
    probe["chapters"] =
        json!([{"start_time":"0.000000","tags":{"title":"Intro"}},{"start_time":"9.000000"}]);
    let probe = probe.to_string();
    let metadata = std::fs::metadata(&path).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let bearer = format!("Bearer {first_party}");
    let (status, lists) = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = lists[0]["id"].as_i64().unwrap();
    state.db.write("test.fixture", move |db| {
        for user in ["alice", "bob"] {
            db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES(?1,'channel','Channel','s',1)", [user])?;
            db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration) VALUES(?1,?2,'channel','Downloaded video','public',18)", rusqlite::params![user,video])?;
        }
        db.execute("INSERT INTO youtube_watchlist_items(user_id,watchlist_id,video_id,manual_position,added_at) VALUES('alice',?1,?2,0,1)", rusqlite::params![list,video])?;
        db.execute("INSERT INTO youtube_media(video_id) VALUES(?1)", [video])?;
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,path,size,modified,probe,requested_at,updated_at) VALUES(?1,'download-generation','ready','{}',?2,?3,?4,?5,1,1)", rusqlite::params![video,path.to_string_lossy(),metadata.len() as i64,modified,probe])?;
        Ok(())
    }).await.unwrap();
    let alice_token = login(&state, "alice", DEVICE).await;
    let second_token = login(&state, "alice", &DEVICE.replace("tv-one", "tv-two")).await;
    let bob_token = login(&state, "bob", DEVICE).await;
    let alice = [("X-Emby-Token", alice_token.as_str())];
    let bob = [("X-Emby-Token", bob_token.as_str())];
    let second = [("X-Emby-Token", second_token.as_str())];
    let (status, page) = call(
        &state,
        "/Items?ParentId=d7df0f21-caa6-45a0-9798-267150600004",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let item = page["Items"][0]["Id"].as_str().unwrap();
    let (_, page) = call(
        &state,
        "/Items?IncludeItemTypes=Video",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    let bob_item = page["Items"][0]["Id"].as_str().unwrap();
    assert_ne!(item, bob_item);
    // The downloaded copy names its chapters in the item's details.
    let (status, details) = call(
        &state,
        &format!("/Items/{item}"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{details}");
    assert_eq!(
        details["Chapters"],
        json!([
            {"StartPositionTicks":0,"Name":"Intro","ImageDateModified":"0001-01-01T00:00:00Z"},
            {"StartPositionTicks":90_000_000,"Name":null,"ImageDateModified":"0001-01-01T00:00:00Z"}
        ])
    );
    let info = format!("/Items/{item}/PlaybackInfo");
    let (status, session) = call(&state, &info, "POST", json!({"MediaSourceId":item,"AudioStreamIndex":1,"DeviceProfile":{
        "DirectPlayProfiles":[{"Type":"Video","Container":"mkv","VideoCodec":"h264","AudioCodec":"aac"}]
    }}), &alice).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let source = &session["MediaSources"][0];
    assert_eq!(source["SupportsDirectPlay"], true);
    let sid = session["PlaySessionId"].as_str().unwrap();
    let file = source["Id"].as_str().unwrap();
    let tag = source["ETag"].as_str().unwrap();
    let stream =
        format!("/Videos/{item}/stream?Static=true&MediaSourceId={file}&PlaySessionId={sid}");
    // Wholphin builds this URL instead of using DirectStreamUrl, for both players.
    let tagged = format!("{stream}&Tag={tag}");
    let (status, _, body) = send(&state, &tagged, "GET", Value::Null, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body.as_ref(), original);
    let (status, headers, body) = send(
        &state,
        &stream,
        "GET",
        Value::Null,
        &[alice[0], ("Range", "bytes=0-1023")],
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 0-1023/{}", original.len())
    );
    assert_eq!(body.as_ref(), &original[..1024]);
    assert_eq!(
        send(&state, &tagged, "HEAD", Value::Null, &[]).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &state,
            &tagged.replace(item, &item.replace('-', "")),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    for auth in [&bob, &second] {
        assert_eq!(
            send(&state, &stream, "GET", Value::Null, auth).await.0,
            StatusCode::NOT_FOUND
        );
    }
    for invalid in [
        tagged.replace(item, bob_item),
        tagged.replace(file, "other000001"),
        tagged.replace(sid, "00000000-0000-0000-0000-000000000000"),
    ] {
        assert_eq!(
            send(&state, &invalid, "GET", Value::Null, &[]).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Progress",
            "POST",
            json!({"ItemId":item,"PlaySessionId":sid,"PositionTicks":40_000_000}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(position(&state, item, &alice).await, 40_000_000);
    let stop = format!("/Videos/ActiveEncodings?PlaySessionId={sid}&DeviceId=tv-one");
    for auth in [&bob, &second] {
        assert_eq!(
            call(&state, &stop, "DELETE", Value::Null, auth).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        call(&state, &stop, "DELETE", Value::Null, &alice).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, sid).await.1, "stopped");
    assert_eq!(
        send(&state, &tagged, "GET", Value::Null, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn local_video_plays_through_each_negotiated_delivery_for_its_own_device_session() {
    let (temp, state, _) = fixture().await;
    let (movie_path, _) = local_library(temp.path(), &state).await;
    let alice_token = login(&state, "alice", DEVICE).await;
    let second_token = login(&state, "alice", &DEVICE.replace("tv-one", "tv-two")).await;
    let bob_token = login(&state, "bob", DEVICE).await;
    let alice = [("X-Emby-Token", alice_token.as_str())];
    let second = [("X-Emby-Token", second_token.as_str())];
    let bob = [("X-Emby-Token", bob_token.as_str())];
    let mut ids = Vec::new();
    for kind in ["Movie", "Audio"] {
        let (status, page) = call(
            &state,
            &format!("/Items?IncludeItemTypes={kind}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 1);
        ids.push(page["Items"][0]["Id"].as_str().unwrap().to_owned());
    }
    let (movie, track) = (&ids[0], &ids[1]);
    let (status, item) = call(
        &state,
        &format!("/Items/{movie}"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{item}");
    let source = &item["MediaSources"][0];
    let file = source["Id"].as_str().unwrap().to_owned();
    let generation = source["ETag"].as_str().unwrap().to_owned();
    assert_eq!(source["Container"], "mkv");
    assert_eq!(
        source["MediaStreams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| format!(
                "{} {} {}",
                s["Type"].as_str().unwrap(),
                s["Index"],
                s["Codec"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        ["Video 0 h264", "Audio 1 aac", "Audio 2 ac3"]
    );
    // The client plays H.264/AAC in `container` files and as HLS.
    let request = |audio: i64, container: &str| {
        json!({"MediaSourceId":file,"AudioStreamIndex":audio,"DeviceProfile":{
            "DirectPlayProfiles":[{"Type":"Video","Container":container,"VideoCodec":"h264","AudioCodec":"aac"}],
            "TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}]}})
    };
    let info = format!("/Items/{movie}/PlaybackInfo");

    // Direct play: the original file through its Tag grant or the device login.
    let (status, direct) = call(&state, &info, "POST", request(1, "mkv"), &alice).await;
    assert_eq!(status, StatusCode::OK, "{direct}");
    let direct_id = direct["PlaySessionId"].as_str().unwrap().to_owned();
    let offer = &direct["MediaSources"][0];
    let tag = offer["ETag"].as_str().unwrap().to_owned();
    assert_eq!(offer["Id"], file);
    assert_eq!(offer["SupportsDirectPlay"], true);
    assert_eq!(offer["SupportsDirectStream"], false);
    assert_eq!(offer["SupportsTranscoding"], false);
    assert!(offer["TranscodingUrl"].is_null());
    assert_eq!(offer["DefaultAudioStreamIndex"], 1);
    assert_eq!(
        offer["DirectStreamUrl"],
        format!("/api/v1/playback/{direct_id}/stream?grant={tag}")
    );
    assert_eq!(
        playback_state(&state, &direct_id).await,
        ("direct".into(), "ready".into())
    );
    let original = std::fs::read(&movie_path).unwrap();
    let tagged = format!("/Videos/{movie}/stream?static=true&MediaSourceId={file}&Tag={tag}");
    let (status, headers, body) = send(&state, &tagged, "GET", Value::Null, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "video/x-matroska");
    assert!(body[..] == original[..]);
    let stream = format!("/Videos/{movie}/stream?PlaySessionId={direct_id}&MediaSourceId={file}");
    let issued = grants(&state).await;
    let (status, headers, body) = send(
        &state,
        &stream,
        "GET",
        Value::Null,
        &[alice[0], ("Range", "bytes=0-1023")],
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 0-1023/{}", original.len()).as_str()
    );
    assert!(body[..] == original[..1024]);
    // Range requests are served on the play session; none mints a grant on the writer.
    assert_eq!(grants(&state).await, issued);
    for other in [&bob, &second] {
        assert_eq!(
            send(&state, &stream, "GET", Value::Null, other).await.0,
            StatusCode::NOT_FOUND
        );
    }
    // A Tag names one item's direct play session; anything else needs a login.
    assert_eq!(
        send(
            &state,
            &format!("/Videos/{track}/stream?Tag={tag}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    // Reports update progress, and only for the device session that opened it.
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing",
            "POST",
            json!({"ItemId":movie,"MediaSourceId":file,"PlaySessionId":direct_id,"PositionTicks":0}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &direct_id).await.1, "playing");
    // Clients also send identifiers without hyphens.
    let progress = json!({"ItemId":movie.replace('-', ""),"PlaySessionId":direct_id.replace('-', ""),"PositionTicks":40_000_000,"IsPaused":true});
    for other in [&bob, &second] {
        assert_eq!(
            call(
                &state,
                "/Sessions/Playing/Progress",
                "POST",
                progress.clone(),
                other
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(position(&state, movie, &alice).await, 0);
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Progress",
            "POST",
            progress,
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &direct_id).await.1, "paused");
    assert_eq!(position(&state, movie, &alice).await, 40_000_000);
    assert_eq!(position(&state, movie, &bob).await, 0);
    for other in [&bob, &second] {
        assert_eq!(
            call(
                &state,
                "/Sessions/Playing/Stopped",
                "POST",
                json!({"ItemId":movie,"PlaySessionId":direct_id,"PositionTicks":10_000_000}),
                other
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Stopped",
            "POST",
            json!({"ItemId":movie,"PlaySessionId":direct_id,"PositionTicks":70_000_000}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &direct_id).await.1, "stopped");
    assert_eq!(position(&state, movie, &alice).await, 70_000_000);
    // A late report is acknowledged, but it cannot reopen or move a stopped session.
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Progress",
            "POST",
            json!({"ItemId":movie,"PlaySessionId":direct_id,"PositionTicks":90_000_000}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &direct_id).await.1, "stopped");
    assert_eq!(position(&state, movie, &alice).await, 70_000_000);
    assert_eq!(
        send(&state, &tagged, "GET", Value::Null, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Remux: the client rejects Matroska, so the copied H.264/AAC packets
    // move into an HLS VOD timeline cut at the source keyframes.
    let (status, remux) = call(&state, &info, "POST", request(1, "mp4"), &alice).await;
    assert_eq!(status, StatusCode::OK, "{remux}");
    let remux_id = remux["PlaySessionId"].as_str().unwrap().to_owned();
    let offer = &remux["MediaSources"][0];
    let url = offer["TranscodingUrl"].as_str().unwrap().to_owned();
    assert_eq!(offer["SupportsDirectPlay"], false);
    assert_eq!(offer["SupportsDirectStream"], false);
    assert_eq!(offer["SupportsTranscoding"], true);
    assert_eq!(offer["TranscodingSubProtocol"], "hls");
    assert_eq!(offer["TranscodingContainer"], "ts");
    assert_eq!(offer["DirectStreamUrl"], url);
    assert!(
        url.starts_with(&format!("/api/v1/playback/{remux_id}/hls/"))
            && url.contains("/index.m3u8?grant="),
        "{url}"
    );
    assert_eq!(
        playback_state(&state, &remux_id).await,
        ("remux".into(), "ready".into())
    );
    // The original stream route and Tag serve only direct play sessions.
    assert_eq!(
        send(
            &state,
            &format!("/Videos/{movie}/stream?PlaySessionId={remux_id}"),
            "GET",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let remux_tag = offer["ETag"].as_str().unwrap();
    assert_eq!(
        send(
            &state,
            &format!("/Videos/{movie}/stream?Tag={remux_tag}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (durations, segment) = vod(&state, &url, &temp.path().join("remux.ts")).await;
    // Copied video is cut at its keyframes, five seconds apart, rather than on
    // the six-second grid of a video conversion. The first interval also
    // covers the stream's start offset.
    assert_eq!(durations.len(), 4, "{durations:?}");
    assert_eq!(durations[1..3], ["5.000000,"; 2], "{durations:?}");
    assert_eq!(segment, ["h264 160x90", "aac 1ch"]);
    for (headers, device) in [(&bob, "tv-one"), (&second, "tv-two"), (&alice, "tv-two")] {
        assert_eq!(
            call(
                &state,
                &format!("/Videos/ActiveEncodings?DeviceId={device}&PlaySessionId={remux_id}"),
                "DELETE",
                Value::Null,
                headers
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(playback_state(&state, &remux_id).await.1, "ready");
    assert_eq!(
        call(
            &state,
            &format!("/Videos/ActiveEncodings?DeviceId=tv-one&PlaySessionId={remux_id}"),
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &remux_id).await.1, "stopped");
    assert_eq!(
        send(&state, &url, "GET", Value::Null, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Audio conversion: the client rejects AC-3, so the video is still copied
    // on keyframe boundaries while the audio becomes stereo AAC.
    let (status, converted) = call(&state, &info, "POST", request(2, "mp4"), &alice).await;
    assert_eq!(status, StatusCode::OK, "{converted}");
    let converted_id = converted["PlaySessionId"].as_str().unwrap().to_owned();
    let offer = &converted["MediaSources"][0];
    assert_eq!(offer["SupportsDirectPlay"], false);
    assert_eq!(offer["SupportsTranscoding"], true);
    assert_eq!(offer["DefaultAudioStreamIndex"], 2);
    assert_eq!(
        playback_state(&state, &converted_id).await,
        ("transcode".into(), "ready".into())
    );
    let url = offer["TranscodingUrl"].as_str().unwrap();
    let (durations, segment) = vod(&state, url, &temp.path().join("converted.ts")).await;
    assert_eq!(durations.len(), 4, "{durations:?}");
    assert_eq!(durations[1..3], ["5.000000,"; 2], "{durations:?}");
    assert_eq!(segment, ["h264 160x90", "aac 2ch"]);
    assert_eq!(
        call(
            &state,
            &format!("/Videos/ActiveEncodings?PlaySessionId={converted_id}"),
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(playback_state(&state, &converted_id).await.1, "stopped");

    // Media segments of the file's current generation.
    let (media, key) = (movie.clone(), file.clone());
    state
        .db
        .write("test.fixture", move |db| {
            for (id, kind, start, end) in [
                ("intro", "Intro", 1.0, 4.0),
                ("credits", "Credits", 15.0, 17.5),
            ] {
                db.execute(
                    "INSERT INTO media_segments VALUES (?1,?2,?3,?4,?5,?6,?7,'manual',1,1)",
                    rusqlite::params![id, media, key, generation, kind, start, end],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let intro = json!({"Id":"intro","ItemId":movie,"Type":"Intro","StartTicks":10_000_000,"EndTicks":40_000_000});
    let credits = json!({"Id":"credits","ItemId":movie,"Type":"Credits","StartTicks":150_000_000,"EndTicks":175_000_000});
    let (status, segments) = call(
        &state,
        &format!("/MediaSegments/{movie}"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{segments}");
    assert_eq!(
        segments,
        json!({"Items":[intro,credits],"TotalRecordCount":2,"StartIndex":0})
    );
    let (_, filtered) = call(
        &state,
        &format!("/MediaSegments/{movie}?IncludeSegmentTypes=Credits"),
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(filtered["Items"], json!([credits]));
}

#[tokio::test]
async fn direct_play_leaves_embedded_subtitles_to_clients_that_read_them() {
    let (temp, state, _) = fixture().await;
    let (feature, _) = local_library(temp.path(), &state).await;
    let caption = temp.path().join("caption.srt");
    std::fs::write(&caption, "1\n00:00:01,000 --> 00:00:03,000\nBonjour\n").unwrap();
    let subtitled = feature.with_file_name("Subtitled (2022).mkv");
    run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            feature.to_str().unwrap(),
            "-i",
            caption.to_str().unwrap(),
            "-map",
            "0",
            "-map",
            "1",
            "-c",
            "copy",
            "-c:s",
            "srt",
            "-metadata:s:s:0",
            "language=fre",
            subtitled.to_str().unwrap(),
        ],
    );
    for root in thelxinoe_catalog::roots(&state.db).await.unwrap() {
        thelxinoe_catalog::scan(&state.db, root, state.tools.runtime.media().unwrap())
            .await
            .unwrap();
    }
    let token = login(&state, "alice", DEVICE).await;
    let alice = [("X-Emby-Token", token.as_str())];
    let (_, page) = call(
        &state,
        "/Items?IncludeItemTypes=Movie",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    let movie = page["Items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["Name"] == "Subtitled")
        .unwrap()["Id"]
        .as_str()
        .unwrap()
        .to_owned();
    let info = format!("/Items/{movie}/PlaybackInfo");
    // Wholphin's ExoPlayer reads SubRip from the file or as a separate download.
    let request = |container: &str, methods: &[&str]| {
        let subtitles = methods
            .iter()
            .flat_map(|method| {
                ["srt", "subrip"].map(|format| json!({"Format":format,"Method":method}))
            })
            .collect::<Vec<_>>();
        json!({"SubtitleStreamIndex":3,"DeviceProfile":{
            "DirectPlayProfiles":[{"Type":"Video","Container":container,"VideoCodec":"h264","AudioCodec":"aac,ac3"}],
            "TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}],
            "SubtitleProfiles":subtitles}})
    };
    let subtitle = |session: &Value| {
        let offer = &session["MediaSources"][0];
        let stream = offer["MediaStreams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["Type"] == "Subtitle")
            .unwrap()
            .clone();
        (offer["SupportsDirectPlay"].clone(), stream)
    };

    // The player selects the track inside the file it plays; nothing is extracted.
    let (status, direct) = call(
        &state,
        &info,
        "POST",
        request("mkv", &["Embed", "External"]),
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{direct}");
    let (direct_play, stream) = subtitle(&direct);
    assert_eq!(direct_play, true);
    assert_eq!(
        (
            &stream["Index"],
            &stream["Codec"],
            &stream["DeliveryMethod"],
            &stream["IsExternal"]
        ),
        (&json!(3), &json!("subrip"), &json!("Embed"), &json!(false))
    );
    assert!(stream["DeliveryUrl"].is_null(), "{stream}");

    // A client that only loads separate subtitle files gets the extracted track.
    let (status, external) =
        call(&state, &info, "POST", request("mkv", &["External"]), &alice).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let (direct_play, stream) = subtitle(&external);
    assert_eq!(direct_play, true);
    assert_eq!(
        (
            &stream["Codec"],
            &stream["DeliveryMethod"],
            &stream["IsExternal"]
        ),
        (&json!("srt"), &json!("External"), &json!(true))
    );
    let (status, _, body) = send(
        &state,
        stream["DeliveryUrl"].as_str().unwrap(),
        "GET",
        Value::Null,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("Bonjour"));

    // HLS carries no subtitle track, so a converted stream keeps External delivery.
    let (status, remux) = call(
        &state,
        &info,
        "POST",
        request("mp4", &["Embed", "External"]),
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{remux}");
    let (direct_play, stream) = subtitle(&remux);
    assert_eq!(direct_play, false);
    assert_eq!(stream["DeliveryMethod"], "External");
}

#[tokio::test]
async fn universal_audio_streams_a_local_track_and_reports_progress_per_device_session() {
    let (temp, state, _) = fixture().await;
    let (_, track_path) = local_library(temp.path(), &state).await;
    let alice_token = login(&state, "alice", DEVICE).await;
    let second_token = login(&state, "alice", &DEVICE.replace("tv-one", "tv-two")).await;
    let bob_token = login(&state, "bob", DEVICE).await;
    // Audio players often send their token inside the MediaBrowser header.
    let authorization = format!("{DEVICE}, Token=\"{alice_token}\"");
    let alice = [("Authorization", authorization.as_str())];
    let second = [("X-Emby-Token", second_token.as_str())];
    let bob = [("X-Emby-Token", bob_token.as_str())];
    let mut ids = Vec::new();
    for kind in ["Audio", "Movie"] {
        let (status, page) = call(
            &state,
            &format!("/Items?IncludeItemTypes={kind}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 1);
        ids.push(page["Items"][0]["Id"].as_str().unwrap().to_owned());
    }
    let (track, movie) = (&ids[0], &ids[1]);
    let universal = |item: &str, container: &str| {
        format!(
            "/Audio/{item}/universal?Container={container}&MaxStreamingBitrate=140000000&TranscodingContainer=ts&TranscodingProtocol=hls&AudioCodec=aac"
        )
    };
    let supported = universal(track, "mp3,m4a%7Caac,flac");
    let original = std::fs::read(&track_path).unwrap();
    let (status, headers, body) = send(&state, &supported, "GET", Value::Null, &alice).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "audio/mp4");
    assert!(body[..] == original[..]);
    // A range request continues this device's playback of the track.
    let (status, _, body) = send(
        &state,
        &supported,
        "GET",
        Value::Null,
        &[alice[0], ("Range", "bytes=0-15")],
    )
    .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert!(body[..] == original[..16]);
    let playback = playbacks(&state, "alice", track).await;
    assert_eq!(playback.len(), 1);
    let playback = playback[0].0.clone();
    // Conversion and video require PlaybackInfo.
    for path in [universal(track, "mp3"), universal(movie, "mkv,m4a")] {
        assert_eq!(
            send(&state, &path, "GET", Value::Null, &alice).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let stream = format!("/Audio/{track}/stream?PlaySessionId={playback}");
    for other in [&bob, &second] {
        assert_eq!(
            send(&state, &stream, "GET", Value::Null, other).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send(&state, &stream, "GET", Value::Null, &alice).await.0,
        StatusCode::OK
    );

    // Reports without a PlaySessionId name this device session's playback.
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing",
            "POST",
            json!({"ItemId":track,"PositionTicks":0}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let progress = json!({"ItemId":track,"PositionTicks":30_000_000});
    for other in [&bob, &second] {
        assert_eq!(
            call(
                &state,
                "/Sessions/Playing/Progress",
                "POST",
                progress.clone(),
                other
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                &state,
                "/Sessions/Playing/Progress",
                "POST",
                json!({"ItemId":track,"PlaySessionId":playback,"PositionTicks":60_000_000}),
                other
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Progress",
            "POST",
            progress,
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(position(&state, track, &alice).await, 30_000_000);
    // Another user's universal stream is a separate playback with its own progress.
    assert_eq!(
        send(&state, &supported, "GET", Value::Null, &bob).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Progress",
            "POST",
            json!({"ItemId":track,"PositionTicks":90_000_000}),
            &bob
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(position(&state, track, &bob).await, 90_000_000);
    assert_eq!(position(&state, track, &alice).await, 30_000_000);
    assert_eq!(
        call(
            &state,
            "/Sessions/Playing/Stopped",
            "POST",
            json!({"ItemId":track,"PositionTicks":50_000_000}),
            &alice
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(position(&state, track, &alice).await, 50_000_000);
    assert_eq!(
        playbacks(&state, "alice", track).await,
        [(playback.clone(), "stopped".to_owned())]
    );
    // The next universal request starts a new playback of the stopped track.
    assert_eq!(
        send(&state, &supported, "GET", Value::Null, &alice).await.0,
        StatusCode::OK
    );
    let after = playbacks(&state, "alice", track).await;
    assert_eq!(after.len(), 2);
    assert_ne!(after[1].0, playback);
    assert_eq!(after[1].1, "ready");
}
#[tokio::test]
async fn catalog_items_carry_manager_metadata_chapters_and_unwatched_counts() {
    let (_temp, state, _) = fixture().await;
    let probe = json!({
        "streams":[{"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080}],
        "format":{"duration":"5400.0"},
        "chapters":[
            {"id":0,"start_time":"0.000000","end_time":"600.000000","tags":{"title":"Opening"}},
            {"id":1,"start_time":"600.500000","end_time":"5400.000000"}
        ]
    })
    .to_string();
    state.db.write("test.fixture", move |db| {
        db.execute_batch(r#"
INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies','/media/movies'),('shows','Shows','shows','/media/shows');
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,metadata,created_at) VALUES
 ('movie','movies','movie',NULL,'m','Feature',NULL,'{"title":"Feature","vote_average":6.665,"critic_rating":30,"certification":"PG-13","studio":"Walt Disney Pictures","tmdb_id":166426,"imdb_id":"tt1790809","release_date":"2017-05-25T00:00:00Z"}',1),
 ('show','shows','show',NULL,'s','Show',NULL,'{"title":"Show","vote_average":8.4,"certification":"TV-MA","studio":"HBO","tvdb_id":201,"tmdb_id":301,"first_air_date":"2016-10-02T00:00:00Z"}',1),
 ('season1','shows','season','show','s1','Season 1',1,'{}',1),
 ('specials','shows','season','show','s0','Specials',0,'{}',1),
 ('e1','shows','episode','season1','e1','Pilot',1,'{"air_date":"2015-11-20","air_date_utc":"2015-11-20T08:00:00Z","tvdb_id":5311261}',1),
 ('e2','shows','episode','season1','e2','Second',2,'{"air_date":"2015-11-27"}',1),
 ('e3','shows','episode','season1','e3','Not downloaded',3,'{}',1),
 ('sp1','shows','episode','specials','sp1','Special',1,'{}',1);
INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','e1',1,1);
"#)?;
        for (file, media, root) in [("fm","movie","movies"),("f1","e1","shows"),("f2","e2","shows"),("fs","sp1","shows")] {
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,?2,?1,'g',1,'m',?1,?3,1)", rusqlite::params![file, root, probe])?;
            db.execute("INSERT INTO media_sources VALUES (?1,?2)", [media, file])?;
        }
        Ok(())
    }).await.unwrap();
    let alice_token = login(&state, "alice", DEVICE).await;
    let bob_token = login(&state, "bob", DEVICE).await;
    let items = |query: String, token: String| {
        let state = state.clone();
        async move {
            let (status, page) = call(
                &state,
                &format!("/Items?{query}"),
                "GET",
                Value::Null,
                &[("X-Emby-Token", token.as_str())],
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{page}");
            page["Items"].as_array().unwrap().clone()
        }
    };
    let alice = |query: &str| items(query.to_owned(), alice_token.clone());

    let movie = alice("Ids=movie").await.remove(0);
    assert_eq!(movie["CommunityRating"], 6.7);
    assert_eq!(movie["CriticRating"], 30.0);
    assert_eq!(movie["OfficialRating"], "PG-13");
    assert_eq!(movie["PremiereDate"], "2017-05-25T00:00:00Z");
    assert_eq!(
        movie["ProviderIds"],
        json!({"Tmdb":"166426","Imdb":"tt1790809"})
    );
    assert_eq!(movie["Studios"][0]["Name"], "Walt Disney Pictures");
    // A video without artwork offers a still from its file.
    assert_eq!(movie["ImageTags"]["Primary"].as_str().unwrap().len(), 64);
    // Lists include chapters only when asked, like Jellyfin.
    assert!(movie.get("Chapters").is_none());
    let movie = alice("Ids=movie&Fields=Chapters,Overview").await.remove(0);
    assert_eq!(
        movie["Chapters"],
        json!([
            {"StartPositionTicks":0,"Name":"Opening","ImageDateModified":"0001-01-01T00:00:00Z"},
            {"StartPositionTicks":6_005_000_000_i64,"Name":null,"ImageDateModified":"0001-01-01T00:00:00Z"}
        ])
    );
    let studio = movie["Studios"][0]["Id"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(studio).is_ok());
    let by_studio = alice(&format!("Recursive=true&StudioIds={studio}")).await;
    assert_eq!(
        by_studio
            .iter()
            .map(|i| i["Id"].clone())
            .collect::<Vec<_>>(),
        ["movie"]
    );

    let show = alice("Ids=show").await.remove(0);
    assert_eq!(show["CommunityRating"], 8.4);
    assert_eq!(show["OfficialRating"], "TV-MA");
    assert_eq!(show["Studios"][0]["Name"], "HBO");
    assert_eq!(show["PremiereDate"], "2016-10-02T00:00:00Z");
    assert_eq!(show["ProviderIds"], json!({"Tmdb":"301","Tvdb":"201"}));
    assert!(show["ImageTags"].get("Primary").is_none());
    let episodes = alice("ParentId=season1&SortBy=IndexNumber").await;
    assert_eq!(
        episodes
            .iter()
            .map(|e| (e["PremiereDate"].clone(), e["ProviderIds"].clone()))
            .collect::<Vec<_>>(),
        [
            (json!("2015-11-20T08:00:00Z"), json!({"Tvdb":"5311261"})),
            (json!("2015-11-27T00:00:00Z"), json!({})),
            (Value::Null, json!({}))
        ]
    );
    // Playable unwatched episodes; a series leaves out its specials.
    for (token, show, season, specials) in [(&alice_token, 1, 1, 1), (&bob_token, 2, 2, 1)] {
        let counts = items("Ids=show,season1,specials".into(), token.clone()).await;
        let count = |id: &str| {
            counts.iter().find(|i| i["Id"] == id).unwrap()["UserData"]["UnplayedItemCount"].clone()
        };
        assert_eq!(
            [count("show"), count("season1"), count("specials")],
            [json!(show), json!(season), json!(specials)]
        );
    }
    assert!(episodes[0]["UserData"].get("UnplayedItemCount").is_none());
}
#[tokio::test]
async fn videos_without_artwork_show_a_still_from_their_file() {
    let (temp, state, _) = fixture().await;
    local_library(temp.path(), &state).await;
    let token = login(&state, "alice", DEVICE).await;
    let headers = [("X-Emby-Token", token.as_str())];
    let mut ids = Vec::new();
    for kind in ["Movie", "Audio"] {
        let (status, page) = call(
            &state,
            &format!("/Items?IncludeItemTypes={kind}"),
            "GET",
            Value::Null,
            &headers,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        ids.push(page["Items"][0].clone());
    }
    let (movie, track) = (&ids[0], &ids[1]);
    let tag = movie["ImageTags"]["Primary"].as_str().unwrap();
    assert!(track["ImageTags"].get("Primary").is_none());
    let id = movie["Id"].as_str().unwrap();
    // Clients that only send the image tag get the same still, extracted once.
    for _ in 0..2 {
        let (status, headers, body) = send(
            &state,
            &format!("/Items/{id}/Images/Primary?tag={tag}&fillWidth=400"),
            "GET",
            Value::Null,
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
        let still = image::load_from_memory(&body).unwrap();
        assert_eq!((still.width(), still.height()), (160, 90));
    }
    let (status, item) = call(
        &state,
        &format!("/Items/{id}"),
        "GET",
        Value::Null,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{item}");
    let source = &item["MediaSources"][0];
    assert_eq!(
        std::fs::read_to_string(
            state
                .config
                .cache
                .join(format!("artwork/stills/{id}.source"))
        )
        .unwrap(),
        format!(
            "{}:{}",
            source["Id"].as_str().unwrap(),
            source["ETag"].as_str().unwrap()
        )
    );
    let track = track["Id"].as_str().unwrap();
    let (status, _, _) = send(
        &state,
        &format!("/Items/{track}/Images/Primary"),
        "GET",
        Value::Null,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
