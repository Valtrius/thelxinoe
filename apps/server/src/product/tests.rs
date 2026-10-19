use super::*;
use crate::test_support::{call, fixture};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn maintenance_survives_restart_and_requires_settled_controller_evidence() {
    let (_temp, state, _cookie) = fixture().await;
    save(&state, "product.maintenance", json!(true))
        .await
        .unwrap();
    // AppState::open reads the same persisted value before workers or HTTP start.
    assert!(maintenance_pending(&state.db).await.unwrap());
    state.release_quiescing.store(true, Ordering::SeqCst);
    for (observation, held) in [
        (json!({"busy":true,"items":[]}), true),
        (json!({"busy":false,"items":[{"stage":"preparing"}]}), true),
        (
            json!({"busy":false,"items":[{"stage":"snapshotting"}]}),
            true,
        ),
        (json!({"busy":false,"items":[{"stage":"handoff"}]}), true),
        (json!({"busy":false,"items":[{"stage":"ready"}]}), false),
    ] {
        state
            .managers
            .docker
            .lock()
            .unwrap()
            .insert("stack/product".into(), observation);
        reconcile_maintenance(&state).await.unwrap();
        assert_eq!(state.release_quiescing.load(Ordering::SeqCst), held);
        assert_eq!(maintenance_pending(&state.db).await.unwrap(), held);
    }
}

#[tokio::test]
async fn lost_command_acknowledgement_keeps_admission_closed_until_reconciled() {
    let (_temp, state, cookie) = fixture().await;
    // The fixture has no controller socket: the command has an uncertain outcome.
    assert!(
        quiesced_command(&state, "/product/preflight", Some(json!({})))
            .await
            .is_err()
    );
    assert!(maintenance_pending(&state.db).await.unwrap());
    assert!(state.release_quiescing.load(Ordering::SeqCst));
    let (status, _, body) =
        call(&state, "/api/v1/auth/sessions", "GET", Value::Null, &cookie).await;
    assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "maintenance");
    // An unavailable or malformed observation cannot reopen the gate.
    assert!(reconcile_maintenance(&state).await.is_err());
    assert!(state.release_quiescing.load(Ordering::SeqCst));
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("stack/product".into(), json!({"items":[]}));
    assert!(reconcile_maintenance(&state).await.is_err());
    assert!(state.release_quiescing.load(Ordering::SeqCst));
    // Only a complete journal with a free mutation gate establishes rejection.
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("stack/product".into(), json!({"busy":false,"items":[]}));
    reconcile_maintenance(&state).await.unwrap();
    assert!(!state.release_quiescing.load(Ordering::SeqCst));
    assert!(!maintenance_pending(&state.db).await.unwrap());
    assert_eq!(
        call(&state, "/api/v1/auth/sessions", "GET", Value::Null, &cookie)
            .await
            .0,
        axum::http::StatusCode::OK
    );
}

#[tokio::test]
async fn unrelated_service_work_does_not_enable_release_maintenance() {
    let (_temp, state, _cookie) = fixture().await;
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("stack/product".into(), json!({"busy":true,"items":[]}));
    reconcile_maintenance(&state).await.unwrap();
    assert!(!state.release_quiescing.load(Ordering::SeqCst));
}

#[test]
fn preflight_is_invalidated_by_recreation_even_when_the_version_is_unchanged() {
    let operation = json!({"version":"0.1.1","stage":"ready","source_generation":3});
    assert!(current_preflight(&operation, &json!({"generation":3})));
    assert!(!current_preflight(&operation, &json!({"generation":4})));
    assert!(!current_preflight(&operation, &json!({})));
}

// Settings pages poll the status every few seconds; they read the controller
// state the operations loop records instead of asking the controller each time.
#[tokio::test]
async fn status_reports_the_recorded_controller_state() {
    let (_temp, state, cookie) = fixture().await;
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let controller = |id: &str| {
        state.managers.docker.lock().unwrap().insert(
            "stack/product".into(),
            json!({"items":[{"id":id,"stage":"ready"}]}),
        );
    };
    async fn reported(state: &AppState, cookie: &str) -> Value {
        let (status, _, body) = call(
            state,
            "/api/v1/admin/product-update",
            "GET",
            Value::Null,
            cookie,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        body["controller"].clone()
    }
    // Before the first record the status asks once.
    controller("first");
    assert_eq!(reported(&state, &cookie).await["items"][0]["id"], "first");
    controller("second");
    assert_eq!(reported(&state, &cookie).await["items"][0]["id"], "first");
    observe(&state).await.unwrap();
    assert_eq!(reported(&state, &cookie).await["items"][0]["id"], "second");
    state.managers.docker.lock().unwrap().insert(
        "stack/product".into(),
        json!({"__error":["controller_unavailable","Controller did not respond"]}),
    );
    assert!(observe(&state).await.is_err());
    let unavailable = reported(&state, &cookie).await;
    assert_eq!(unavailable["error"], "Controller unavailable");
    assert_eq!(unavailable["items"], json!([]));
}

// Automatic mode retries a blocked preparation after a growing pause and never
// retries a release that failed validation or activation.
#[test]
fn automatic_attempts_back_off_and_count_each_blocked_attempt_once() {
    assert_eq!(automatic_backoff(1), 15 * 60);
    assert_eq!(automatic_backoff(2), 30 * 60);
    assert_eq!(automatic_backoff(9), 6 * 3600);
    let mut automatic = Automatic {
        version: "0.0.2".into(),
        ..Default::default()
    };
    assert!(automatic_due(&mut automatic, &[], 1000));
    let blocked = |id: &str| json!({"id":id,"version":"0.0.2","stage":"blocked","error":"Image registry unavailable"});
    let items = [
        blocked("first"),
        json!({"id":"old","version":"0.0.1","stage":"recovered"}),
    ];
    assert!(!automatic_due(&mut automatic, &items, 1000));
    assert_eq!(
        (automatic.failures, automatic.retry_at),
        (1, 1000 + 15 * 60)
    );
    assert_eq!(
        automatic.error.as_deref(),
        Some("Image registry unavailable")
    );
    // Seeing the same attempt again does not extend the pause.
    assert!(!automatic_due(&mut automatic, &items, 1500));
    assert!(automatic_due(&mut automatic, &items, 1000 + 15 * 60));
    assert_eq!(automatic.failures, 1);
    let items = [blocked("second"), blocked("first")];
    assert!(!automatic_due(&mut automatic, &items, 5000));
    assert_eq!(
        (automatic.failures, automatic.retry_at),
        (2, 5000 + 30 * 60)
    );
    let failed = [json!({"id":"third","version":"0.0.2","stage":"runtime-failure"})];
    assert!(!automatic_due(&mut automatic, &failed, 1_000_000));
}

#[test]
fn automatic_attempts_follow_the_failure_class_and_stop_after_repeated_failures() {
    let fresh = || Automatic {
        version: "0.0.3".into(),
        ..Default::default()
    };
    let failed = |id: &str, stage: &str, failure: Value| json!({"id":id,"version":"0.0.3","stage":stage,"error":"Docker inspection unavailable","failure":failure});
    // Validation and compatibility fail the same way on the same data, as does
    // an activation older controllers recovered without saying why.
    for (stage, failure) in [
        ("blocked", json!("validation")),
        ("blocked", json!("incompatible")),
        ("recovered", json!("validation")),
        ("recovered", Value::Null),
    ] {
        let mut automatic = fresh();
        assert!(!automatic_due(
            &mut automatic,
            &[failed("failed", stage, failure)],
            1_000_000
        ));
        assert!(automatic.held);
        assert_eq!(automatic.failures, 0);
    }
    // A restart during preparation or activation is retried soon without counting.
    for stage in ["blocked", "recovered"] {
        let mut automatic = fresh();
        let interrupted = [failed("restart", stage, json!("interrupted"))];
        assert!(!automatic_due(&mut automatic, &interrupted, 1000));
        assert_eq!((automatic.failures, automatic.retry_at), (0, 1060));
        assert!(automatic_due(&mut automatic, &interrupted, 1060));
        assert!(
            automatic
                .error
                .as_deref()
                .unwrap()
                .contains("check that Docker is running")
        );
    }
    // Environment failures, also unclassified ones older controllers report as
    // blocked, back off and stop after a few attempts.
    let mut automatic = fresh();
    let mut now = 0;
    for attempt in 1..=AUTOMATIC_ATTEMPTS {
        let (stage, failure) = match attempt % 3 {
            0 => ("blocked", Value::Null),
            1 => ("blocked", json!("environment")),
            _ => ("recovered", json!("environment")),
        };
        let items = [failed(&format!("attempt-{attempt}"), stage, failure)];
        assert!(!automatic_due(&mut automatic, &items, now));
        now = automatic.retry_at;
        assert_eq!(
            automatic_due(&mut automatic, &items, now),
            attempt < AUTOMATIC_ATTEMPTS
        );
    }
    assert!(automatic.held);
}

// Validation works on copies while the restarted server runs; only a busy gate
// without that journal may still be an unrecorded command.
#[test]
fn validation_runs_without_the_maintenance_fence() {
    let fenced = |observed: Value| controller_requires_maintenance(&observed).unwrap();
    assert!(!fenced(
        json!({"busy":true,"items":[{"stage":"validating"}]})
    ));
    assert!(fenced(json!({"busy":true,"items":[{"stage":"ready"}]})));
    assert!(fenced(
        json!({"busy":false,"items":[{"stage":"preparing-activation"}]})
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn a_slow_controller_does_not_hold_the_request_gate() {
    let (_temp, state, _cookie) = fixture().await;
    let listener = tokio::net::UnixListener::bind(&state.config.controller_socket).unwrap();
    let command = tokio::spawn({
        let state = state.clone();
        async move { quiesced_command(&state, "/product/preflight", Some(json!({}))).await }
    });
    let (connection, _) = listener.accept().await.unwrap();
    // The fence holds while the controller has not answered; requests still pass the gate.
    assert!(state.release_quiescing.load(Ordering::SeqCst));
    assert!(state.release_gate.try_read().is_ok());
    drop((connection, listener));
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), command).await;
    assert!(result.unwrap().unwrap().is_err());
}

fn manifest(version: &str) -> Value {
    json!({
        "format":1,"version":version,"published_at":now()-60,"expires_at":now()+86400,
        "server":{"reference":format!("registry.example/thelxinoe/server@sha256:{}","a".repeat(64))},
        "controller":{"reference":format!("registry.example/thelxinoe/controller@sha256:{}","b".repeat(64))},
        "windows_x64":{"url":"https://example.test/thelxinoe.exe","sha256":"c".repeat(64),"bytes":1,"signature":"signature","updater_public_key":"key"},
        "api":{"min":1,"max":1},
        "migration":{"from":{"min":0,"max":thelxinoe_database::SCHEMA_VERSION},"target":thelxinoe_database::SCHEMA_VERSION,"recovery":"full-state-restore","recovery_protocol":1},
        "notes":""
    })
}
fn script(state: &AppState, path: &str, value: Value) {
    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert(format!("stack{path}"), value);
}
async fn admin(state: &AppState) {
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
}
async fn queue(state: &AppState, version: &str) -> Value {
    let intent = json!({"id":thelxinoe_core::id(),"version":version,"previous_version":thelxinoe_core::VERSION,"release":manifest(version),"envelope":{"payload":"pinned","signature":"pinned"},"state":"pending","error":null});
    save(state, "product.install", intent.clone())
        .await
        .unwrap();
    intent
}
async fn intent(state: &AppState) -> Value {
    setting(state, "product.install").await.unwrap().unwrap()
}
/// Lets the next tick try again at once, as if the pause had passed.
async fn resume(state: &AppState) {
    let mut request = intent(state).await;
    request["retry_at"] = Value::Null;
    save(state, "product.install", request).await.unwrap();
    state.release_quiescing.store(false, Ordering::SeqCst);
}

// One click must survive a busy registry, Docker or controller outage, a restart
// and a newer discovery; only a definitive refusal fails it.
#[tokio::test]
async fn queued_installs_wait_through_transient_failures_and_keep_their_release() {
    let (_temp, state, cookie) = fixture().await;
    admin(&state).await;
    script(
        &state,
        "/product",
        json!({"busy":false,"generation":1,"items":[]}),
    );
    let queued = queue(&state, "99.0.0").await;
    // An older controller lacks the download endpoint; its preflight is unreachable here.
    script(
        &state,
        "/product/download",
        json!({"__error":["not_found","Not found"]}),
    );
    assert!(advance_install(&state).await.unwrap());
    let waiting = intent(&state).await;
    assert_eq!(waiting["state"], "pending");
    assert!(
        waiting["waiting"]
            .as_str()
            .unwrap()
            .contains("Docker controller")
    );
    assert!(waiting["retry_at"].as_i64().unwrap() > now());
    resume(&state).await;
    // Downloads run unfenced, and the status reports their progress.
    script(
        &state,
        "/product/download",
        json!({"state":"downloading","download":{"component":"server","received":5,"total":10}}),
    );
    assert!(advance_install(&state).await.unwrap());
    assert!(!state.release_quiescing.load(Ordering::SeqCst));
    assert_eq!(intent(&state).await["waiting"], Value::Null);
    let (_, _, status) = call(
        &state,
        "/api/v1/admin/product-update",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(status["download"]["download"]["received"], 5);
    assert!(status["request"].get("envelope").is_none());
    script(
        &state,
        "/product/download",
        json!({"state":"failed","error":"Docker is not reachable"}),
    );
    assert!(advance_install(&state).await.unwrap());
    let waiting = intent(&state).await;
    assert_eq!(waiting["state"], "pending");
    assert!(
        waiting["waiting"]
            .as_str()
            .unwrap()
            .starts_with("Downloading the update failed: The Docker controller couldn't")
    );
    // A newer discovery leaves the pinned release queued.
    resume(&state).await;
    save(&state, "product.release", manifest("99.0.1"))
        .await
        .unwrap();
    script(&state, "/product/download", json!({"state":"ready"}));
    script(
        &state,
        "/product/preflight",
        json!({"id":queued["id"],"stage":"preparing"}),
    );
    assert!(advance_install(&state).await.unwrap());
    assert!(state.release_quiescing.load(Ordering::SeqCst));
    assert_eq!(intent(&state).await["state"], "pending");
    assert_eq!(intent(&state).await["version"], "99.0.0");
    // A restart during preparation qualifies the same release again.
    script(
        &state,
        "/product",
        json!({"busy":false,"generation":1,"items":[{"id":queued["id"],"stage":"blocked","failure":"interrupted","source_generation":1}]}),
    );
    resume(&state).await;
    assert!(advance_install(&state).await.unwrap());
    assert_ne!(intent(&state).await["id"], queued["id"]);
    assert_eq!(intent(&state).await["state"], "pending");
    script(
        &state,
        "/product",
        json!({"busy":false,"generation":1,"items":[]}),
    );
    resume(&state).await;
    script(
        &state,
        "/product/preflight",
        json!({"__error":["conflict","First-party drift blocks release preparation"]}),
    );
    assert!(advance_install(&state).await.unwrap());
    let failed = intent(&state).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(
        failed["error"],
        "First-party drift blocks release preparation"
    );
    // A Compose update to the queued version completes the request.
    queue(&state, thelxinoe_core::VERSION).await;
    assert!(advance_install(&state).await.unwrap());
    assert_eq!(intent(&state).await["state"], "completed");
}

#[tokio::test]
async fn queued_installs_can_be_cancelled_until_the_server_is_fenced() {
    let (_temp, state, cookie) = fixture().await;
    admin(&state).await;
    let queued = queue(&state, "99.0.0").await;
    let journal = |stage: &str| json!({"busy":true,"generation":1,"items":[{"id":queued["id"],"stage":stage,"source_generation":1}]});
    let cancel = || {
        call(
            &state,
            "/api/v1/admin/product-update/cancel",
            "POST",
            json!({}),
            &cookie,
        )
    };
    script(&state, "/product", journal("snapshotting"));
    assert_eq!(cancel().await.0, axum::http::StatusCode::CONFLICT);
    assert_eq!(intent(&state).await["state"], "pending");
    script(&state, "/product", journal("validating"));
    assert_eq!(cancel().await.0, axum::http::StatusCode::OK);
    assert_eq!(intent(&state).await["state"], "cancelled");
    let audited = state
        .db
        .read("test.audit", |db| {
            Ok(db.query_row(
                "SELECT target FROM audit WHERE action='product.update.cancel'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(audited, "99.0.0");
    // Nothing is queued any more, and the withdrawn request no longer advances.
    assert_eq!(cancel().await.0, axum::http::StatusCode::CONFLICT);
    assert!(!advance_install(&state).await.unwrap());
}

// The NAS case: two attempts were blocked at generation 1, then `docker compose
// up` installed the release and the controller adopted generation 2.
#[tokio::test]
async fn obsolete_or_dismissed_journals_need_no_attention() {
    let (_temp, state, cookie) = fixture().await;
    admin(&state).await;
    let (first, second) = (thelxinoe_core::id(), thelxinoe_core::id());
    let blocked = |id: &str| json!({"id":id,"stage":"blocked","error":"Docker inspection unavailable","version":"0.0.2","previous_version":"0.0.1","source_generation":1,"download":{"component":"controller","received":0,"total":null}});
    let journal = |generation: u64| json!({"busy":false,"configured":true,"generation":generation,"items":[blocked(&first),blocked(&second)]});
    let observed = journal(2);
    assert!(
        observed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| obsolete(u, &observed))
    );
    for (operation, expected) in [
        (json!({"stage":"superseded","source_generation":2}), true),
        (json!({"stage":"ready","source_generation":1}), true),
        (json!({"stage":"recovered","source_generation":1}), true),
        (
            json!({"stage":"recovered","source_generation":1,"snapshot_ready":true}),
            false,
        ),
        (json!({"stage":"committed","source_generation":1}), false),
        (
            json!({"stage":"runtime-failure","source_generation":1}),
            false,
        ),
        (json!({"stage":"blocked","source_generation":2}), false),
    ] {
        assert_eq!(obsolete(&operation, &observed), expected, "{operation}");
    }
    assert!(!obsolete(&blocked(&first), &json!({"items":[]})));
    let attention = || async {
        call(&state, "/api/v1/me/attention", "GET", Value::Null, &cookie)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_owned))
            .filter(|id| id.starts_with("product-update:"))
            .collect::<Vec<_>>()
    };
    script(&state, "/product", observed);
    observe(&state).await.unwrap();
    assert!(attention().await.is_empty());
    script(&state, "/product", journal(1));
    observe(&state).await.unwrap();
    assert_eq!(attention().await, [format!("product-update:{first}")]);
    let (status, _, _) = call(
        &state,
        "/api/v1/admin/product-update/dismiss",
        "POST",
        json!({"id":first}),
        &cookie,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(attention().await.is_empty());
    let (_, _, status) = call(
        &state,
        "/api/v1/admin/product-update",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(status["dismissed"], first);
    // Journal errors explain the controller's generic outage text.
    assert!(
        status["controller"]["items"][0]["error"]
            .as_str()
            .unwrap()
            .contains("check that Docker is running")
    );
    // A preparation a restart interrupted starts again by itself.
    let mut interrupted = journal(1);
    interrupted["items"][0] = json!({"id":thelxinoe_core::id(),"stage":"blocked","failure":"interrupted","source_generation":1});
    script(&state, "/product", interrupted);
    observe(&state).await.unwrap();
    assert!(attention().await.is_empty());
    // Repeated automatic failures at the available release are raised as well.
    save(&state, "product.release", manifest("99.0.0"))
        .await
        .unwrap();
    let automatic = |failures: u32| json!({"version":"99.0.0","failures":failures,"retry_at":0});
    let raised = || async {
        call(&state, "/api/v1/me/attention", "GET", Value::Null, &cookie)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "product-automatic")
    };
    save(&state, "product.automatic", automatic(2))
        .await
        .unwrap();
    assert!(!raised().await);
    save(&state, "product.automatic", automatic(3))
        .await
        .unwrap();
    assert!(raised().await);
}

#[test]
fn releases_are_checked_hourly_at_startup_and_when_the_window_opens() {
    let now = 100_000;
    let fresh = json!({"next_check":now + 1800,"last_success":now - 600,"failures":0});
    assert!(!check_due(&fresh, now, true, false));
    assert!(check_due(&fresh, now, false, true));
    let old = json!({"next_check":now + 600,"last_success":now - 3000,"failures":0});
    assert!(check_due(&old, now, true, false));
    assert!(!check_due(&old, now, false, false));
    // A failing channel keeps its own backoff, also in the window.
    let failing = json!({"next_check":now + 120,"last_success":now - 9000,"failures":2});
    assert!(!check_due(&failing, now, true, false));
    assert!(check_due(&Value::Null, now, false, false));
}

#[tokio::test]
async fn successful_checks_repeat_hourly_and_report_incompatible_releases() {
    let (_temp, state, _cookie) = fixture().await;
    let manifest: Manifest = serde_json::from_value(manifest("99.0.0")).unwrap();
    let envelope = Envelope {
        payload: "payload".into(),
        signature: "signature".into(),
    };
    let reason = json!({"version":"99.0.0","reason":"Release cannot migrate the current database"});
    storage::discovered(
        &state.db,
        Some((envelope, manifest, false, Some(reason.clone()))),
        None,
    )
    .await
    .unwrap();
    let observation = setting(&state, "product.observation")
        .await
        .unwrap()
        .unwrap();
    let delay =
        observation["next_check"].as_i64().unwrap() - observation["checked_at"].as_i64().unwrap();
    assert!((3600..3900).contains(&delay), "{delay}");
    assert_eq!(observation["incompatible"], reason);
    assert_eq!(observation["error"], Value::Null);
    assert_eq!(
        setting(&state, "product.release").await.unwrap(),
        Some(Value::Null)
    );
}

#[tokio::test]
async fn release_channel_failures_keep_an_actionable_cause() {
    let refused = reqwest::Client::new()
        .get("https://127.0.0.1:9/latest.json")
        .send()
        .await
        .unwrap_err();
    assert!(
        channel_failure(&refused.into())
            .2
            .contains("could not be reached")
    );
    let invalid = serde_json::from_slice::<Value>(b"<html>").unwrap_err();
    assert_eq!(
        channel_failure(&invalid.into()).2,
        "The release channel returned an invalid release envelope"
    );
    assert_eq!(
        channel_failure(&anyhow::anyhow!("Release manifest exceeds the size limit")).2,
        "The release manifest exceeds the size limit"
    );
    assert!(
        channel_failure(&anyhow::anyhow!("signature error"))
            .2
            .starts_with("The release signature or manifest is invalid")
    );
}
