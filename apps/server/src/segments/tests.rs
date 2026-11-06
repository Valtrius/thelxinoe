use super::*;
use crate::test_support::{call, fixture};
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
    state.db.write("test.fixture", |db|{
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
    state.db.write("test.fixture", move|db|{
        db.execute("INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('play','alice',?1,'m','f2','g1','','ready','direct','{}',120,1,1)",[auth])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(for_jellyfin(&state, &p, "m").await.unwrap()[0].start, 40.0);
    let mut other = p.clone();
    other.session_id = "another-device".into();
    assert!(for_jellyfin(&state, &other, "m").await.unwrap().is_empty());
    state
        .db
        .write("test.fixture", |db| {
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
    state.db.write("test.fixture", |db|{
        db.execute("UPDATE users SET role='admin' WHERE id='bob'",[])?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','TV','shows','/fixture')",[])?;
        db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES ('m','r','episode','e','Episode',1)",[])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('f','r','/fixture/e.mkv','g1',1,'1','hash','{\"format\":{\"duration\":\"120\"}}',1)",[])?;
        db.execute("INSERT INTO media_sources VALUES ('m','f')",[])?;
        db.execute("INSERT INTO media_segments VALUES ('automatic','m','f','g1','Intro',10,30,'local',0.85,1)",[])?;
        Ok(())
    }).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
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
        .write("test.fixture", |db| {
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
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
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

#[tokio::test]
async fn analysis_results_cover_every_current_episode_with_what_applies_to_it() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='bob'", [])?;
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('r','Shows','shows','/fixture')", [])?;
        let media = "INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,metadata,created_at) VALUES (?1,'r',?2,?3,?1,?4,?5,?6,?7)";
        db.execute(media, params!["alpha", "show", Option::<String>::None, "Alpha (2020)", Option::<i64>::None, "{\"title\":\"Alpha Show\"}", 1])?;
        db.execute(media, params!["alpha-1", "season", "alpha", "Season 1", 1, "{}", 1])?;
        db.execute(media, params!["beta", "show", Option::<String>::None, "Beta", Option::<i64>::None, "{}", 1])?;
        db.execute(media, params!["beta-1", "season", "beta", "Season 2", 2, "{}", 1])?;
        // id, season, episode number, analysis state, requested, completed, error
        for (id, season, number, analysis, requested, completed, error) in [
            ("found", "alpha-1", 1, Some("complete"), 1, Some(100), None),
            ("nothing", "alpha-1", 2, Some("complete"), 1, Some(200), None),
            ("failed", "alpha-1", 3, Some("failed"), 1, Some(300), Some("Audio analysis timed out")),
            ("queued", "alpha-1", 4, Some("queued"), 40, Some(350), None),
            ("running", "alpha-1", 5, Some("running"), 1, None, None),
            ("unreached", "alpha-1", 6, None, 30, None, None),
            ("corrected", "alpha-1", 7, Some("complete"), 1, Some(250), None),
            ("beta-found", "beta-1", 1, Some("complete"), 1, Some(150), None),
        ] {
            db.execute(media, params![id, "episode", season, format!("raw {id}"), number, "{}", requested])?;
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,'r',?1,'g1',1,'1','hash','{}',1)", [id])?;
            db.execute("INSERT INTO media_sources VALUES (?1,?1)", [id])?;
            if let Some(analysis) = analysis {
                db.execute("INSERT INTO segment_analysis VALUES (?1,?1,'g1',?2,?3,?4,?5)", params![id, analysis, requested, completed, error])?;
            }
        }
        // A replaced file: its old result no longer counts, and the new one still has to run.
        db.execute(media, params!["replaced", "episode", "alpha-1", "raw replaced", 8, "{}", 20])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('replaced','r','replaced','new',1,'1','hash','{}',1)", [])?;
        db.execute("INSERT INTO media_sources VALUES ('replaced','replaced')", [])?;
        db.execute("INSERT INTO segment_analysis VALUES ('replaced','replaced','old','complete',1,400,NULL)", [])?;
        db.execute("INSERT INTO media_segments VALUES ('old','replaced','replaced','old','Intro',1,20,'local',0.85,1)", [])?;
        // A removed file and a file shared by two episodes are never analyzed.
        db.execute(media, params!["removed", "episode", "alpha-1", "raw removed", 9, "{}", 1])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at,present) VALUES ('removed','r','removed','g1',1,'1','hash','{}',1,0)", [])?;
        db.execute("INSERT INTO media_sources VALUES ('removed','removed')", [])?;
        db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('shared','r','shared','g1',1,'1','hash','{}',1)", [])?;
        for id in ["shared-1", "shared-2"] {
            db.execute(media, params![id, "episode", "alpha-1", id, 10, "{}", 1])?;
            db.execute("INSERT INTO media_sources VALUES (?1,'shared')", [id])?;
        }
        let segment = "INSERT INTO media_segments VALUES (?1,?2,?2,'g1',?3,?4,?5,?6,0.85,1)";
        db.execute(segment, params!["a", "found", "Intro", 10, 30, "local"])?;
        db.execute(segment, params!["b", "found", "Credits", 500, 520, "local"])?;
        db.execute(segment, params!["c", "queued", "Intro", 15, 35, "local"])?;
        // A correction replaces everything detected, and a better source replaces that kind.
        db.execute(segment, params!["d", "corrected", "Intro", 40, 60, "manual"])?;
        db.execute(segment, params!["e", "corrected", "Credits", 500, 520, "local"])?;
        db.execute("INSERT INTO segment_overrides VALUES ('corrected','corrected','g1')", [])?;
        db.execute(segment, params!["f", "beta-found", "Intro", 70, 90, "theintrodb"])?;
        db.execute(segment, params!["g", "beta-found", "Intro", 10, 30, "local"])?;
        db.execute(segment, params!["h", "beta-found", "Recap", 0, 5, "theintrodb"])?;
        Ok(())
    }).await.unwrap();
    let token =
        crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Admin".into())
            .await
            .unwrap();
    let admin = format!("thelxinoe_session={token}");
    let results = async |query: &str| {
        let (status, _, body) = call(
            &state,
            &format!("/api/v1/admin/segments/analysis{query}"),
            "GET",
            json!({}),
            &admin,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    };
    let ids = |page: &Value| {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["media_id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        call(
            &state,
            "/api/v1/admin/segments/analysis",
            "GET",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let page = results("").await;
    assert_eq!(
        page["totals"],
        json!({"episodes":9,"intro":4,"credits":1,"nothing":1,"failed":1,"waiting":4})
    );
    assert_eq!(
        page["shows"],
        json!([
            {"id":"alpha","title":"Alpha Show","episodes":8,"intro":3,"credits":1,"nothing":1,"failed":1,"waiting":4},
            {"id":"beta","title":"Beta","episodes":1,"intro":1,"credits":0,"nothing":0,"failed":0,"waiting":0},
        ])
    );
    // Work in progress first, then what finished most recently.
    assert_eq!(
        ids(&page),
        [
            "running",
            "queued",
            "unreached",
            "replaced",
            "failed",
            "corrected",
            "nothing",
            "beta-found",
            "found"
        ]
    );
    let item = |id: &str| {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["media_id"] == id)
            .unwrap()
            .clone()
    };
    let segments = |id: &str| {
        item(id)["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["kind"].as_str().unwrap().to_owned(),
                    s["start"].as_f64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let found = item("found");
    assert_eq!(found["title"], "Alpha Show · S01E01");
    assert_eq!(found["status"], "found");
    assert_eq!(found["analyzed_at"], 100);
    assert_eq!(segments("corrected"), [("Intro".to_owned(), 40.0)]);
    assert_eq!(
        segments("beta-found"),
        [("Recap".to_owned(), 0.0), ("Intro".to_owned(), 70.0)]
    );
    assert_eq!(item("beta-found")["title"], "Beta · S02E01");
    assert_eq!(item("nothing")["status"], "nothing");
    let failed = item("failed");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "Audio analysis timed out");
    // A file waiting to run again keeps what it had, and shows no finish time from before.
    let queued = item("queued");
    assert_eq!(queued["status"], "waiting");
    assert!(queued["analyzed_at"].is_null());
    assert_eq!(segments("queued"), [("Intro".to_owned(), 15.0)]);
    assert_eq!(item("unreached")["status"], "waiting");
    let replaced = item("replaced");
    assert_eq!(replaced["status"], "waiting");
    assert!(replaced["segments"].as_array().unwrap().is_empty());
    assert_eq!(item("running")["status"], "running");

    assert_eq!(ids(&results("?status=nothing").await), ["nothing"]);
    assert_eq!(
        ids(&results("?status=waiting").await),
        ["running", "queued", "unreached", "replaced"]
    );
    // A show lists in episode order, and the coverage stays complete under any filter.
    let alpha = results("?show=alpha").await;
    assert_eq!(
        ids(&alpha),
        [
            "found",
            "nothing",
            "failed",
            "queued",
            "running",
            "unreached",
            "corrected",
            "replaced"
        ]
    );
    assert_eq!(alpha["totals"]["episodes"], 9);
    assert_eq!(
        ids(&results("?show=beta&status=failed").await),
        Vec::<String>::new()
    );
    let limited = results("?limit=2").await;
    assert_eq!(ids(&limited), ["running", "queued"]);
    assert_eq!(limited["totals"]["episodes"], 9);
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/segments/analysis?limit=nonsense",
            "GET",
            json!({}),
            &admin
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
