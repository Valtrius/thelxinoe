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
