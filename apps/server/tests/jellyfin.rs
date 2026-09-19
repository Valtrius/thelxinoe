use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use thelxinoe_server::{AppState, config::Config, router};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(Config {
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: temp.path().join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec![],
        controller_socket: temp.path().join("socket"),
    })
    .await
    .unwrap();
    let hash = thelxinoe_auth::password_hash("a long test-only password".into())
        .await
        .unwrap();
    state
        .db
        .call(move |db| {
            for name in ["alice", "bob"] {
                db.execute(
                    "INSERT INTO users VALUES (?1,?1,?2,'user','UTC',1)",
                    rusqlite::params![name, hash],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let token = thelxinoe_auth::issue_session(
        &state.db,
        "alice".into(),
        "device".into(),
        "Browser test".into(),
    )
    .await
    .unwrap();
    (temp, state, token)
}
async fn call(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
const DEVICE: &str = "MediaBrowser Client=\"Test%20client\", Device=\"Living room, TV\", DeviceId=\"tv-one\", Version=\"1\"";
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
        StatusCode::UNAUTHORIZED
    );
}
