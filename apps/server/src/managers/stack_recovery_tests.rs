use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;

#[tokio::test]
async fn removal_requires_confirmation_from_controller_and_erases_configuration() {
    let (_temp, state, cookie, key) = managed("radarr").await;
    let route = format!("/api/v1/admin/stack/{key}/action");
    for can_remove in [false, true] {
        state.managers.docker.lock().unwrap().extend([
            (
                "stack".into(),
                json!({"items":[{"id":key,"can_remove":can_remove}]}),
            ),
            (
                format!("stack/{key}/action"),
                json!({"accepted":true,"removed":false}),
            ),
        ]);
        let response = call(&state, &route, "POST", json!({"action":"remove"}), &cookie).await;
        assert_eq!(response.0, StatusCode::CONFLICT);
        assert!(
            super::super::storage::service("integration".into(), &state.db)
                .await
                .unwrap()
                .is_some()
        );
    }
    state.managers.docker.lock().unwrap().insert(
        format!("stack/{key}/action"),
        json!({"accepted":true,"removed":true}),
    );
    let response = call(&state, &route, "POST", json!({"action":"remove"}), &cookie).await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    state
        .db
        .read("test.removed", |db| {
            let (enabled, credential, defaults): (bool, Vec<u8>, String) = db.query_row(
                "SELECT enabled,credential,defaults FROM manager_services WHERE id='integration'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert!(!enabled);
            assert!(credential.is_empty());
            assert_eq!(defaults, "{}");
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM stack_provisions", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    let response = call(
        &state,
        "/api/v1/admin/stack/install",
        "POST",
        json!({"kind":"radarr","host_port":17878}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
}

async fn managed(kind: &'static str) -> (tempfile::TempDir, AppState, String, String) {
    let (temp, state, cookie) = fixture().await;
    let key = id();
    let insert_key = key.clone();
    state.db.write("test.managed", move |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'",[])?;
        let table = if kind == "radarr" { "manager_services" } else { "support_services" };
        db.execute(&format!("INSERT INTO {table}(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('integration','Saved service',?1,'old',7878,'old-generation',X'01','/media','1',1)"),[kind])?;
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,container_id,service_id,created_at,updated_at) VALUES (?1,?2,'alice',17878,X'01','complete','old','integration',1,1)",params![insert_key,kind])?;
        db.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES ('install','stack.install',?1,'install','complete',1,1)",[json!({"id":insert_key}).to_string()])?;
        Ok(())
    }).await.unwrap();
    (temp, state, cookie, key)
}

#[tokio::test]
async fn stopped_actions_skip_api_checks_but_unknown_or_running_services_do_not() {
    for kind in ["radarr", "prowlarr"] {
        let (_temp, state, cookie, key) = managed(kind).await;
        for action in ["stop", "restart"] {
            for (observation, expected) in [
                (
                    json!({"existence":"present","phase":"active","drift":false,"running":false}),
                    StatusCode::OK,
                ),
                (
                    json!({"existence":"present","phase":"active","drift":false,"running":true}),
                    StatusCode::CONFLICT,
                ),
                (
                    json!({"existence":"unknown","phase":"active","drift":null,"running":null}),
                    StatusCode::CONFLICT,
                ),
                (
                    json!({"existence":"missing","phase":"active","drift":null,"running":null}),
                    StatusCode::CONFLICT,
                ),
                (
                    json!({"existence":"present","phase":"active","drift":true,"running":false}),
                    StatusCode::CONFLICT,
                ),
            ] {
                let mut observed = observation;
                observed["id"] = json!(key);
                observed["container_id"] = json!("old");
                state.managers.docker.lock().unwrap().extend([
                    ("stack".into(), json!({"items":[observed]})),
                    (
                        format!("stack/{key}/action"),
                        json!({"accepted":true,"container_id":"old","running":false}),
                    ),
                ]);
                let response = call(
                    &state,
                    &format!("/api/v1/admin/stack/{key}/action"),
                    "POST",
                    json!({"action":action}),
                    &cookie,
                )
                .await;
                assert_eq!(
                    response.0.is_success(),
                    expected == StatusCode::OK,
                    "{kind} {action}: {}",
                    response.2
                );
            }
        }
    }
}

#[tokio::test]
async fn accepted_replacement_reconnects_both_records_and_queues_api_verification() {
    for kind in ["radarr", "prowlarr"] {
        let (_temp, state, cookie, key) = managed(kind).await;
        state.managers.docker.lock().unwrap().extend([
            (
                "stack".into(),
                json!({"items":[{"id":key,"container_id":"new","existence":"present"}]}),
            ),
            (
                format!("stack/{key}/action"),
                json!({"accepted":true,"container_id":"new","running":true}),
            ),
        ]);
        let response = call(
            &state,
            &format!("/api/v1/admin/stack/{key}/action"),
            "POST",
            json!({"action":"reconcile"}),
            &cookie,
        )
        .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.2);
        state
            .db
            .read("test.reconnected", move |db| {
                let table = if kind == "radarr" {
                    "manager_services"
                } else {
                    "support_services"
                };
                let (container, generation): (String, String) = db.query_row(
                    &format!("SELECT container_id,generation FROM {table} WHERE id='integration'"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!(container, "new");
                assert_ne!(generation, "old-generation");
                let provision: (String, String, String) = db.query_row(
                    "SELECT container_id,service_id,state FROM stack_provisions WHERE id=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                assert_eq!(
                    provision,
                    ("new".into(), "integration".into(), "connecting".into())
                );
                assert_eq!(
                    db.query_row("SELECT state FROM jobs WHERE id='install'", [], |r| r
                        .get::<_, String>(0))?,
                    "queued"
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn retirement_preserves_history_and_releases_the_kind_for_a_new_installation() {
    let (_temp, state, cookie, key) = managed("radarr").await;
    state.db.write("test.request",|db| {
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('request','alice','integration','old-generation','42','Retained request','pending',1,1)",[])?;
        Ok(())
    }).await.unwrap();
    state.managers.docker.lock().unwrap().extend([
        (
            "stack".into(),
            json!({"items":[{"id":key,"existence":"missing"}]}),
        ),
        (
            format!("stack/{key}/action"),
            json!({"accepted":true,"retired":true,"appdata_preserved":true}),
        ),
    ]);
    let response = call(
        &state,
        &format!("/api/v1/admin/stack/{key}/action"),
        "POST",
        json!({"action":"retire"}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    state
        .db
        .read("test.retired", |db| {
            assert!(!db.query_row(
                "SELECT enabled FROM manager_services WHERE id='integration'",
                [],
                |r| r.get::<_, bool>(0)
            )?);
            assert_eq!(
                db.query_row(
                    "SELECT state FROM acquisition_requests WHERE id='request'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "cancelled"
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM stack_provisions", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    let services = call(
        &state,
        "/api/v1/admin/managers",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(services.2["items"], json!([]));
    let install = call(
        &state,
        "/api/v1/admin/stack/install",
        "POST",
        json!({"kind":"radarr","host_port":17878}),
        &cookie,
    )
    .await;
    assert_eq!(install.0, StatusCode::OK, "{}", install.2);
    state
        .db
        .write("test.new_container", |db| {
            db.execute(
                "UPDATE stack_provisions SET state='connecting',container_id='replacement'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let retained = super::super::storage::register_with_actor_read_manager_services(
        "radarr".into(),
        &state.db,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(retained, "integration");
    assert!(
        super::super::storage::register_with_actor_write_stack_provisions(
            "/media".into(),
            "2".into(),
            retained,
            vec![1],
            &state.db,
            super::super::Register {
                name: "Replacement".into(),
                kind: "radarr".into(),
                container_id: "replacement".into(),
                port: 7878,
                api_key: "unused".into()
            },
            "alice".into()
        )
        .await
        .unwrap()
    );
    assert!(
        super::super::storage::service("integration".into(), &state.db)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn retirement_rejects_unknown_or_present_containers_without_changing_setup() {
    let (_temp, state, cookie, key) = managed("radarr").await;
    for existence in ["unknown", "present"] {
        state.managers.docker.lock().unwrap().insert(
            "stack".into(),
            json!({"items":[{"id":key,"existence":existence}]}),
        );
        let response = call(
            &state,
            &format!("/api/v1/admin/stack/{key}/action"),
            "POST",
            json!({"action":"retire"}),
            &cookie,
        )
        .await;
        assert_eq!(response.0, StatusCode::CONFLICT);
    }
    state
        .db
        .read("test.still_active", |db| {
            assert_eq!(
                db.query_row("SELECT state FROM stack_provisions", [], |r| r
                    .get::<_, String>(0))?,
                "complete"
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn another_service_can_be_removed_while_one_service_is_busy() {
    let (_temp, state, cookie, key) = managed("radarr").await;
    let other = id();
    let insert = other.clone();
    state.db.write("test.concurrent_service", move |db| {
        db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at) VALUES (?1,'sonarr','alice',18989,X'01','complete',1,1)", [insert])?;
        Ok(())
    }).await.unwrap();
    state.managers.docker.lock().unwrap().extend([
        (
            "stack".into(),
            json!({"items":[{"id":key,"can_remove":true},{"id":other,"can_remove":true}]}),
        ),
        (format!("stack/{key}/action"), json!({"removed":true})),
        (format!("stack/{other}/action"), json!({"removed":true})),
    ]);
    let lease = state.managers.maintenance(&state).await;
    let guard = state.managers.guard.service("radarr").await;
    let path = format!("/api/v1/admin/stack/{key}/action");
    let blocked = call(&state, &path, "POST", json!({"action":"remove"}), &cookie);
    tokio::pin!(blocked);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut blocked)
            .await
            .is_err()
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        call(
            &state,
            &format!("/api/v1/admin/stack/{other}/action"),
            "POST",
            json!({"action":"remove"}),
            &cookie,
        ),
    )
    .await
    .expect("An unrelated service must not wait for Radarr");
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    assert!(
        state.media_operations.try_read().is_err(),
        "Maintenance must still exclude new playback/media work"
    );
    drop(guard);
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), &mut blocked)
        .await
        .unwrap();
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    drop(lease);
    assert!(state.media_operations.try_read().is_ok());
}

#[tokio::test]
async fn fresh_server_rejects_recovery_of_unregistered_controller_records() {
    let (_temp, state, cookie) = fixture().await;
    state
        .db
        .write("test.admin", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let key = id();
    state.managers.docker.lock().unwrap().insert(
        "stack".into(),
        json!({"items":[{
            "id":key,"kind":"radarr","status":"missing","existence":"missing",
            "can_recreate":true,"can_retire":true,"can_remove":true
        }]}),
    );
    let response = call(&state, "/api/v1/admin/stack", "GET", json!({}), &cookie).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.2["provisions"], json!([]));
    let item = &response.2["items"][0];
    assert_eq!(item["registered"], false);
    for flag in ["can_recreate", "can_retire", "can_remove"] {
        assert_eq!(item[flag], false);
    }
    let response = call(
        &state,
        &format!("/api/v1/admin/stack/{key}/action"),
        "POST",
        json!({"action":"recreate"}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::CONFLICT);
    assert!(response.2.to_string().contains("no matching server record"));

    state
        .managers
        .docker
        .lock()
        .unwrap()
        .insert("stack".into(), json!({"items":[]}));
    let response = call(&state, "/api/v1/admin/stack", "GET", json!({}), &cookie).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.2["items"], json!([]));
    assert_eq!(response.2["provisions"], json!([]));
}

#[tokio::test]
async fn matching_server_record_keeps_missing_container_recovery_available() {
    let (_temp, state, cookie, key) = managed("radarr").await;
    state.managers.docker.lock().unwrap().insert(
        "stack".into(),
        json!({"items":[{
            "id":key,"kind":"radarr","status":"missing","existence":"missing",
            "can_recreate":true,"can_retire":true,"can_remove":true
        }]}),
    );
    let response = call(&state, "/api/v1/admin/stack", "GET", json!({}), &cookie).await;
    assert_eq!(response.0, StatusCode::OK);
    let item = &response.2["items"][0];
    assert_eq!(item["registered"], true);
    assert_eq!(item["can_recreate"], true);
}
