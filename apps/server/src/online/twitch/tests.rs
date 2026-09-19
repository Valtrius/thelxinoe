use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::{extract::Query, http::StatusCode};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Stub {
    mode: AtomicUsize,
    refreshes: AtomicUsize,
    pages: AtomicUsize,
}
async fn setup() -> (
    tempfile::TempDir,
    AppState,
    String,
    Arc<Stub>,
    tokio::task::JoinHandle<()>,
) {
    let (temp, mut state, cookie) = fixture().await;
    state
        .secrets
        .put(
            &state.db,
            "provider.twitch_client_id".into(),
            b"\"fixtureclient\"",
        )
        .await
        .unwrap();
    let control = Arc::new(Stub {
        mode: AtomicUsize::new(0),
        refreshes: AtomicUsize::new(0),
        pages: AtomicUsize::new(0),
    });
    let token_control = control.clone();
    let page_control = control.clone();
    let stub=Router::new()
        .route("/device",post(|axum::Form(form):axum::Form<BTreeMap<String,String>>|async move {
            assert_eq!(form["scopes"],SCOPE);assert_eq!(form["client_id"],"fixtureclient");
            Json(json!({"device_code":"device-secret","user_code":"ABCD","verification_uri":"https://www.twitch.tv/activate","expires_in":600,"interval":5}))
        }))
        .route("/token",post(move|axum::Form(form):axum::Form<BTreeMap<String,String>>|{let control=token_control.clone();async move {
            assert_eq!(form["client_id"],"fixtureclient");assert!(!form.contains_key("client_secret"));
            if form["grant_type"]=="refresh_token" {
                assert_eq!(form["refresh_token"],"refresh-secret");control.refreshes.fetch_add(1,Ordering::SeqCst);
                return (StatusCode::OK,Json(json!({"access_token":"new-access-secret","refresh_token":"new-refresh-secret","expires_in":3600})));
            }
            assert_eq!(form["grant_type"],"urn:ietf:params:oauth:grant-type:device_code");assert_eq!(form["device_code"],"device-secret");
            match control.mode.load(Ordering::SeqCst){
                0=>(StatusCode::BAD_REQUEST,Json(json!({"message":"authorization_pending"}))),
                2=>(StatusCode::BAD_REQUEST,Json(json!({"message":"slow_down"}))),
                3=>(StatusCode::BAD_REQUEST,Json(json!({"message":"access_denied"}))),
                _=>(StatusCode::OK,Json(json!({"access_token":"access-secret","refresh_token":"refresh-secret","expires_in":3600})))
            }
        }}))
        .route("/validate",get(|headers:HeaderMap|async move {
            assert!(matches!(headers["authorization"].to_str().unwrap(),"OAuth access-secret"|"OAuth new-access-secret"));
            Json(json!({"client_id":"fixtureclient","login":"fixtureviewer","user_id":"123","scopes":[SCOPE],"expires_in":3600}))
        }))
        .route("/streams/followed",get(move|headers:HeaderMap,Query(query):Query<BTreeMap<String,String>>|{let c=page_control.clone();async move {
            assert_eq!(headers["client-id"],"fixtureclient");assert_eq!(query["user_id"],"123");
            c.pages.fetch_add(1,Ordering::SeqCst);
            if c.mode.load(Ordering::SeqCst)==4 {return (StatusCode::TOO_MANY_REQUESTS,[("ratelimit-reset",(now()+600).to_string())],Json(json!({"message":"rate limit"})));}
            let after=query["after"].as_str();
            (StatusCode::OK,[("ratelimit-remaining","100".into())],Json(json!({"data":[{"user_id":if after.is_empty(){"42"}else{"43"},"user_login":"fixturechannel","user_name":"Fixture channel","title":"Public live","game_name":"Science","viewer_count":8,"started_at":"2026-09-19T12:00:00Z"}],"pagination":if after.is_empty(){json!({"cursor":"second"})}else{json!({})}})))
        }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let runtime = Arc::get_mut(&mut state.online).unwrap();
    runtime.http = reqwest::Client::new();
    runtime.twitch.auth = format!("http://{address}");
    runtime.twitch.api = format!("http://{address}");
    (temp, state, cookie, control, server)
}
async fn start_attempt(state: &AppState, cookie: &str) -> Attempt {
    let result = call(
        state,
        "/api/v1/online/twitch/connect",
        "POST",
        json!({}),
        cookie,
    )
    .await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.2);
    assert_eq!(result.2["user_code"], "ABCD");
    assert!(!result.2.to_string().contains("device-secret"));
    assert!(claim_attempt(state).await.unwrap().is_none());
    state
        .db
        .call(|db| {
            db.execute("UPDATE twitch_attempts SET next_poll=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    claim_attempt(state).await.unwrap().unwrap()
}
async fn ready_sync(state: &AppState) {
    state
        .db
        .call(|db| {
            db.execute("UPDATE twitch_sync SET next_run=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
}
#[tokio::test]
async fn device_connection_is_encrypted_session_bound_and_rate_limited() {
    let (_temp, state, cookie, control, server) = setup().await;
    let a = start_attempt(&state, &cookie).await;
    assert!(!String::from_utf8_lossy(&a.device).contains("device-secret"));
    let second = call(
        &state,
        "/api/v1/online/twitch/connect",
        "POST",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(second.0, StatusCode::CONFLICT);
    let another =
        thelxinoe_auth::issue_session(&state.db, "alice".into(), "web".into(), "second".into())
            .await
            .unwrap();
    let status = call(
        &state,
        "/api/v1/online/twitch",
        "GET",
        Value::Null,
        &format!("thelxinoe_session={another}"),
    )
    .await;
    assert!(status.2["pending"].is_null());
    poll(&state, &a).await.unwrap();
    assert!(claim_attempt(&state).await.unwrap().is_none());
    control.mode.store(2, Ordering::SeqCst);
    poll(&state, &a).await.unwrap();
    let interval = state
        .db
        .call(|db| {
            Ok(
                db.query_row("SELECT interval FROM twitch_attempts", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(interval, 10);
    control.mode.store(1, Ordering::SeqCst);
    poll(&state, &a).await.unwrap();
    let status = call(&state, "/api/v1/online/twitch", "GET", Value::Null, &cookie).await;
    assert_eq!(status.2["account"]["status"], "connected");
    assert!(status.2["pending"].is_null());
    assert!(!status.2.to_string().contains("secret"));
    let credential = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT credential FROM online_accounts WHERE provider='twitch'",
                [],
                |r| r.get::<_, Vec<u8>>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&credential).contains("access-secret"));
    poll(&state, &a).await.unwrap(); // Replaying the consumed attempt cannot replace its connection.
    assert!(claim_attempt(&state).await.unwrap().is_none());
    server.abort();
}
#[tokio::test]
async fn disconnect_and_revoked_session_block_late_authorization() {
    let (_temp, state, cookie, control, server) = setup().await;
    let a = start_attempt(&state, &cookie).await;
    control.mode.store(1, Ordering::SeqCst);
    assert_eq!(
        call(
            &state,
            "/api/v1/online/twitch",
            "DELETE",
            json!({}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    poll(&state, &a).await.unwrap();
    let status = call(&state, "/api/v1/online/twitch", "GET", Value::Null, &cookie).await;
    assert_eq!(status.2["account"]["status"], "disconnected");
    state
        .db
        .call(|db| {
            db.execute("UPDATE online_accounts SET updated_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let a = start_attempt(&state, &cookie).await;
    let session = a.session.clone();
    state
        .db
        .call(move |db| {
            db.execute("DELETE FROM sessions WHERE id=?1", [session])?;
            Ok(())
        })
        .await
        .unwrap();
    poll(&state, &a).await.unwrap();
    let count=state.db.call(|db|Ok(db.query_row("SELECT COUNT(*) FROM online_accounts WHERE provider='twitch' AND credential IS NOT NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap();
    assert_eq!(count, 0);
    server.abort();
}
#[tokio::test]
async fn followed_snapshot_is_private_paginated_and_refreshes_once() {
    let (_temp, state, cookie, control, server) = setup().await;
    control.mode.store(1, Ordering::SeqCst);
    let a = start_attempt(&state, &cookie).await;
    poll(&state, &a).await.unwrap();
    state
        .db
        .call(|db| {
            db.execute(
                "UPDATE online_accounts SET expires_at=0 WHERE provider='twitch'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    sync_tick(&state).await.unwrap();
    assert_eq!(control.refreshes.load(Ordering::SeqCst), 1);
    let first = call(
        &state,
        "/api/v1/online/twitch/feed",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(first.2["items"].as_array().unwrap().len(), 0);
    ready_sync(&state).await;
    sync_tick(&state).await.unwrap();
    assert_eq!(control.refreshes.load(Ordering::SeqCst), 1);
    let complete = call(
        &state,
        "/api/v1/online/twitch/feed",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(complete.2["items"].as_array().unwrap().len(), 2);
    let bob = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "bob".into())
        .await
        .unwrap();
    let bob = format!("thelxinoe_session={bob}");
    let other = call(
        &state,
        "/api/v1/online/twitch/feed",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(other.2["items"].as_array().unwrap().len(), 0);
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/twitch",
            "PUT",
            json!({"client_id":"replacement"}),
            &bob
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    control.mode.store(4, Ordering::SeqCst);
    ready_sync(&state).await;
    sync_tick(&state).await.unwrap();
    let next = state
        .db
        .call(|db| {
            Ok(db.query_row("SELECT next_run FROM twitch_sync", [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .unwrap();
    assert!(next >= now() + 590);
    let pages = control.pages.load(Ordering::SeqCst);
    sync_tick(&state).await.unwrap();
    assert_eq!(control.pages.load(Ordering::SeqCst), pages);
    call(
        &state,
        "/api/v1/online/twitch",
        "DELETE",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(
        call(
            &state,
            "/api/v1/online/twitch/feed",
            "GET",
            Value::Null,
            &cookie
        )
        .await
        .2["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    call(
        &state,
        "/api/v1/online/twitch/data",
        "DELETE",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(
        call(
            &state,
            "/api/v1/online/twitch/feed",
            "GET",
            Value::Null,
            &cookie
        )
        .await
        .2["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    server.abort();
}
