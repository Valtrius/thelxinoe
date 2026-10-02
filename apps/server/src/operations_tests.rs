use crate::test_support::{call, fixture};
use axum::http::StatusCode;
use serde_json::json;

// UI fixtures cannot catch the storage limit or administrator visibility scope.
// Follow the public history/acknowledgment routes against the real database.
#[tokio::test]
async fn older_request_updates_remain_reachable_for_members_and_administrators() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.history", |db| {
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)",[])?;
        for index in 0..201 {
            db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,'alice','lidarr','g',?1,?1,'denied',?2,1)",rusqlite::params![format!("album-{index:03}"),index])?;
        }
        Ok(())
    }).await.unwrap();
    for page in 1..=2 {
        let result = call(
            &state,
            &format!("/api/v1/acquisition/requests?page={page}"),
            "GET",
            json!({}),
            &alice,
        )
        .await
        .2;
        assert_eq!(result["pages"], 2);
        assert_eq!(result["page"], page);
        let rows = result["items"].as_array().unwrap();
        assert_eq!(rows.len(), if page == 1 { 200 } else { 1 });
        for row in rows {
            let reference = &row["attention"];
            assert_eq!(
                call(
                    &state,
                    &format!("/api/v1/me/attention/{}", reference["id"].as_str().unwrap()),
                    "PUT",
                    json!({"revision":reference["revision"]}),
                    &alice
                )
                .await
                .0,
                StatusCode::OK
            );
        }
    }
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
    state.db.write("test.admin_history", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'",[])?;
        db.execute("DELETE FROM acquisition_requests WHERE id<>'album-000'",[])?;
        db.execute("UPDATE acquisition_requests SET state='failed',updated_at=2 WHERE id='album-000'",[])?;
        for index in 1..=200 {
            db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,'bob','lidarr','g',?1,?1,'denied',?2,1)",rusqlite::params![format!("bob-{index:03}"),index])?;
        }
        Ok(())
    }).await.unwrap();
    let first = call(
        &state,
        "/api/v1/acquisition/requests?page=1",
        "GET",
        json!({}),
        &alice,
    )
    .await
    .2;
    assert!(
        first["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["user_id"] == "bob" && r["attention"].is_null())
    );
    let older = call(
        &state,
        "/api/v1/acquisition/requests?page=2",
        "GET",
        json!({}),
        &alice,
    )
    .await
    .2;
    let own = &older["items"][0];
    assert_eq!(own["id"], "album-000");
    assert_eq!(
        call(
            &state,
            "/api/v1/me/attention/request:album-000",
            "PUT",
            json!({"revision":own["attention"]["revision"]}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
}
#[tokio::test]
async fn attention_is_private_live_and_diagnostics_exclude_secret_material() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;db.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,completed_at,error,created_at) VALUES ('failed','metadata.match','{\"secret\":\"PRIVATE-TOKEN\"}','test','failed',1,?1,'https://private/?ApiKey=PRIVATE-TOKEN',1)",[thelxinoe_core::now()])?;Ok(())}).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    super::operations::observe(&state).await.unwrap();
    super::operations::observe(&state).await.unwrap();
    let notices = call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
        .await
        .2;
    assert_eq!(notices["items"].as_array().unwrap().len(), 1);
    assert!(!notices.to_string().contains("PRIVATE"));
    assert!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let key = notices["items"][0]["id"].as_str().unwrap();
    call(
        &state,
        &format!("/api/v1/me/attention/{key}"),
        "PUT",
        json!({"revision":notices["items"][0]["revision"]}),
        &alice,
    )
    .await;
    assert!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
            .await
            .2["items"][0]["dismissible"]
            == true
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/diagnostics",
            "GET",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let diagnostic = call(
        &state,
        "/api/v1/admin/diagnostics",
        "GET",
        json!({}),
        &admin,
    )
    .await
    .2;
    assert_eq!(diagnostic["database_ok"], true);
    assert!(!diagnostic.to_string().contains("PRIVATE"));
    assert!(!diagnostic.to_string().contains("bob"));
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE jobs SET state='complete' WHERE id='failed'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    super::operations::observe(&state).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE jobs SET state='failed' WHERE id='failed'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    super::operations::observe(&state).await.unwrap();
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &admin)
            .await
            .2["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn user_deletion_revokes_personal_state_and_preserves_shared_data() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;db.execute("INSERT INTO online_accounts(user_id,provider,generation,updated_at) VALUES ('alice','youtube','g',1)",[])?;db.execute("INSERT INTO events(user_id,kind,payload,created_at) VALUES ('alice','private','{}',1)",[])?;db.execute("INSERT INTO stack_provisions(id,kind,actor_id,host_port,credential,state,created_at,updated_at) VALUES ('service','radarr','alice',7878,X'00','active',1,1)",[])?;Ok(())}).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    assert_eq!(
        call(&state, "/api/v1/users/bob", "DELETE", json!({}), &alice)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/users/bob",
            "PUT",
            json!({"role":"user"}),
            &admin
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(&state, "/api/v1/users/alice", "DELETE", json!({}), &admin)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/auth/me", "GET", json!({}), &alice)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    state.db.write("test.fixture", |db|{assert_eq!(db.query_row("SELECT (SELECT COUNT(*) FROM online_accounts)+(SELECT COUNT(*) FROM events WHERE user_id='alice')",[],|r|r.get::<_,i64>(0))?,0);assert_eq!(db.query_row("SELECT actor_id FROM stack_provisions",[],|r|r.get::<_,String>(0))?,"bob");assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);Ok(())}).await.unwrap();
}

// Browser fixtures cannot exercise owner checks or a changed revision inside the
// storage transaction. This API integration test covers those acknowledgment races.
#[tokio::test]
async fn request_attention_maps_album_ancestors_and_rejects_foreign_or_stale_acknowledgments() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/music')", [])?;
        db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,metadata,created_at) VALUES ('artist','music','artist',NULL,'artist','Artist','{}',1),('album','music','album','artist','album','Album','{\"musicbrainz_release_group_id\":\"album-id\"}',1)", [])?;
        db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('lidarr','Lidarr','lidarr','container',8686,'g',X'00','/music','1',1)", [])?;
        db.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES ('request','alice','lidarr','g','album-id','Album','available',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
            .await
            .unwrap();
    let bob = format!("thelxinoe_session={token}");
    let list = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
        .await
        .2;
    let item = &list["items"][0];
    assert_eq!(item["target"], "music");
    assert_eq!(item["media_ids"], json!(["album", "artist"]));
    let path = format!("/api/v1/me/attention/{}", item["id"].as_str().unwrap());
    let revision = item["revision"].clone();
    assert_eq!(
        call(&state, &path, "PUT", json!({"revision":revision}), &bob)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    state
        .db
        .write("test.change", |db| {
            db.execute(
                "UPDATE acquisition_requests SET updated_at=2 WHERE id='request'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(&state, &path, "PUT", json!({"revision":revision}), &alice)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let new = call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
        .await
        .2;
    assert_eq!(
        call(
            &state,
            &path,
            "PUT",
            json!({"revision":new["items"][0]["revision"]}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/me/attention", "GET", json!({}), &alice)
            .await
            .2["items"],
        json!([])
    );
}
