use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use thelxinoe_core::{Principal, Role, User, id, now};
use thelxinoe_server::{
    AppState,
    config::Config,
    playback::{Progress, report},
    router,
};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(Config {
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: temp.path().join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec![],
        cors_origins: vec![],
        controller_socket: temp.path().join("socket"),
    })
    .await
    .unwrap();
    state.db.call(|db|{
        for user in ["alice","bob","admin"]{db.execute("INSERT INTO users(id,username,password_hash,role,timezone,created_at) VALUES (?1,?1,'unused',?2,'UTC',?3)",params![user,if user=="admin"{"admin"}else{"user"},now()])?;}
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
    let token = thelxinoe_auth::issue_session(
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
        .call(move |db| {
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
        json!({"timezone":"UTC","timezone_override":null,"server_timezone":"UTC"})
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
        json!({"timezone":"Europe/Paris","timezone_override":null,"server_timezone":"Europe/Paris"})
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
        json!({"timezone":"America/New_York","timezone_override":null,"server_timezone":"America/New_York"})
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
        .call(|db| {
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
async fn timezone_choices_include_every_supported_timezone() {
    let (_temp, state) = fixture().await;
    let (alice, _) = login(&state, "alice").await;
    let response = ok(&state, &alice, "/timezones", "GET", Value::Null).await;
    let zones: Vec<_> = response["timezones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|zone| zone.as_str().unwrap())
        .collect();
    assert_eq!(zones.len(), chrono_tz::TZ_VARIANTS.len());
    assert!(zones.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(zones.contains(&"Europe/Paris"));
    assert!(zones.contains(&"UTC"));
    assert!(zones.contains(&"Etc/GMT-2"));
    for zone in zones {
        assert!(zone.parse::<chrono_tz::Tz>().is_ok());
    }
}
#[tokio::test]
async fn shared_playlists_keep_owner_edits_and_private_favorites_and_queues() {
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
    state.db.call(|db|{
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
    let client = id();
    let queue = format!("/me/queue/{client}");
    ok(
        &state,
        &alice,
        &queue,
        "PUT",
        json!({"revision":0,"items":["a","b"],"current_index":1}),
    )
    .await;
    assert_eq!(
        ok(&state, &bob, &queue, "GET", Value::Null).await["revision"],
        0
    );
    assert_eq!(
        ok(
            &state,
            &alice,
            &format!("/me/queue/{}", id()),
            "GET",
            Value::Null
        )
        .await["revision"],
        0
    );
    assert_eq!(
        call(
            &state,
            &alice,
            &queue,
            "PUT",
            json!({"revision":0,"items":["b"],"current_index":0})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let reopened =
        thelxinoe_database::Database::open(state.config.state.join("thelxinoe.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .call(move |db| Ok(db.query_row(
                "SELECT current_index FROM music_queues WHERE client_id=?1",
                [client],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        1
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
async fn session(
    state: &AppState,
    p: &Principal,
    key: &str,
    media: &str,
    queue: Option<(String, i64, i64)>,
) {
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let key = key.to_string();
    let media = media.to_string();
    state.db.call(move|db|{db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at,client_id,queue_revision,queue_index) VALUES (?1,?2,?3,?4,?4,'generation','','ready','direct','{}',100,?5,?5,?6,?7,?8)",params![key,user,auth,media,now(),queue.as_ref().map(|q|&q.0),queue.as_ref().map(|q|q.1),queue.as_ref().map(|q|q.2)])?;Ok(())}).await.unwrap();
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
    session(&state, &p, "play", "movie", None).await;
    progress(&state, &p, "play", 0, 0.0, "playing").await;
    state
        .db
        .call(|db| {
            db.execute(
                "UPDATE playback_history SET updated_at=?1 WHERE playback_id='play'",
                [now() - 10],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    progress(&state, &p, "play", 1, 10.0, "playing").await;
    progress(&state, &p, "play", 2, 90.0, "paused").await;
    progress(&state, &p, "play", 1, 20.0, "playing").await;
    let own = ok(&state, &alice, "/me/history", "GET", Value::Null).await;
    assert_eq!(own["stats"]["played_seconds"], 10.0);
    assert_eq!(own["items"][0]["position"], 90.0);
    assert_eq!(
        ok(&state, &bob, "/me/history?user=alice", "GET", Value::Null).await["stats"]["plays"],
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
            "/admin/history?user=alice",
            "GET",
            Value::Null
        )
        .await["users"][0]["username"],
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
        ok(&state, &again, "/me/history", "GET", Value::Null).await["stats"]["played_seconds"],
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
#[tokio::test]
async fn prefetched_and_stale_playbacks_cannot_rewind_or_overwrite_the_current_queue() {
    let (_temp, state) = fixture().await;
    let (token, p) = login(&state, "alice").await;
    let client = id();
    let path = format!("/me/queue/{client}");
    ok(
        &state,
        &token,
        &path,
        "PUT",
        json!({"revision":0,"items":["a","b"],"current_index":0}),
    )
    .await;
    session(&state, &p, "first", "a", Some((client.clone(), 1, 0))).await;
    session(&state, &p, "second", "b", Some((client.clone(), 1, 1))).await;
    assert_eq!(
        ok(&state, &token, &path, "GET", Value::Null).await["current_index"],
        0
    );
    progress(&state, &p, "first", 0, 5.0, "playing").await;
    progress(&state, &p, "second", 0, 1.0, "playing").await;
    progress(&state, &p, "first", 1, 100.0, "stopped").await;
    let saved = ok(&state, &token, &path, "GET", Value::Null).await;
    assert_eq!(saved["current_index"], 1);
    assert_eq!(saved["position"], 1.0);
    ok(
        &state,
        &token,
        &path,
        "PUT",
        json!({"revision":1,"items":["a"],"current_index":0}),
    )
    .await;
    progress(&state, &p, "second", 1, 20.0, "playing").await;
    let saved = ok(&state, &token, &path, "GET", Value::Null).await;
    assert_eq!(saved["current_index"], 0);
    assert_eq!(saved["position"], 0.0);
}
