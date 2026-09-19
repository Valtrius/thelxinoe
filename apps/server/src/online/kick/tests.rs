use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::{extract::Query, http::StatusCode};
use std::{collections::BTreeMap, sync::Arc};
#[test]
fn channel_input_cannot_supply_a_foreign_url_or_path() {
    assert_eq!(slug("https://kick.com/Example").unwrap(), "example");
    for v in [
        "https://localhost/private",
        "https://kick.com.attacker.test/a",
        "https://user@kick.com/a",
        "https://kick.com/a/b",
        "a?x=1",
        "../a",
    ] {
        assert!(slug(v).is_err());
    }
}
#[tokio::test]
async fn tracking_and_metadata_are_private_and_removed_generation_stays_removed() {
    let (_temp, mut state, alice) = fixture().await;
    state
        .secrets
        .put(
            &state.db,
            "provider.kick".into(),
            br#"{"client_id":"fixture","client_secret":"application-secret"}"#,
        )
        .await
        .unwrap();
    let stub=Router::new().route("/token",post(|axum::Form(form):axum::Form<BTreeMap<String,String>>|async move{assert_eq!(form["client_secret"],"application-secret");Json(json!({"access_token":"app-token-secret","expires_in":3600}))}))
        .route("/channels",get(|headers:HeaderMap,Query(query):Query<BTreeMap<String,String>>|async move{assert_eq!(headers["authorization"],"Bearer app-token-secret");assert_eq!(query["slug"],"fixture");Json(json!({"data":[{"slug":"fixture","stream_title":"Live title","category":{"name":"Science"},"stream":{"is_live":true,"viewer_count":123,"key":"never-expose-stream-key"}}]}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let runtime = Arc::get_mut(&mut state.online).unwrap();
    runtime.http = reqwest::Client::new();
    runtime.kick.api = format!("http://{address}");
    runtime.kick.token_url = format!("http://{address}/token");
    assert_eq!(
        call(
            &state,
            "/api/v1/online/kick/channels",
            "POST",
            json!({"channel":"https://kick.com/fixture"}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    tick(&state).await.unwrap();
    let feed = call(&state, "/api/v1/online/kick", "GET", Value::Null, &alice).await;
    assert_eq!(feed.2["items"][0]["live"], true);
    assert_eq!(feed.2["items"][0]["viewers"], 123);
    assert!(!feed.2.to_string().contains("secret"));
    assert!(!feed.2.to_string().contains("stream-key"));
    let bob = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
        .await
        .unwrap();
    let bob = format!("thelxinoe_session={bob}");
    assert_eq!(
        call(&state, "/api/v1/online/kick", "GET", Value::Null, &bob)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/online/kick",
            "PUT",
            json!({"client_id":"fixture","client_secret":"replace"}),
            &bob
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let t=state.db.call(|db|Ok(db.query_row("SELECT c.generation,a.generation FROM kick_channels c JOIN online_accounts a ON a.user_id=c.user_id AND a.provider='kick'",[],|r|Ok(Turn{user:"alice".into(),slug:"fixture".into(),generation:r.get(0)?,account:r.get(1)?,failures:0}))?)).await.unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/online/kick/channels/fixture",
            "DELETE",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    step(&state, &t).await.unwrap();
    assert_eq!(
        call(&state, "/api/v1/online/kick", "GET", Value::Null, &alice)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    call(
        &state,
        "/api/v1/online/kick/channels",
        "POST",
        json!({"channel":"fixture"}),
        &alice,
    )
    .await;
    step(&state, &t).await.unwrap();
    assert!(
        call(&state, "/api/v1/online/kick", "GET", Value::Null, &alice)
            .await
            .2["items"][0]["live"]
            .is_null()
    );
    call(&state, "/api/v1/online/kick", "DELETE", json!({}), &alice).await;
    tick(&state).await.unwrap();
    let feed = call(&state, "/api/v1/online/kick", "GET", Value::Null, &alice).await;
    assert_eq!(feed.2["connected"], false);
    assert_eq!(feed.2["items"].as_array().unwrap().len(), 1);
    call(
        &state,
        "/api/v1/online/kick/data",
        "DELETE",
        json!({}),
        &alice,
    )
    .await;
    assert_eq!(
        call(&state, "/api/v1/online/kick", "GET", Value::Null, &alice)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    server.abort();
}
