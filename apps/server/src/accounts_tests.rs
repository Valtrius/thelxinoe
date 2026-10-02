use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn password_change_rejects_a_previously_verified_login_and_revokes_an_issued_one() {
    let (_temp, state, cookie) = fixture().await;
    let hash = thelxinoe_auth::password_hash("old password".into())
        .await
        .unwrap();
    state
        .db
        .write("test.credential", move |db| {
            db.execute("UPDATE users SET password_hash=?1 WHERE id='alice'", [hash])?;
            Ok(())
        })
        .await
        .unwrap();
    let address = "127.0.0.1".parse().unwrap();
    let before = crate::accounts::check_credentials(&state, address, "alice", "old password")
        .await
        .unwrap();
    let delayed = crate::accounts::check_credentials(&state, address, "alice", "old password")
        .await
        .unwrap();
    let issued =
        thelxinoe_auth::issue_session(&state.db, before, "device".into(), "before change".into())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/me/password",
            "PUT",
            json!({"current_password":"old password","new_password":"new password"}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        thelxinoe_auth::issue_session(&state.db, delayed, "device".into(), "after change".into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        thelxinoe_auth::resolve(&state.db, &issued, "device")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &cookie)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn closed_setup_never_waits_for_password_hashing_capacity() {
    let (_temp, state, cookie) = fixture().await;
    let slots = state.password_slots.acquire_many(4).await.unwrap();
    for _ in 0..25 {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            call(
                &state,
                "/api/v1/setup",
                "POST",
                json!({"username":"admin","password":"a valid password"}),
                "",
            ),
        )
        .await
        .expect("closed setup must reject before hashing");
        assert_eq!(response.0, StatusCode::CONFLICT);
    }
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &cookie)
            .await
            .0,
        StatusCode::OK
    );
    drop(slots);
}

#[tokio::test]
async fn open_setup_has_bounded_admission_and_an_attempt_limit() {
    let (_temp, state, _) = fixture().await;
    state
        .db
        .write("test.open_setup", |db| {
            db.execute("DELETE FROM users", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let slots = state.password_slots.acquire_many(4).await.unwrap();
    for _ in 0..20 {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            call(
                &state,
                "/api/v1/setup",
                "POST",
                json!({"username":"admin","password":"a valid password"}),
                "",
            ),
        )
        .await
        .expect("setup must not queue behind hashing");
        assert_eq!(response.0, StatusCode::TOO_MANY_REQUESTS);
    }
    drop(slots);
    assert_eq!(
        call(
            &state,
            "/api/v1/setup",
            "POST",
            json!({"username":"admin","password":"a valid password"}),
            ""
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn own_password_change_verifies_current_password_and_revokes_other_devices() {
    let (_temp, state, cookie) = fixture().await;
    let old = "previous passphrase";
    let new = "12345678";
    let hash = thelxinoe_auth::password_hash(old.into()).await.unwrap();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute("UPDATE users SET password_hash=?1 WHERE id='alice'", [hash])?;
            Ok(())
        })
        .await
        .unwrap();
    let other = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "device".into(),
        "Other device".into(),
    )
    .await
    .unwrap();
    let bob =
        crate::test_support::issue_session(&state.db, "bob".into(), "device".into(), "Bob".into())
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
        .write("test.fixture", |db| {
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
        .write("test.fixture", |db| {
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
