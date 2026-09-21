use super::*;
use crate::online::oauth::tests::{call, fixture};
use axum::http::StatusCode;
#[test]
fn external_timestamps_require_identity_and_explicit_bounded_fields() {
    let value = json!({"tmdb_id":1402,"season":1,"episode":1,"type":"tv","intro":[{}, {"start_ms":10000,"end_ms":20000},{"start_ms":20000,"end_ms":999999}],"credits":[{"start_ms":100000,"end_ms":null}]});
    let items = analysis::parse_external(&value, Some(1402), 1, 1, 120.0).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].start, 10.0);
    assert_eq!(items[1].end, 120.0);
    assert!(analysis::parse_external(&value, Some(1402), 1, 2, 120.0).is_err());
    assert!(analysis::parse_external(&value, Some(1), 1, 1, 120.0).is_err());
    assert!(analysis::parse_external(&value, None, 1, 1, 120.0).is_ok());
}
#[tokio::test]
async fn jellyfin_segments_follow_only_this_devices_selected_edition() {
    let (_temp, state, alice) = fixture().await;
    let mut headers = HeaderMap::new();
    headers.insert("cookie", alice.parse().unwrap());
    let p = security::principal(&state, &headers).await.unwrap();
    state.db.call(|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','Movies','movies','/fixture')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('m','r','movie','e','Movie',1)",[])?;
        for (file,start) in [("f1",10),("f2",40)] {
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,'r',?2,'g1',1,'1','hash','{\"format\":{\"duration\":\"120\"}}',1)",params![file,format!("/fixture/{file}.mkv")])?;
            db.execute("INSERT INTO media_sources VALUES ('m',?1)",[file])?;
            db.execute("INSERT INTO media_segments VALUES (?1,'m',?1,'g1','Intro',?2,?2+20,'manual',1,1)",params![file,start])?;
        }
        Ok(())
    }).await.unwrap();
    assert!(for_jellyfin(&state, &p, "m").await.unwrap().is_empty());
    let auth = p.session_id.clone();
    state.db.call(move|db|{
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('play','alice',?1,'m','f2','g1','','ready','direct','{}',120,1,1)",[auth])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(for_jellyfin(&state, &p, "m").await.unwrap()[0].start, 40.0);
    let mut other = p.clone();
    other.session_id = "another-device".into();
    assert!(for_jellyfin(&state, &other, "m").await.unwrap().is_empty());
    state
        .db
        .call(|db| {
            db.execute("UPDATE media_files SET generation='g2' WHERE id='f2'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(for_jellyfin(&state, &p, "m").await.unwrap().is_empty());
}
#[tokio::test]
async fn manual_overrides_are_authorized_bounded_and_specific_to_file_generation() {
    let (_temp, state, alice) = fixture().await;
    state.db.call(|db|{
        db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','TV','shows','/fixture')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('m','r','episode','e','Episode',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('f','r','/fixture/e.mkv','g1',1,'1','hash','{\"format\":{\"duration\":\"120\"}}',1)",[])?;
        db.execute("INSERT INTO media_sources VALUES ('m','f')",[])?;
        db.execute("INSERT INTO media_segments VALUES ('automatic','m','f','g1','Intro',10,30,'local',0.85,1)",[])?;
        Ok(())
    }).await.unwrap();
    let token =
        thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let edit =
        json!({"file_id":"f","generation":"g1","items":[{"kind":"Intro","start":15,"end":35}]});
    assert_eq!(
        call(
            &state,
            "/api/v1/catalog/m/segments",
            "PUT",
            edit.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&state,"/api/v1/catalog/m/segments","PUT",json!({"file_id":"f","generation":"g1","items":[{"kind":"Intro","start":15,"end":135}]}),&admin).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(
        call(&state, "/api/v1/catalog/m/segments", "PUT", edit, &admin)
            .await
            .0,
        StatusCode::OK
    );
    let (_, _, items) = resolved(&state, "m", Some("f")).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].start, 15.0);
    assert_eq!(items[0].source, "manual");
    assert_eq!(
        call(
            &state,
            "/api/v1/catalog/m/segments",
            "PUT",
            json!({"file_id":"f","generation":"g1","items":[]}),
            &admin
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(resolved(&state, "m", None).await.unwrap().2.is_empty());
    assert_eq!(
        call(
            &state,
            "/api/v1/catalog/m/segments",
            "PUT",
            json!({"file_id":"f","generation":"g1","items":[],"reset":true}),
            &admin
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        resolved(&state, "m", None).await.unwrap().2[0].source,
        "local"
    );
    state
        .db
        .call(|db| {
            db.execute("UPDATE media_files SET generation='g2'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(resolved(&state, "m", None).await.unwrap().2.is_empty());
    assert_eq!(
        call(
            &state,
            "/api/v1/catalog/m/segments",
            "PUT",
            json!({"file_id":"f","generation":"g1","items":[]}),
            &admin
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn skip_preferences_are_private_and_default_to_ask() {
    let (_temp, state, alice) = fixture().await;
    let token = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
        .await
        .unwrap();
    let bob = format!("thelxinoe_session={token}");
    let changed = json!({"Intro":"Auto","Recap":"Ignore","Credits":"Ask","Preview":"Ignore"});
    assert_eq!(
        call(
            &state,
            "/api/v1/me/segments",
            "PUT",
            changed.clone(),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, "/api/v1/me/segments", "GET", json!({}), &alice)
            .await
            .2,
        changed
    );
    assert_eq!(
        call(&state, "/api/v1/me/segments", "GET", json!({}), &bob)
            .await
            .2,
        json!({"Intro":"Ask","Recap":"Ask","Credits":"Ask","Preview":"Ask"})
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/me/segments",
            "PUT",
            json!({"Intro":"Unsupported"}),
            &bob
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
