#[path = "../../../tests/helpers/server-config.rs"]
mod configuration;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use thelxinoe_server::{AppState, config::Config, router};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(Config {
        trusted_proxies: vec!["10.0.0.0/24".parse().unwrap()],
        ..configuration::config(temp.path())
    })
    .await
    .unwrap();
    (temp, state)
}
async fn request(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut builder = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "media.test")
        .header("x-thelxinoe-client", "1");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    for (k, v) in extra {
        builder = builder.header(*k, *v);
    }
    let mut req = builder
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "10.0.0.2:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router(state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}
async fn setup(state: &AppState) -> String {
    let (status, headers, _) = request(
        state,
        "/api/v1/setup",
        "POST",
        json!({"username":"admin","password":"a good long password"}),
        None,
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("HttpOnly"));
    cookie.split(';').next().unwrap().into()
}
#[tokio::test]
async fn setup_and_user_creation_accept_eight_characters() {
    let (_temp, state) = fixture().await;
    let (status, _, _) = request(
        &state,
        "/api/v1/setup",
        "POST",
        json!({"username":"admin","password":"1234567"}),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, headers, _) = request(
        &state,
        "/api/v1/setup",
        "POST",
        json!({"username":"admin","password":"12345678"}),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let (status, _, _) = request(
        &state,
        "/api/v1/users",
        "POST",
        json!({"username":"member","password":"abcdefgh","role":"user"}),
        Some(cookie),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn setup_needs_only_credentials_and_accepts_one_concurrent_administrator() {
    let (_temp, state) = fixture().await;
    assert!(!state.config.state.join("secrets/setup-token").exists());
    assert_eq!(
        request(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"intruder","password":"a good long password"}),
            None,
            &[("origin", "https://unrelated.test")],
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (first, second) = tokio::join!(
        request(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"alice","password":"a good long password"}),
            None,
            &[],
        ),
        request(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"bob","password":"another long password"}),
            None,
            &[],
        )
    );
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);
    state
        .db
        .write("test.fixture", |db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM users WHERE role='admin'", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();

    let reopened = AppState::open(state.config.as_ref().clone()).await.unwrap();
    assert_eq!(
        request(&reopened, "/api/v1/setup", "GET", Value::Null, None, &[])
            .await
            .2["setup_required"],
        false
    );
    assert_eq!(
        request(
            &reopened,
            "/api/v1/setup",
            "POST",
            json!({"username":"later","password":"a good long password"}),
            None,
            &[],
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn first_admin_is_atomic_and_sessions_survive_restart_and_revoke() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    assert_eq!(
        request(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"other","password":"a good long password"}),
            None,
            &[]
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let reopened = AppState::open(state.config.as_ref().clone()).await.unwrap();
    assert_eq!(
        request(
            &reopened,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            Some(&cookie),
            &[]
        )
        .await
        .2["user"]["role"],
        "admin"
    );
    let sessions = request(
        &reopened,
        "/api/v1/auth/sessions",
        "GET",
        Value::Null,
        Some(&cookie),
        &[],
    )
    .await
    .2;
    let sid = sessions["items"][0]["id"].as_str().unwrap();
    assert_eq!(
        request(
            &reopened,
            &format!("/api/v1/auth/sessions/{sid}"),
            "DELETE",
            Value::Null,
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &reopened,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn regular_user_cannot_administer_and_device_tokens_are_transport_scoped() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    assert_eq!(
        request(
            &state,
            "/api/v1/users",
            "POST",
            json!({"username":"member","password":"member long password","role":"user"}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, login) = request(
        &state,
        "/api/v1/auth/login",
        "POST",
        json!({"username":"member","password":"member long password","transport":"device"}),
        None,
        &[],
    )
    .await;
    let token = login["token"].as_str().unwrap();
    let bearer = format!("Bearer {token}");
    assert_eq!(
        request(
            &state,
            "/api/v1/users",
            "GET",
            Value::Null,
            None,
            &[("authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            None,
            &[("authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            Some(&format!("thelxinoe_session={token}")),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/users",
            "POST",
            json!({}),
            Some(&cookie),
            &[("origin", "https://evil.test")]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn forwarded_headers_require_trusted_proxy_and_client_chain_is_resolved_from_right() {
    let (_temp, state) = fixture().await;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("host", "media.test".parse().unwrap());
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    headers.insert("x-forwarded-host", "public.test".parse().unwrap());
    headers.insert(
        "x-forwarded-for",
        "1.2.3.4, 5.6.7.8, 10.0.0.3".parse().unwrap(),
    );
    let context = thelxinoe_server::security::request_context(
        &state.config,
        &headers,
        "192.168.1.4:12".parse().unwrap(),
    )
    .unwrap();
    assert!(!context.secure);
    assert_eq!(context.origin, "http://media.test");
    assert_eq!(context.address.to_string(), "192.168.1.4");
    let context = thelxinoe_server::security::request_context(
        &state.config,
        &headers,
        "10.0.0.2:12".parse().unwrap(),
    )
    .unwrap();
    assert!(context.secure);
    assert_eq!(context.origin, "https://public.test");
    assert_eq!(context.address.to_string(), "5.6.7.8");
}

#[tokio::test]
async fn manual_fields_win_after_metadata_refresh() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    state.db.write("test.fixture", |db|{db.execute_batch("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Fixture','movies','/fixture'); INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES ('movie','root','movie','fixture','Local title','{\"title\":\"Initial provider title\"}',0);")?;Ok(())}).await.unwrap();
    assert_eq!(
        request(
            &state,
            "/api/v1/catalog/movie/overrides",
            "PUT",
            json!({"title":"My corrected title"}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute(
                "UPDATE media SET metadata=?1 WHERE id='movie'",
                [json!({"title":"Refreshed provider title"}).to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let item = request(
        &state,
        "/api/v1/catalog/movie",
        "GET",
        Value::Null,
        Some(&cookie),
        &[],
    )
    .await
    .2;
    assert_eq!(item["title"], "My corrected title");
    assert_eq!(item["metadata"]["title"], "Refreshed provider title");
    request(
        &state,
        "/api/v1/catalog/movie/overrides",
        "PUT",
        json!({}),
        Some(&cookie),
        &[],
    )
    .await;
    assert_eq!(
        request(
            &state,
            "/api/v1/catalog/movie",
            "GET",
            Value::Null,
            Some(&cookie),
            &[]
        )
        .await
        .2["title"],
        "Refreshed provider title"
    );
}

#[tokio::test]
async fn episode_manager_numbering_is_explicit_and_complex_mappings_are_marked() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    state.db.write("test.fixture", |db|{db.execute_batch("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','TV','shows','/tv');
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('show','r','show',NULL,'show','Show',NULL,0),('season','r','season','show','s','Season',1,0),('e1','r','episode','season','e1','First',1,0),('e2','r','episode','season','e2','Second',2,0);
INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at,error) VALUES ('sonarr','Fixture','sonarr','container',8989,'g',X'00','','{}','1',1,1,NULL);
INSERT INTO metadata_bindings VALUES ('show','sonarr','g','100',NULL,1);
INSERT INTO manager_episodes VALUES ('sonarr','g','100',201,1,2,1,'{}'),('sonarr','g','100',202,1,1,1,'{}');")?;Ok(())}).await.unwrap();
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT count(*) FROM manager_episode_mappings",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/catalog/e1/episode-mapping",
            "PUT",
            json!({"episode_ids":["201"]}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/catalog/e2/episode-mapping",
            "PUT",
            json!({"episode_ids":["201"]}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT count(*) FROM manager_episode_mappings WHERE state='complex'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        request(
            &state,
            "/api/v1/catalog/e1/episode-mapping",
            "PUT",
            json!({"episode_ids":["999"]}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn missing_master_key_blocks_startup_without_generating_a_replacement() {
    let (_temp, state) = fixture().await;
    state
        .secrets
        .put(&state.db, "fixture".into(), b"a secret")
        .await
        .unwrap();
    let key = state.config.state.join("secrets/master.key");
    std::fs::remove_file(&key).unwrap();
    assert!(AppState::open(state.config.as_ref().clone()).await.is_err());
    assert!(!key.exists());
}

#[tokio::test]
async fn explicit_cors_and_api_compatibility_preserve_authentication() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let origin = "https://client.example.test";
    let preflight = [
        ("origin", origin),
        ("access-control-request-method", "POST"),
        (
            "access-control-request-headers",
            "X-Thelxinoe-Client, Content-Type",
        ),
    ];
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/login",
            "OPTIONS",
            Value::Null,
            None,
            &preflight
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut config = state.config.as_ref().clone();
    config.cors_origins = vec![origin.into()];
    let state = AppState::open(config).await.unwrap();
    let (status, headers, _) = request(
        &state,
        "/api/v1/auth/login",
        "OPTIONS",
        Value::Null,
        None,
        &preflight,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(headers["access-control-allow-origin"], origin);
    assert_eq!(headers["access-control-allow-credentials"], "true");
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            None,
            &[("origin", origin)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, headers, _) = request(
        &state,
        "/api/v1/auth/me",
        "GET",
        Value::Null,
        Some(&cookie),
        &[("origin", origin)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["access-control-allow-origin"], origin);
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            Some(&cookie),
            &[("x-thelxinoe-api", "999")]
        )
        .await
        .0,
        StatusCode::UPGRADE_REQUIRED
    );
    for endpoint in ["/api/v1/health", "/api/v1/release"] {
        assert_eq!(
            request(
                &state,
                endpoint,
                "GET",
                Value::Null,
                None,
                &[("x-thelxinoe-api", "999")]
            )
            .await
            .0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn remote_quality_uses_the_trusted_client_address() {
    let (_temp, state) = fixture().await;
    for (address, remote) in [
        ("192.168.1.9", false),
        ("127.0.0.1", false),
        ("::ffff:192.168.1.9", false),
        ("2001:4860:4860::8888", true),
        ("8.8.8.8", true),
    ] {
        let headers = axum::http::HeaderMap::from_iter([(
            "x-forwarded-for".parse::<axum::http::HeaderName>().unwrap(),
            address.parse().unwrap(),
        )]);
        let context = thelxinoe_server::security::request_context(
            &state.config,
            &headers,
            "10.0.0.2:1234".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(context.remote(), remote);
        let untrusted = thelxinoe_server::security::request_context(
            &state.config,
            &headers,
            "192.168.1.2:1234".parse().unwrap(),
        )
        .unwrap();
        assert!(!untrusted.remote());
    }
}

#[tokio::test]
async fn concurrent_library_additions_serialize_overlap_checks_and_job_writes() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let state = state.clone();
        let cookie = cookie.clone();
        let path = state.config.media.join(format!("library-{}", index / 2));
        std::fs::create_dir_all(&path).unwrap();
        tasks.spawn(async move {
            request(
                &state,
                "/api/v1/catalog/roots",
                "POST",
                json!({"name":"Concurrent library","kind":"movies","path":path}),
                Some(&cookie),
                &[],
            )
            .await
            .0
        });
    }
    let mut added = 0;
    let mut overlap = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            StatusCode::OK => added += 1,
            StatusCode::CONFLICT => overlap += 1,
            status => panic!("Unexpected concurrent library response: {status}"),
        }
    }
    assert_eq!((added, overlap), (8, 8));
    assert_eq!(
        state
            .db
            .write("test.fixture", |db| Ok(db.query_row(
                "SELECT count(*) FROM jobs WHERE kind='library.scan'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        8
    );
}

#[tokio::test]
async fn desktop_event_tickets_connect_from_dev_origins_without_relaxing_cookie_auth() {
    use futures_util::StreamExt;
    use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let (_, _, login) = request(
        &state,
        "/api/v1/auth/login",
        "POST",
        json!({"username":"admin","password":"a good long password","transport":"device"}),
        None,
        &[],
    )
    .await;
    let bearer = format!("Bearer {}", login["token"].as_str().unwrap());
    let device = [("authorization", bearer.as_str())];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let handshake = |ticket: &str, since: i64, origin: &str| {
        let mut req = format!("ws://{address}/api/v1/events?ticket={ticket}&since={since}")
            .into_client_request()
            .unwrap();
        req.headers_mut().insert("origin", origin.parse().unwrap());
        req
    };

    // Both packaged and development WebViews receive changes from browser clients.
    // Each reconnect obtains a new ticket and replays changes after its cursor.
    for origin in [
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://tauri.localhost",
    ] {
        let (_, _, issued) = request(
            &state,
            "/api/v1/auth/event-ticket",
            "POST",
            json!({}),
            None,
            &device,
        )
        .await;
        let ticket = issued["ticket"].as_str().unwrap();
        let since = issued["cursor"].as_i64().unwrap();
        let (mut socket, response) = connect_async(handshake(ticket, since, origin))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            request(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"theme":"dark"}),
                Some(&cookie),
                &[]
            )
            .await
            .0,
            StatusCode::OK
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if message.is_text() {
                    let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    if event["kind"] == "appearance.changed" {
                        break event;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event["payload"]["appearance"]["theme"], "dark");
        let cursor = event["id"].as_i64().unwrap();
        socket.close(None).await.unwrap();
        assert!(
            connect_async(handshake(ticket, cursor, origin))
                .await
                .is_err(),
            "Tickets must be single-use"
        );

        assert_eq!(
            request(
                &state,
                "/api/v1/me/appearance",
                "PATCH",
                json!({"theme":"light"}),
                Some(&cookie),
                &[]
            )
            .await
            .0,
            StatusCode::OK
        );
        let (_, _, issued) = request(
            &state,
            "/api/v1/auth/event-ticket",
            "POST",
            json!({}),
            None,
            &device,
        )
        .await;
        let (mut socket, _) = connect_async(handshake(
            issued["ticket"].as_str().unwrap(),
            cursor,
            origin,
        ))
        .await
        .unwrap();
        let replay = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let replay: Value = serde_json::from_str(replay.to_text().unwrap()).unwrap();
        assert_eq!(replay["payload"]["appearance"]["theme"], "light");
        socket.close(None).await.unwrap();
    }

    let origin = "http://localhost:5173";
    let (_, _, web) = request(
        &state,
        "/api/v1/auth/event-ticket",
        "POST",
        json!({}),
        Some(&cookie),
        &[],
    )
    .await;
    for ticket in ["", "invalid", web["ticket"].as_str().unwrap()] {
        let error = connect_async(handshake(ticket, 0, origin))
            .await
            .unwrap_err();
        assert!(
            matches!(error, tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == StatusCode::FORBIDDEN)
        );
    }
    assert_eq!(
        request(
            &state,
            "/api/v1/auth/event-ticket",
            "POST",
            json!({}),
            Some(&cookie),
            &[("origin", origin)]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, _, issued) = request(
        &state,
        "/api/v1/auth/event-ticket",
        "POST",
        json!({}),
        None,
        &device,
    )
    .await;
    let ticket = issued["ticket"].as_str().unwrap();
    assert_eq!(
        request(
            &state,
            &format!("/api/v1/auth/me?ticket={ticket}"),
            "GET",
            json!({}),
            Some(&cookie),
            &[("origin", origin)]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    request(
        &state,
        "/api/v1/auth/logout",
        "POST",
        json!({}),
        None,
        &device,
    )
    .await;
    assert!(
        connect_async(handshake(ticket, 0, origin)).await.is_err(),
        "Revoked device tickets must fail"
    );
    server.abort();
}
fn client(address: &str) -> [(&str, &str); 2] {
    [("x-forwarded-proto", "https"), ("x-forwarded-for", address)]
}
async fn post(
    state: &AppState,
    path: &str,
    body: Value,
    cookie: Option<&str>,
) -> (StatusCode, Value) {
    let (status, _, body) =
        request(state, path, "POST", body, cookie, &client("198.51.100.7")).await;
    (status, body)
}
fn error_code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_default()
}
#[tokio::test]
async fn desktop_approval_requires_the_desktop_code_and_never_transfers_freshness() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let (status, started) = post(
        &state,
        "/api/v1/auth/desktop/start",
        json!({"device_name":"Test desktop"}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = started["code"].as_str().unwrap();
    let id = started["request"].as_str().unwrap();
    let (status, _, info) = request(
        &state,
        &format!("/api/v1/auth/desktop/info?request={id}"),
        "GET",
        Value::Null,
        Some(&cookie),
        &client("203.0.113.5"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["requested_from"], "198.51.100.7");
    assert_eq!(info["same_network"], false);
    let approve = async |code: &str| {
        let (status, _, body) = request(
            &state,
            "/api/v1/auth/desktop/approve",
            "POST",
            json!({"request":id,"code":code}),
            Some(&cookie),
            &client("203.0.113.5"),
        )
        .await;
        (status, error_code(&body).to_owned())
    };
    let wrong = if code.starts_with('A') {
        "BBBB-BBBB"
    } else {
        "AAAA-AAAA"
    };
    assert_eq!(
        approve("").await,
        (StatusCode::BAD_REQUEST, "desktop_code_format".into())
    );
    assert_eq!(
        approve(wrong).await,
        (StatusCode::BAD_REQUEST, "desktop_code_mismatch".into())
    );
    assert_eq!(
        approve(&code.replace('-', " ").to_lowercase()).await.0,
        StatusCode::OK
    );
    let (status, exchanged) = post(
        &state,
        "/api/v1/auth/desktop/exchange",
        json!({"request":id,"secret":started["secret"]}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bearer = format!("Bearer {}", exchanged["token"].as_str().unwrap());
    let (status, _, methods) = request(
        &state,
        "/api/v1/me/auth",
        "GET",
        Value::Null,
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(methods["fresh"], false);

    let (_, guessed) = post(
        &state,
        "/api/v1/auth/desktop/start",
        json!({"device_name":"Guessed desktop"}),
        None,
    )
    .await;
    let id = guessed["request"].as_str().unwrap();
    let code = guessed["code"].as_str().unwrap();
    let wrong = if code.starts_with('A') {
        "BBBB-BBBB"
    } else {
        "AAAA-AAAA"
    };
    let approve = async |code: &str| {
        let (status, _, body) = request(
            &state,
            "/api/v1/auth/desktop/approve",
            "POST",
            json!({"request":id,"code":code}),
            Some(&cookie),
            &client("203.0.113.5"),
        )
        .await;
        (status, error_code(&body).to_owned())
    };
    for _ in 1..5 {
        assert_eq!(
            approve(wrong).await,
            (StatusCode::BAD_REQUEST, "desktop_code_mismatch".into())
        );
    }
    assert_eq!(
        approve(wrong).await,
        (StatusCode::CONFLICT, "desktop_request_ended".into())
    );
    assert_eq!(
        approve(code).await,
        (StatusCode::CONFLICT, "desktop_request_ended".into())
    );
}
fn totp_code(secret: &str, offset: i64) -> String {
    let totp = totp_rs::Builder::new()
        .with_secret(totp_rs::Secret::try_from_base32(secret).unwrap())
        .build()
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    totp.generate((now + offset * 30) as u64).to_string()
}
fn wrong_code(secret: &str) -> String {
    let valid: Vec<String> = (-2..=2).map(|offset| totp_code(secret, offset)).collect();
    (100_000..)
        .map(|n| n.to_string())
        .find(|code| !valid.contains(code))
        .unwrap()
}
#[tokio::test]
async fn totp_errors_are_specific_and_failed_codes_are_throttled_per_account() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let (status, enrollment) = post(
        &state,
        "/api/v1/me/auth/totp/start",
        json!({}),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enrollment["account"], "admin@media.test");
    assert!(
        enrollment["uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Thelxinoe:admin%40media.test?")
    );
    let secret = enrollment["secret"].as_str().unwrap().to_owned();
    let confirm = async |code: &str| {
        let (status, body) = post(
            &state,
            "/api/v1/me/auth/totp/confirm",
            json!({"code":code}),
            Some(&cookie),
        )
        .await;
        (status, error_code(&body).to_owned())
    };
    assert_eq!(
        confirm("12").await,
        (StatusCode::BAD_REQUEST, "totp_format".into())
    );
    assert_eq!(
        confirm(&wrong_code(&secret)).await,
        (StatusCode::BAD_REQUEST, "totp_incorrect".into())
    );
    let enrolled = totp_code(&secret, 0);
    assert_eq!(confirm(&enrolled).await.0, StatusCode::OK);
    assert_eq!(
        confirm(&totp_code(&secret, 0)).await,
        (StatusCode::CONFLICT, "totp_enabled".into())
    );

    let login = async || {
        let (_, challenge) = post(
            &state,
            "/api/v1/auth/login",
            json!({"username":"admin","password":"a good long password"}),
            None,
        )
        .await;
        assert_eq!(challenge["totp_required"], true);
        challenge["attempt"].clone()
    };
    let totp = async |attempt: &Value, code: &str| {
        let (status, body) = post(
            &state,
            "/api/v1/auth/totp",
            json!({"attempt":attempt,"code":code}),
            None,
        )
        .await;
        (status, error_code(&body).to_owned())
    };
    let attempt = login().await;
    assert_eq!(
        totp(&attempt, &enrolled).await,
        (StatusCode::BAD_REQUEST, "totp_reused".into())
    );
    for _ in 1..5 {
        assert_eq!(
            totp(&attempt, &wrong_code(&secret)).await,
            (StatusCode::BAD_REQUEST, "totp_incorrect".into())
        );
    }
    assert_eq!(
        totp(&attempt, &wrong_code(&secret)).await,
        (StatusCode::UNAUTHORIZED, "sign_in_expired".into())
    );
    assert_eq!(
        totp(&attempt, &totp_code(&secret, 1)).await,
        (StatusCode::UNAUTHORIZED, "sign_in_expired".into())
    );
    let attempt = login().await;
    for _ in 0..5 {
        totp(&attempt, &wrong_code(&secret)).await;
    }
    // Ten failed codes for the account: a new attempt is refused even with a valid code.
    let attempt = login().await;
    assert_eq!(
        totp(&attempt, &totp_code(&secret, 1)).await,
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited".into())
    );
}
#[tokio::test]
async fn failed_passwords_are_throttled_per_account_across_addresses() {
    let (_temp, state) = fixture().await;
    setup(&state).await;
    let login = async |username: &str, password: &str, address: &str| {
        request(
            &state,
            "/api/v1/auth/login",
            "POST",
            json!({"username":username,"password":password}),
            None,
            &client(address),
        )
        .await
        .0
    };
    for n in 0..10 {
        assert_eq!(
            login("ADMIN", "not the password", &format!("198.51.100.{n}")).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login("admin", "a good long password", "198.51.100.200").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login("nobody", "not the password", "198.51.100.201").await,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn password_changes_revoke_client_passwords_only_when_asked() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let (status, created) = post(
        &state,
        "/api/v1/me/auth/client-passwords",
        json!({"name":"Living room TV"}),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let client_password = created["password"].as_str().unwrap().to_owned();
    let jellyfin = async || {
        request(
            &state,
            "/Users/AuthenticateByName",
            "POST",
            json!({"Username":"admin","Pw":client_password}),
            None,
            &[(
                "x-emby-authorization",
                r#"MediaBrowser Client="Test", Device="TV", DeviceId="tv-1", Version="1""#,
            )],
        )
        .await
        .0
    };
    let change = async |password: &str, revoke: bool| {
        let (status, _, body) = request(
            &state,
            "/api/v1/me/password",
            "PUT",
            json!({"new_password":password,"revoke_client_passwords":revoke}),
            Some(&cookie),
            &client("198.51.100.7"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    };
    assert_eq!(jellyfin().await, StatusCode::OK);
    change("a second long password", false).await;
    assert_eq!(jellyfin().await, StatusCode::OK);
    change("a third long password", true).await;
    assert_eq!(jellyfin().await, StatusCode::UNAUTHORIZED);
    let (status, _, remaining) = request(
        &state,
        "/api/v1/me/auth/client-passwords",
        "GET",
        Value::Null,
        Some(&cookie),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(remaining["items"], json!([]));
}
