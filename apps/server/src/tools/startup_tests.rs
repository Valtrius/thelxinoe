use crate::test_support::call;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[tokio::test]
async fn twitch_connection_and_followed_channels_do_not_wait_for_tool_installation() {
    let (_temp, state, cookie, server) = crate::online::twitch_startup_fixture().await;
    state.tools.ready.send_replace(false);
    let response = call(
        &state,
        "/api/v1/online/twitch/connect",
        "POST",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let started = Instant::now();
    let worker_state = state.clone();
    let worker = tokio::spawn(crate::online::run(worker_state));
    let connected = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let account = call(&state, "/api/v1/online/twitch", "GET", Value::Null, &cookie)
                .await
                .2;
            let feed = call(
                &state,
                "/api/v1/online/twitch/feed",
                "GET",
                Value::Null,
                &cookie,
            )
            .await
            .2;
            if account["account"]["status"] == "connected"
                && account["sync"]["in_progress"] == false
                && feed["items"].as_array().is_some_and(|items| items.len() == 2)
            {
                eprintln!(
                    "Twitch connected and both followed channels loaded in {:.2}s with tool installation still pending",
                    started.elapsed().as_secs_f64()
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    worker.abort();
    server.abort();
    assert!(!*state.tools.ready.borrow());
    connected.expect(
        "Twitch approval and followed channels must load while playback tools are still installing",
    );
}
