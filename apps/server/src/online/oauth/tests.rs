use super::*;
use crate::{config::Config, online::Runtime};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
pub(crate) async fn call(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    cookie: &str,
) -> (StatusCode, HeaderMap, Value) {
    let request = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "internal:8484")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "media.test")
        .header("x-thelxinoe-client", "1")
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = crate::router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
pub(crate) async fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(Config {
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: temp.path().join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: Some("https://media.test".parse().unwrap()),
        trusted_proxies: vec!["127.0.0.1/32".parse().unwrap()],
        cors_origins: vec![],
        controller_socket: temp.path().join("socket"),
    })
    .await
    .unwrap();
    state
        .db
        .call(|db| {
            for name in ["alice", "bob"] {
                db.execute(
                    "INSERT INTO users VALUES (?1,?1,'unused','user','UTC',1)",
                    [name],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    state.secrets.put(&state.db,"provider.google".into(),br#"{"client_id":"test.apps.googleusercontent.com","client_secret":"test-application-secret"}"#).await.unwrap();
    let token = thelxinoe_auth::issue_session(
        &state.db,
        "alice".into(),
        "web".into(),
        "test browser".into(),
    )
    .await
    .unwrap();
    (temp, state, format!("thelxinoe_session={token}"))
}
fn attempt(response: (StatusCode, HeaderMap, Value)) -> (String, String, String) {
    assert_eq!(response.0, StatusCode::OK);
    let cookie = response.1[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly; SameSite=Lax") && cookie.contains("; Secure"));
    let url = url::Url::parse(response.2["url"].as_str().unwrap()).unwrap();
    let params = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        params["redirect_uri"],
        "https://media.test/api/v1/online/youtube/callback"
    );
    assert_eq!(params["scope"], SCOPE);
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(!response.2.to_string().contains("test-application-secret"));
    (
        params["state"].to_string(),
        cookie.split(';').next().unwrap().to_owned(),
        params["code_challenge"].to_string(),
    )
}
#[tokio::test]
async fn oauth_is_browser_bound_single_use_and_keeps_tokens_encrypted() {
    let (_temp, mut state, session) = fixture().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let challenges = Arc::new(tokio::sync::Mutex::new(String::new()));
    let count = requests.clone();
    let expected = challenges.clone();
    let stub=axum::Router::new().route("/token",post(move |axum::Form(form):axum::Form<std::collections::BTreeMap<String,String>>|{
        let count=count.clone();let expected=expected.clone();async move{
            count.fetch_add(1,Ordering::SeqCst);
            assert_eq!(form["grant_type"],"authorization_code");
            assert_eq!(form["client_secret"],"test-application-secret");
            assert_eq!(form["redirect_uri"],"https://media.test/api/v1/online/youtube/callback");
            assert_eq!(PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(form["code_verifier"].clone())).as_str(),expected.lock().await.as_str());
            Json(json!({"access_token":"viewer-access-secret","refresh_token":"viewer-refresh-secret","token_type":"Bearer","expires_in":3600,"scope":SCOPE}))
        }
    })).route("/channels",axum::routing::get(|headers:HeaderMap|async move{
        assert_eq!(headers[header::AUTHORIZATION],"Bearer viewer-access-secret");
        Json(json!({"items":[{"id":"UCfixture","snippet":{"title":"Alice's channel"}}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    state.online = Arc::new(Runtime {
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
        token: format!("http://{address}/token"),
        api: format!("http://{address}"),
        slots: tokio::sync::Semaphore::new(4),
        refresh: tokio::sync::Mutex::new(()),
        extraction: tokio::sync::Semaphore::new(2),
        streamlink: super::super::streamlink_worker::Pool::default(),
        youtube_worker: super::super::youtube_worker::Pool::default(),
        streams: crate::online::streams::Runtime::default(),
        twitch: crate::online::twitch::Runtime::default(),
        kick: crate::online::kick::Runtime::default(),
    });
    let (csrf, browser, challenge) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    *challenges.lock().await = challenge;
    let path =
        format!("/api/v1/online/youtube/callback?state={csrf}&code=authorization-code-secret");
    let wrong = call(
        &state,
        &path,
        "GET",
        Value::Null,
        "thelxinoe_youtube_oauth=wrong",
    )
    .await;
    assert_eq!(wrong.1[header::LOCATION], "/?youtube_link=failed");
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    // Google's return carries the Lax binding cookie, without the Strict login cookie.
    let completed = call(&state, &path, "GET", Value::Null, &browser).await;
    assert_eq!(completed.1[header::LOCATION], "/?youtube_link=connected");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let mine = call(
        &state,
        "/api/v1/online/youtube",
        "GET",
        Value::Null,
        &session,
    )
    .await
    .2;
    assert_eq!(mine["account"]["display_name"], "Alice's channel");
    assert_eq!(mine["account"]["status"], "connected");
    assert!(!mine.to_string().contains("secret"));
    let encrypted = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT credential FROM online_accounts WHERE user_id='alice'",
                [],
                |r| r.get::<_, Vec<u8>>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("viewer-"));
    assert!(
        state
            .secrets
            .decrypt("online:youtube:bob", &encrypted)
            .is_err()
    );
    let replay = call(&state, &path, "GET", Value::Null, &browser).await;
    assert_eq!(replay.1[header::LOCATION], "/?youtube_link=failed");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let (csrf, browser, _) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    call(
        &state,
        "/api/v1/online/youtube",
        "DELETE",
        Value::Null,
        &session,
    )
    .await;
    let canceled = call(
        &state,
        &format!("/api/v1/online/youtube/callback?state={csrf}&code=unused"),
        "GET",
        Value::Null,
        &browser,
    )
    .await;
    assert_eq!(canceled.1[header::LOCATION], "/?youtube_link=failed");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let (csrf, browser, _) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    state
        .db
        .call(|db| {
            db.execute("DELETE FROM sessions WHERE user_id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let revoked = call(
        &state,
        &format!("/api/v1/online/youtube/callback?state={csrf}&code=unused"),
        "GET",
        Value::Null,
        &browser,
    )
    .await;
    assert_eq!(revoked.1[header::LOCATION], "/?youtube_link=failed");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}
#[tokio::test]
async fn concurrent_requests_cannot_overspend_shared_quota() {
    let (_temp, state, _session) = fixture().await;
    state
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO settings VALUES ('youtube_daily_quota','7')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..24 {
        let state = state.clone();
        tasks.spawn(async move {
            quota::reserve(&state, if i % 2 == 0 { "alice" } else { "bob" })
                .await
                .is_ok()
        });
    }
    let mut allowed = 0;
    while let Some(result) = tasks.join_next().await {
        allowed += usize::from(result.unwrap());
    }
    assert_eq!(allowed, 7);
    assert_eq!(quota::status(&state).await.unwrap()["used"], 7);
}

#[tokio::test]
async fn replacement_expiry_and_application_changes_cancel_authorization_attempts() {
    let (_temp, mut state, session) = fixture().await;
    crate::online::sync::tests::connect(&state, "alice").await;
    let count = Arc::new(AtomicUsize::new(0));
    let requests = count.clone();
    let server = crate::online::sync::tests::stub(
        &mut state,
        axum::Router::new().route(
            "/token",
            post(move || {
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error":"invalid_grant"})),
                    )
                }
            }),
        ),
    )
    .await;
    let (old, browser, _) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    let (current, current_browser, _) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    let response = call(
        &state,
        &format!("/api/v1/online/youtube/callback?state={old}&code=unused"),
        "GET",
        Value::Null,
        &browser,
    )
    .await;
    assert_eq!(response.1[header::LOCATION], "/?youtube_link=failed");
    // Abandoning a reconnect attempt still leaves the existing connection's
    // scheduler generation in step with its account.
    assert!(state.db.call(|db|Ok(db.query_row("SELECT y.generation=a.generation FROM youtube_sync y JOIN online_accounts a USING(user_id) WHERE a.provider='youtube'",[],|r|r.get::<_,bool>(0))?)).await.unwrap());
    state
        .db
        .call(|db| {
            db.execute("UPDATE oauth_attempts SET expires_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let expired = call(
        &state,
        &format!("/api/v1/online/youtube/callback?state={current}&code=unused"),
        "GET",
        Value::Null,
        &current_browser,
    )
    .await;
    assert_eq!(expired.1[header::LOCATION], "/?youtube_link=failed");
    let (current, browser, _) = attempt(
        call(
            &state,
            "/api/v1/online/youtube/connect",
            "POST",
            json!({}),
            &session,
        )
        .await,
    );
    state
        .db
        .call(|db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let configured=call(&state,"/api/v1/admin/online","PUT",json!({"google":{"client_id":"replacement.apps.googleusercontent.com","client_secret":"replacement-fixture"},"youtube_downloads":true,"youtube_daily_quota":10000}),&session).await;
    assert_eq!(configured.0, StatusCode::OK);
    let changed = call(
        &state,
        &format!("/api/v1/online/youtube/callback?state={current}&code=unused"),
        "GET",
        Value::Null,
        &browser,
    )
    .await;
    assert_eq!(changed.1[header::LOCATION], "/?youtube_link=failed");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let account = call(
        &state,
        "/api/v1/online/youtube",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(account.2["account"]["status"], "reconnect_required");
    assert!(
        state
            .db
            .call(|db| Ok(db.query_row(
                "SELECT credential IS NULL FROM online_accounts WHERE user_id='alice'",
                [],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap()
    );
    server.abort();
}
