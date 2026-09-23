use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;

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
