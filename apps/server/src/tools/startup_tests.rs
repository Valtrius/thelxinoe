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

#[tokio::test]
async fn playback_reports_preparing_tools_to_viewers_and_stops_waiting_after_failure() {
    let (_temp, state, cookie, server) = crate::online::twitch_startup_fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES ('alice','42','fixture','Fixture','Live fixture','Science',10,'today','s',1)", [])?;
        db.execute("INSERT INTO tool_jobs(id,tool,action,manual,revision,stage,created_at,updated_at) VALUES ('install-streamlink','streamlink','bootstrap',0,0,'queued',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let metadata = call(
        &state,
        "/api/v1/catalog/twitch:42/playback",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(
        metadata.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        metadata.2
    );
    assert_eq!(metadata.2["error"]["code"], "tools_preparing");
    state.db.write("test.fixture", |db| {
        db.execute("UPDATE tool_jobs SET stage='failed',error='Download failed' WHERE id='install-streamlink'", [])?;
        Ok(())
    }).await.unwrap();
    let failed = call(
        &state,
        "/api/v1/catalog/twitch:42/playback",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(failed.0, StatusCode::CONFLICT, "{}", failed.2);
    assert_ne!(failed.2["error"]["code"], "tools_preparing");
    assert!(
        failed.2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Settings")
    );
    server.abort();
}
