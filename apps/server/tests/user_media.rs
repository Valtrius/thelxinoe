#[path = "../../../tests/helpers/auth-session.rs"]
mod auth_session;
#[path = "../../../tests/helpers/server-config.rs"]
mod configuration;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, Role, User, now};
use thelxinoe_server::{
    AppState,
    config::Config,
    playback::{Progress, report},
    router,
};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(configuration::config(temp.path()))
        .await
        .unwrap();
    state.db.write("test.fixture", |db|{
        for user in ["alice","bob","admin"]{db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES (?1,?1,'unused',?2,?3)",params![user,if user=="admin"{"admin"}else{"user"},now()])?;}
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Test','movies','/media/test')",[])?;
        for (id,kind,parent,number) in [("movie","movie",None,0),("a","track",None,1),("b","track",None,2),("show","show",None,0),("season","season",Some("show"),1),("specials","season",Some("show"),0),("special","episode",Some("specials"),1),("e1","episode",Some("season"),1),("e2","episode",Some("season"),2)] {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES (?1,'root',?2,?3,?1,?1,?4,?5)",params![id,kind,parent,number,now()])?;
            if ["movie","track","episode"].contains(&kind){
                db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,'root',?1,'generation',10,'mtime',?1,'{}',?2)",params![id,now()])?;
                db.execute("INSERT INTO media_sources VALUES (?1,?1)",[id])?;
            }
        }
        Ok(())
    }).await.unwrap();
    (temp, state)
}
async fn login(state: &AppState, user: &str) -> (String, Principal) {
    let token = auth_session::issue_session(
        &state.db,
        user.into(),
        "device".into(),
        format!("{user} test device"),
    )
    .await
    .unwrap();
    let hash = thelxinoe_auth::digest(&token);
    let session = state
        .db
        .write("test.fixture", move |db| {
            Ok(
                db.query_row("SELECT id FROM sessions WHERE token_hash=?1", [hash], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .unwrap();
    (
        token,
        Principal {
            session_id: session,
            user: User {
                id: user.into(),
                username: user.into(),
                role: if user == "admin" {
                    Role::Admin
                } else {
                    Role::User
                },
                timezone: "UTC".into(),
            },
            transport: "device".into(),
        },
    )
}
async fn call(
    state: &AppState,
    token: &str,
    path: &str,
    method: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .uri(format!("/api/v1{path}"))
        .method(method)
        .header("authorization", format!("Bearer {token}"))
        .header("host", "media.test")
        .header("x-thelxinoe-client", "1")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let code = response.status();
    let value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
        .unwrap_or(Value::Null);
    (code, value)
}
async fn ok(state: &AppState, token: &str, path: &str, method: &str, body: Value) -> Value {
    let (status, value) = call(state, token, path, method, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value
}
#[tokio::test]
async fn timezone_defaults_follow_server_changes_until_explicitly_overridden() {
    let (_temp, state) = fixture().await;
    let (alice, _) = login(&state, "alice").await;
    let (bob, _) = login(&state, "bob").await;
    let (admin, _) = login(&state, "admin").await;
    let initial = ok(&state, &alice, "/me/preferences", "GET", Value::Null).await;
    assert_eq!(
        initial,
        json!({"timezone":"UTC","timezone_override":null,"server_timezone":"UTC","time_format":"24h","time_format_override":null,"server_time_format":"24h"})
    );

    // Explicitly choosing UTC must remain distinct from leaving the default.
    ok(
        &state,
        &bob,
        "/me/preferences",
        "PUT",
        json!({"timezone":"UTC"}),
    )
    .await;
    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"Europe/Paris"}),
    )
    .await;
    assert_eq!(
        ok(&state, &alice, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "Europe/Paris"
    );
    assert_eq!(
        ok(&state, &bob, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "UTC"
    );
    assert_eq!(
        ok(&state, &alice, "/me/preferences", "GET", Value::Null).await,
        json!({"timezone":"Europe/Paris","timezone_override":null,"server_timezone":"Europe/Paris","time_format":"24h","time_format_override":null,"server_time_format":"24h"})
    );

    // New accounts and their initial login use the current server default.
    let created = ok(
        &state,
        &admin,
        "/users",
        "POST",
        json!({"username":"charlie","password":"test-only long passphrase","role":"user"}),
    )
    .await;
    let (charlie, _) = login(&state, created["id"].as_str().unwrap()).await;
    assert_eq!(
        ok(&state, &charlie, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "Europe/Paris"
    );
    let logged_in = ok(
        &state,
        "",
        "/auth/login",
        "POST",
        json!({"username":"charlie","password":"test-only long passphrase","transport":"device"}),
    )
    .await;
    assert_eq!(logged_in["user"]["timezone"], "Europe/Paris");

    ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":"Asia/Tokyo"}),
    )
    .await;
    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"America/New_York"}),
    )
    .await;
    assert_eq!(
        ok(&state, &alice, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "Asia/Tokyo"
    );
    assert_eq!(
        ok(&state, &charlie, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "America/New_York"
    );
    assert_eq!(
        ok(
            &state,
            &alice,
            "/me/preferences",
            "PUT",
            json!({"timezone":null})
        )
        .await,
        json!({"timezone":"America/New_York","timezone_override":null,"server_timezone":"America/New_York","time_format":"24h","time_format_override":null,"server_time_format":"24h"})
    );
    let users = ok(&state, &admin, "/users", "GET", Value::Null).await;
    assert!(
        users["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|user| user["id"] == "alice" && user["timezone"] == "America/New_York")
    );

    for zone in ["UTC+2", "not/a/zone", ""] {
        assert_eq!(
            call(
                &state,
                &alice,
                "/me/preferences",
                "PUT",
                json!({"timezone":zone})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                &state,
                &admin,
                "/admin/settings",
                "PUT",
                json!({"timezone":zone})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &state,
            &alice,
            "/admin/settings",
            "PUT",
            json!({"timezone":"UTC"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ok(&state, &alice, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "America/New_York"
    );
    state
        .db
        .write("test.fixture", |db| {
            assert!(
                db.prepare(
                    "SELECT 1 FROM events WHERE kind='preferences.changed' AND user_id='alice'"
                )?
                .exists([])?
            );
            assert!(
                db.prepare(
                    "SELECT 1 FROM events WHERE kind='server.settings.changed' AND user_id IS NULL"
                )?
                .exists([])?
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn time_format_follows_server_until_explicitly_overridden() {
    let (_temp, state) = fixture().await;
    let (alice, _) = login(&state, "alice").await;
    let (bob, _) = login(&state, "bob").await;
    let (admin, _) = login(&state, "admin").await;

    let initial = ok(&state, &alice, "/me/preferences", "GET", Value::Null).await;
    assert_eq!(initial["time_format"], "24h");
    assert_eq!(initial["time_format_override"], Value::Null);
    assert_eq!(initial["server_time_format"], "24h");

    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"UTC","time_format":"12h"}),
    )
    .await;
    assert_eq!(
        ok(&state, &alice, "/me/preferences", "GET", Value::Null).await["time_format"],
        "12h"
    );
    assert_eq!(
        ok(&state, &bob, "/me/preferences", "GET", Value::Null).await["time_format"],
        "12h"
    );

    let overridden = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":null,"time_format":"24h"}),
    )
    .await;
    assert_eq!(overridden["time_format"], "24h");
    assert_eq!(overridden["time_format_override"], "24h");
    assert_eq!(overridden["server_time_format"], "12h");

    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"UTC","time_format":"24h"}),
    )
    .await;
    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"UTC","time_format":"12h"}),
    )
    .await;
    assert_eq!(
        ok(&state, &bob, "/me/preferences", "GET", Value::Null).await["time_format"],
        "12h"
    );
    assert_eq!(
        ok(&state, &alice, "/me/preferences", "GET", Value::Null).await["time_format"],
        "24h"
    );

    let inherited = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":null,"time_format":null}),
    )
    .await;
    assert_eq!(inherited["time_format"], "12h");
    assert_eq!(inherited["time_format_override"], Value::Null);

    assert_eq!(
        call(
            &state,
            &alice,
            "/me/preferences",
            "PUT",
            json!({"timezone":null,"time_format":"system"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &admin,
            "/admin/settings",
            "PUT",
            json!({"timezone":"UTC","time_format":"system"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        ok(&state, &alice, "/me/preferences", "GET", Value::Null).await["time_format"],
        "12h"
    );
}

#[tokio::test]
async fn time_format_update_distinguishes_omitted_null_and_override() {
    let (_temp, state) = fixture().await;
    let (alice, _) = login(&state, "alice").await;
    let (admin, _) = login(&state, "admin").await;

    ok(
        &state,
        &admin,
        "/admin/settings",
        "PUT",
        json!({"timezone":"UTC","time_format":"12h"}),
    )
    .await;
    let overridden = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":null,"time_format":"24h"}),
    )
    .await;
    assert_eq!(overridden["time_format"], "24h");
    assert_eq!(overridden["time_format_override"], "24h");

    let omitted = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":"Europe/Paris"}),
    )
    .await;
    assert_eq!(omitted["timezone_override"], "Europe/Paris");
    assert_eq!(omitted["time_format"], "24h");
    assert_eq!(omitted["time_format_override"], "24h");
    assert_eq!(omitted["server_time_format"], "12h");

    let inherited = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":"Europe/Paris","time_format":null}),
    )
    .await;
    assert_eq!(inherited["time_format"], "12h");
    assert_eq!(inherited["time_format_override"], Value::Null);

    let explicit = ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":"Europe/Paris","time_format":"24h"}),
    )
    .await;
    assert_eq!(explicit["time_format"], "24h");
    assert_eq!(explicit["time_format_override"], "24h");
}

#[tokio::test]
async fn shared_playlists_keep_owner_edits_and_private_favorites() {
    let (_temp, state) = fixture().await;
    let (alice, _) = login(&state, "alice").await;
    let (bob, _) = login(&state, "bob").await;
    ok(
        &state,
        &alice,
        "/catalog/movie/state",
        "PUT",
        json!({"favorite":true,"watch_later":true}),
    )
    .await;
    let other = ok(&state, &bob, "/catalog/movie/state", "GET", Value::Null).await;
    assert_eq!(other["favorite"], false);
    assert_eq!(other["watch_later"], false);
    state.db.write("test.fixture", |db|{
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,edition,scanned_at) VALUES ('extended','root','extended','generation',10,'mtime','extended','{}','Extended',?1)",[now()])?;
        db.execute("INSERT INTO media_sources VALUES ('movie','extended')",[])?;
        db.execute("INSERT INTO edition_progress VALUES ('alice','movie','',3,100,?1),('alice','movie','Extended',7,100,?2)",params![now()-10,now()])?;
        Ok(())
    }).await.unwrap();
    let resume = ok(&state, &alice, "/me/home", "GET", Value::Null).await;
    assert_eq!(resume["continue_watching"][0]["fileId"], "extended");
    assert_eq!(resume["continue_watching"][0]["position"], 7.0);
    assert_eq!(
        call(
            &state,
            &bob,
            "/catalog/a/state",
            "PUT",
            json!({"watch_later":true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    ok(
        &state,
        &alice,
        "/catalog/show/state",
        "PUT",
        json!({"favorite":true}),
    )
    .await;
    let home = ok(&state, &alice, "/me/home", "GET", Value::Null).await;
    assert_eq!(home["next_up"][0]["id"], "e1");
    ok(
        &state,
        &alice,
        "/catalog/e1/state",
        "PUT",
        json!({"watched":true}),
    )
    .await;
    let home = ok(&state, &alice, "/me/home", "GET", Value::Null).await;
    assert_eq!(home["next_up"][0]["id"], "e2");
    assert!(
        ok(&state, &bob, "/me/home", "GET", Value::Null).await["next_up"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let created = ok(
        &state,
        &alice,
        "/playlists",
        "POST",
        json!({"name":"Shared mix","items":["a","b","a"]}),
    )
    .await;
    let path = format!("/playlists/{}", created["id"].as_str().unwrap());
    let list = ok(&state, &bob, &path, "GET", Value::Null).await;
    assert_eq!(list["owner"], "alice");
    assert_eq!(list["items"].as_array().unwrap().len(), 3);
    assert_eq!(list["items"][2]["id"], "a");
    assert_eq!(
        call(
            &state,
            &bob,
            &path,
            "PUT",
            json!({"name":"Hijack","revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&state, &bob, &path, "DELETE", Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    ok(
        &state,
        &bob,
        &format!("{path}/favorite"),
        "PUT",
        json!({"favorite":true}),
    )
    .await;
    assert_eq!(
        ok(&state, &bob, &path, "GET", Value::Null).await["favorite"],
        true
    );
    assert_eq!(
        ok(&state, &alice, &path, "GET", Value::Null).await["favorite"],
        false
    );
    let edit = json!({"name":"Reordered","items":["b","a"],"revision":1});
    ok(&state, &alice, &path, "PUT", edit.clone()).await;
    assert_eq!(
        call(&state, &alice, &path, "PUT", edit).await.0,
        StatusCode::CONFLICT
    );
    ok(
        &state,
        &alice,
        "/me/preferences",
        "PUT",
        json!({"timezone":"Europe/Paris"}),
    )
    .await;
    assert_eq!(
        ok(&state, &alice, "/auth/me", "GET", Value::Null).await["user"]["timezone"],
        "Europe/Paris"
    );
    assert_eq!(
        call(
            &state,
            &alice,
            "/me/preferences",
            "PUT",
            json!({"timezone":"not/a/zone"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
async fn session(state: &AppState, p: &Principal, key: &str, media: &str) {
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let key = key.to_string();
    let media = media.to_string();
    state.db.write("test.fixture", move|db|{db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES (?1,?2,?3,?4,?4,'generation','','ready','direct','{}',100,?5,?5)",params![key,user,auth,media,now()])?;Ok(())}).await.unwrap();
}
async fn progress(
    state: &AppState,
    p: &Principal,
    key: &str,
    sequence: i64,
    position: f64,
    status: &str,
) {
    report(
        state,
        p,
        key,
        Progress {
            active_seconds: None,
            sequence,
            position,
            state: status.into(),
        },
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn viewing_statistics_exclude_seek_jumps_and_survive_device_revocation() {
    let (_temp, state) = fixture().await;
    let (alice, p) = login(&state, "alice").await;
    let (bob, _) = login(&state, "bob").await;
    let (admin, _) = login(&state, "admin").await;
    session(&state, &p, "play", "movie").await;
    progress(&state, &p, "play", 0, 0.0, "playing").await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE playback_sessions SET reported_at_ms=?1 WHERE id='play'",
                [chrono::Utc::now().timestamp_millis() - 10_000],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    progress(&state, &p, "play", 1, 10.0, "playing").await;
    progress(&state, &p, "play", 2, 90.0, "paused").await;
    progress(&state, &p, "play", 1, 20.0, "playing").await;
    let own = ok(
        &state,
        &alice,
        "/me/history?range=all&platform=movies",
        "GET",
        Value::Null,
    )
    .await;
    assert_eq!(own["items"][0]["played_seconds"], 10.0);
    assert_eq!(own["items"][0]["position"], 90.0);
    assert_eq!(
        ok(
            &state,
            &bob,
            "/me/history?range=all&user=alice",
            "GET",
            Value::Null
        )
        .await["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        call(&state, &bob, "/admin/history", "GET", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ok(
            &state,
            &admin,
            "/admin/history?range=all&user=alice",
            "GET",
            Value::Null
        )
        .await["items"][0]["username"],
        "alice"
    );
    ok(
        &state,
        &admin,
        &format!("/auth/sessions/{}", p.session_id),
        "DELETE",
        Value::Null,
    )
    .await;
    let (again, _) = login(&state, "alice").await;
    assert_eq!(
        ok(&state, &again, "/me/history?range=all", "GET", Value::Null).await["items"][0]["played_seconds"],
        10.0
    );
    assert_eq!(
        call(&state, &again, "/admin/audit", "GET", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ok(&state, &admin, "/admin/audit", "GET", Value::Null).await["items"][0]["action"],
        "session.revoke"
    );
}

async fn watched(state: &AppState, token: &str) -> Vec<&'static str> {
    let mut watched = Vec::new();
    for id in ["e1", "e2", "e3", "e4", "special"] {
        let path = format!("/catalog/{id}/state");
        if ok(state, token, &path, "GET", Value::Null).await["watched"] == true {
            watched.push(id);
        }
    }
    watched
}
#[tokio::test]
async fn seasons_and_the_episodes_before_one_are_marked_at_once() {
    let (_temp, state) = fixture().await;
    state.db.write("test.fixture", |db| {
        for (id,kind,parent,number) in [("season2","season","show",2),("e3","episode","season2",1),("e4","episode","season2",2)] {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES (?1,'root',?2,?3,?1,?1,?4,?5)",params![id,kind,parent,number,now()])?;
        }
        Ok(())
    }).await.unwrap();
    let (alice, _) = login(&state, "alice").await;
    let (bob, _) = login(&state, "bob").await;
    let mark = |id: &str, body: Value| {
        let (state, alice, path) = (
            state.clone(),
            alice.clone(),
            format!("/catalog/{id}/watched"),
        );
        async move { call(&state, &alice, &path, "PUT", body).await }
    };

    // Through an episode: the seasons before it and its own earlier episodes.
    let through = json!({"watched":true,"through":true});
    assert_eq!(mark("e3", through.clone()).await.1, json!({"episodes":3}));
    assert_eq!(watched(&state, &alice).await, ["e1", "e2", "e3"]);
    // Specials only count other Specials.
    assert_eq!(
        mark("special", through.clone()).await.1,
        json!({"episodes":1})
    );
    assert_eq!(watched(&state, &alice).await, ["e1", "e2", "e3", "special"]);

    // A whole season, which also leaves Watch Later once every episode is watched.
    ok(
        &state,
        &alice,
        "/catalog/season2/state",
        "PUT",
        json!({"watch_later":true}),
    )
    .await;
    assert_eq!(
        mark("season2", json!({"watched":true})).await.1,
        json!({"episodes":2})
    );
    assert_eq!(
        ok(&state, &alice, "/catalog/season2/state", "GET", Value::Null).await["watch_later"],
        false
    );
    assert_eq!(
        mark("season", json!({"watched":false})).await.1,
        json!({"episodes":2})
    );
    assert_eq!(watched(&state, &alice).await, ["e3", "e4", "special"]);
    assert!(watched(&state, &bob).await.is_empty());

    for (id, body, status) in [
        ("season", through.clone(), StatusCode::BAD_REQUEST),
        ("show", json!({"watched":true}), StatusCode::BAD_REQUEST),
        ("movie", json!({"watched":true}), StatusCode::BAD_REQUEST),
        ("nothing", json!({"watched":true}), StatusCode::NOT_FOUND),
    ] {
        assert_eq!(mark(id, body).await.0, status, "{id}");
    }
}
