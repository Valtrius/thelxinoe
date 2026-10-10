use super::*;
use crate::test_support::{call, fixture};
use axum::http::StatusCode;

async fn admin(state: &AppState, window: (u8, u8)) {
    state
        .db
        .write("test.backup_admin", move |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            db.execute(
                "INSERT INTO settings VALUES ('product.policy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json!({"policy":"notify","window_start":window.0,"window_end":window.1}).to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}
// Equal hours keep the server window open regardless of the test clock.
const ALWAYS: (u8, u8) = (4, 4);
async fn enable(state: &AppState, cookie: &str) {
    let response = call(
        state,
        "/api/v1/admin/backups/policy",
        "POST",
        json!({"policy":"automatic","retain":7}),
        cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
}
fn controller(state: &AppState, items: Value) {
    state.managers.docker.lock().unwrap().insert(
        "stack/backups".into(),
        json!({"items":items,"destination":"/backups"}),
    );
}
async fn current(state: &AppState) -> Run {
    storage::run(&state.db).await.unwrap().unwrap()
}
async fn attention(state: &AppState, cookie: &str) -> Vec<String> {
    call(state, "/api/v1/me/attention", "GET", Value::Null, cookie)
        .await
        .2["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn automatic_backup_runs_once_per_window_then_releases_updates() {
    let (_temp, state, cookie) = fixture().await;
    admin(&state, ALWAYS).await;
    controller(&state, json!([]));
    // Manual is the default: nothing runs, holds updates or asks for a passphrase.
    let listed = call(&state, "/api/v1/admin/backups", "GET", Value::Null, &cookie).await;
    assert_eq!(listed.2["policy"], json!({"policy":"manual","retain":7}));
    assert!(attention(&state, &cookie).await.is_empty());
    enable(&state, &cookie).await;
    // Automatic backups neither run nor hold updates until a passphrase is saved.
    tick(&state).await.unwrap();
    assert!(storage::run(&state.db).await.unwrap().is_none());
    assert!(!holds_updates(&state).await.unwrap());
    assert!(
        attention(&state, &cookie)
            .await
            .contains(&"backup-passphrase".into())
    );
    let secret = "an automatic backup passphrase";
    for (passphrase, status) in [
        ("too short", StatusCode::BAD_REQUEST),
        (secret, StatusCode::OK),
    ] {
        let response = call(
            &state,
            "/api/v1/admin/backups/passphrase",
            "POST",
            json!({"passphrase":passphrase}),
            &cookie,
        )
        .await;
        assert_eq!(response.0, status, "{}", response.2);
    }
    assert!(
        !attention(&state, &cookie)
            .await
            .contains(&"backup-passphrase".into())
    );
    assert!(holds_updates(&state).await.unwrap());
    tick(&state).await.unwrap();
    let run = current(&state).await;
    assert_eq!(run.state, "running");
    for (stage, expected, held) in [
        ("snapshotting", "running", true),
        ("complete", "complete", false),
    ] {
        controller(
            &state,
            json!([{"id":run.id,"stage":stage,"automatic":true}]),
        );
        tick(&state).await.unwrap();
        assert_eq!(current(&state).await.state, expected);
        assert_eq!(holds_updates(&state).await.unwrap(), held);
    }
    // The same window never starts a second backup.
    controller(&state, json!([]));
    tick(&state).await.unwrap();
    let finished = current(&state).await;
    assert_eq!((finished.id, finished.state.as_str()), (run.id, "complete"));
    let listed = call(&state, "/api/v1/admin/backups", "GET", Value::Null, &cookie).await;
    assert_eq!(listed.0, StatusCode::OK, "{}", listed.2);
    assert_eq!(listed.2["policy"], json!({"policy":"automatic","retain":7}));
    assert_eq!(listed.2["passphrase_saved"], true);
    assert_eq!(listed.2["automatic"]["state"], "complete");
    assert_eq!(listed.2["window"], json!({"start":4,"end":4}));
    assert!(!listed.2.to_string().contains(secret));
}

#[tokio::test]
async fn busy_or_closed_windows_skip_the_backup_and_release_updates() {
    for closed in [false, true] {
        let (_temp, state, cookie) = fixture().await;
        admin(&state, ALWAYS).await;
        enable(&state, &cookie).await;
        controller(&state, json!([]));
        state
            .secrets
            .put(
                &state.db,
                PASSPHRASE.into(),
                b"an automatic backup passphrase",
            )
            .await
            .unwrap();
        state.db.write("test.backup_busy", |db| {
            db.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES ('busy','library.scan','{}','busy','running',1,1)",[])?;
            Ok(())
        }).await.unwrap();
        tick(&state).await.unwrap();
        assert_eq!(current(&state).await.state, "waiting");
        assert!(holds_updates(&state).await.unwrap());
        if closed {
            let hour = chrono::Timelike::hour(&chrono::Utc::now()) as u8;
            admin(&state, ((hour + 6) % 24, (hour + 7) % 24)).await;
        } else {
            let mut run = current(&state).await;
            run.since -= IDLE_GRACE;
            storage::save_run(&state.db, run).await.unwrap();
        }
        tick(&state).await.unwrap();
        let run = current(&state).await;
        assert_eq!(run.state, "missed");
        assert!(run.error.is_some());
        assert!(!holds_updates(&state).await.unwrap());
        assert!(
            attention(&state, &cookie)
                .await
                .contains(&"backup-automatic".into())
        );
    }
}

#[tokio::test]
async fn manual_policy_disables_scheduling_and_keeps_manual_backups() {
    let (_temp, state, cookie) = fixture().await;
    admin(&state, ALWAYS).await;
    controller(&state, json!([]));
    for (policy, retain, status) in [
        ("automatic", 0, StatusCode::BAD_REQUEST),
        ("automatic", 366, StatusCode::BAD_REQUEST),
        ("daily", 7, StatusCode::BAD_REQUEST),
        ("manual", 3, StatusCode::OK),
    ] {
        let response = call(
            &state,
            "/api/v1/admin/backups/policy",
            "POST",
            json!({"policy":policy,"retain":retain}),
            &cookie,
        )
        .await;
        assert_eq!(response.0, status, "{}", response.2);
    }
    assert!(
        !attention(&state, &cookie)
            .await
            .contains(&"backup-passphrase".into())
    );
    state
        .secrets
        .put(
            &state.db,
            PASSPHRASE.into(),
            b"an automatic backup passphrase",
        )
        .await
        .unwrap();
    tick(&state).await.unwrap();
    assert!(storage::run(&state.db).await.unwrap().is_none());
    assert!(!holds_updates(&state).await.unwrap());
    for (confirm, status) in [(false, StatusCode::BAD_REQUEST), (true, StatusCode::OK)] {
        let response = call(
            &state,
            "/api/v1/admin/backups",
            "POST",
            json!({"passphrase":"a manual backup passphrase","confirm":confirm}),
            &cookie,
        )
        .await;
        assert_eq!(response.0, status, "{}", response.2);
    }
}
