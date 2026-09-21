use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn own_password_change_verifies_current_password_and_revokes_other_devices() {
    let (_temp, state, cookie) = fixture().await;
    let old = "previous passphrase";
    let new = "replacement passphrase";
    let hash = thelxinoe_auth::password_hash(old.into()).await.unwrap();
    state
        .db
        .call(move |db| {
            db.execute("UPDATE users SET password_hash=?1 WHERE id='alice'", [hash])?;
            Ok(())
        })
        .await
        .unwrap();
    let other = thelxinoe_auth::issue_session(
        &state.db,
        "alice".into(),
        "device".into(),
        "Other device".into(),
    )
    .await
    .unwrap();
    let bob = thelxinoe_auth::issue_session(&state.db, "bob".into(), "device".into(), "Bob".into())
        .await
        .unwrap();
    let change =
        |current: &str, next: &str| json!({"current_password":current,"new_password":next});
    assert_eq!(
        call(
            &state,
            "/api/v1/me/password",
            "PUT",
            change("incorrect", new),
            &cookie
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/password",
            "PUT",
            change(old, "short"),
            &cookie
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(
        thelxinoe_auth::resolve(&state.db, &other, "device")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        call(&state, "/api/v1/me/password", "PUT", change(old, new), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/password",
            "PUT",
            change(old, new),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &cookie)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        thelxinoe_auth::resolve(&state.db, &other, "device")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        thelxinoe_auth::resolve(&state.db, &bob, "device")
            .await
            .unwrap()
            .is_some()
    );
    let hash = state
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT password_hash FROM users WHERE id='alice'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(
        !thelxinoe_auth::verify_password(old.into(), hash.clone())
            .await
            .unwrap()
    );
    assert!(
        thelxinoe_auth::verify_password(new.into(), hash)
            .await
            .unwrap()
    );
    state
        .db
        .call(|db| {
            let audit: String =
                db.query_row("SELECT action FROM audit WHERE actor_id='alice'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(audit, "user.password");
            assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
            Ok(())
        })
        .await
        .unwrap();
}
