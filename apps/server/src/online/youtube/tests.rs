use super::*;
use crate::online::{
    oauth::tests::{call, fixture},
    sync::tests::{connect, stub},
};
use axum::{Json, http::StatusCode, routing::post};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn refresh_is_single_flight_and_disconnect_fences_an_inflight_result() {
    let (_temp, mut state, session) = fixture().await;
    connect(&state, "alice").await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE online_accounts SET expires_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let called = count.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let s = started.clone();
    let r = release.clone();
    let server=stub(&mut state,axum::Router::new().route("/token",post(move|axum::Form(form):axum::Form<std::collections::HashMap<String,String>>|{
        let called=called.clone();let s=s.clone();let r=r.clone();async move{
            assert_eq!(form["grant_type"],"refresh_token");assert_eq!(form["client_secret"],"test-application-secret");
            if called.fetch_add(1,Ordering::SeqCst)==0 {assert_eq!(form["refresh_token"],"refresh-fixture");}
            else {assert_eq!(form["refresh_token"],"rotated-refresh-fixture");s.notify_one();r.notified().await;}
            // Scope may be omitted when unchanged, as permitted by OAuth 2.
            Json(json!({"access_token":"new-access-fixture","refresh_token":"rotated-refresh-fixture","token_type":"Bearer","expires_in":3600}))
        }
    }))).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let state = state.clone();
        tasks.spawn(async move { access(&state, "alice").await.map(|a| a.token) });
    }
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap().unwrap(), "new-access-fixture");
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE online_accounts SET expires_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let worker = state.clone();
    let task = tokio::spawn(async move { access(&worker, "alice").await.is_ok() });
    started.notified().await;
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube",
            "DELETE",
            Value::Null,
            &session
        )
        .await
        .0,
        StatusCode::OK
    );
    release.notify_one();
    assert!(!task.await.unwrap());
    assert!(stored(&state, "alice").await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 2);
    server.abort();
}
#[tokio::test]
async fn provider_revocation_requires_reconnection_without_erasing_saved_videos() {
    let (_temp, mut state, session) = fixture().await;
    connect(&state, "alice").await;
    call(
        &state,
        "/api/v1/online/youtube/watchlist",
        "POST",
        json!({"url":"abcdefghijk"}),
        &session,
    )
    .await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE online_accounts SET expires_at=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let server=stub(&mut state,axum::Router::new().route("/token",post(||async{(StatusCode::BAD_REQUEST,Json(json!({"error":"invalid_grant","error_description":"provider detail must not escape"})))}))).await;
    let error = access(&state, "alice").await.err().unwrap();
    assert!(!error.2.contains("provider detail"));
    let mine = call(
        &state,
        "/api/v1/online/youtube",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(mine.2["account"]["status"], "reconnect_required");
    let feed = call(
        &state,
        "/api/v1/online/youtube/feed?watchlist=true",
        "GET",
        Value::Null,
        &session,
    )
    .await;
    assert_eq!(feed.2["total"], 1);
    server.abort();
}
