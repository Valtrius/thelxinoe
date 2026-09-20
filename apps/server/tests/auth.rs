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
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: temp.path().join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec!["10.0.0.0/24".parse().unwrap()],
        cors_origins: vec![],
        controller_socket: temp.path().join("socket"),
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
    let token = std::fs::read_to_string(state.config.state.join("secrets/setup-token")).unwrap();
    let (status, headers, _) = request(
        state,
        "/api/v1/setup",
        "POST",
        json!({"username":"admin","password":"a good long password","setup_token":token}),
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
async fn first_admin_is_atomic_and_sessions_survive_restart_and_revoke() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let token = std::fs::read_to_string(state.config.state.join("secrets/setup-token")).unwrap();
    assert_eq!(
        request(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"other","password":"a good long password","setup_token":token}),
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
async fn metadata_credentials_are_redacted_and_manual_fields_win_after_provider_refresh() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    let secret = "tmdb-test-secret-that-must-never-be-returned";
    assert_eq!(
        request(
            &state,
            "/api/v1/admin/metadata",
            "PUT",
            json!({"tmdb_token":secret}),
            Some(&cookie),
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    let config = request(
        &state,
        "/api/v1/admin/metadata",
        "GET",
        Value::Null,
        Some(&cookie),
        &[],
    )
    .await
    .2;
    assert_eq!(config["tmdb_configured"], true);
    assert!(!config.to_string().contains(secret));
    state.db.call(|db|{db.execute_batch("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Fixture','movies','/fixture'); INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES ('movie','root','movie','fixture','Local title','{\"title\":\"Initial provider title\"}',0);")?;Ok(())}).await.unwrap();
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
        .call(|db| {
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
async fn episode_provider_numbering_is_explicit_and_complex_mappings_are_marked() {
    let (_temp, state) = fixture().await;
    let cookie = setup(&state).await;
    state.db.call(|db|{db.execute_batch("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','TV','shows','/tv');
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('show','r','show',NULL,'show','Show',NULL,0),('season','r','season','show','s','Season',1,0),('e1','r','episode','season','e1','First',1,0),('e2','r','episode','season','e2','Second',2,0);
INSERT INTO provider_ids(media_id,provider,external_id) VALUES ('show','tmdb','100');
INSERT INTO provider_episodes VALUES ('tmdb','100','201',1,2,'{}'),('tmdb','100','202',1,1,'{}');")?;Ok(())}).await.unwrap();
    assert_eq!(
        state
            .db
            .call(|db| Ok(
                db.query_row("SELECT count(*) FROM episode_mappings", [], |r| r
                    .get::<_, i64>(0))?
            ))
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
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM episode_mappings WHERE state='complex'",
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
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM jobs WHERE kind='library.scan'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        8
    );
}
