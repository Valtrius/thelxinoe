use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use thelxinoe_server::{AppState, config::Config, router};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::open(Config {
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: temp.path().join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec![],
        cors_origins: vec![],
        controller_socket: temp.path().join("socket"),
    })
    .await
    .unwrap();
    let hash = thelxinoe_auth::password_hash("a long test-only password".into())
        .await
        .unwrap();
    state
        .db
        .call(move |db| {
            for name in ["alice", "bob"] {
                db.execute(
                    "INSERT INTO users VALUES (?1,?1,?2,'user','UTC',1)",
                    rusqlite::params![name, hash],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let token = thelxinoe_auth::issue_session(
        &state.db,
        "alice".into(),
        "device".into(),
        "Browser test".into(),
    )
    .await
    .unwrap();
    (temp, state, token)
}
async fn call(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "media.test")
        .header("content-type", "application/json")
        .header("x-thelxinoe-client", "1");
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    let response = router(state.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
const DEVICE: &str = "MediaBrowser Client=\"Test%20client\", Device=\"Living room, TV\", DeviceId=\"tv-one\", Version=\"1\"";

#[tokio::test]
async fn diagnostics_exclude_compatibility_credentials_and_untrusted_errors() {
    let (_temp, state, first_party) = fixture().await;
    state
        .db
        .call(|db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, login) = call(
        &state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":"alice","Pw":"a long test-only password"}),
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = login["AccessToken"].as_str().unwrap();
    let query = format!("/Users/Me?api_key={token}");
    assert_eq!(
        call(&state, &query, "GET", Value::Null, &[]).await.0,
        StatusCode::OK
    );
    let leaked_error = format!("untrusted-error-sentinel: {query}");
    state.db.call(move |db| {
        db.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,error,created_at) VALUES ('redaction','fixture','{}','redaction','failed',1,?1,1)", [&leaked_error])?;
        db.execute("INSERT INTO audit(action,target,created_at) VALUES ('fixture',?1,1)", [&leaked_error])?;
        Ok(())
    }).await.unwrap();
    let bearer = format!("Bearer {first_party}");
    let (status, bundle) = call(
        &state,
        "/api/v1/admin/diagnostics",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bundle["database_ok"], true);
    assert_eq!(bundle["jobs"], json!([{"state":"failed","count":1}]));
    let serialized = bundle.to_string();
    for secret in [
        token,
        first_party.as_str(),
        "untrusted-error-sentinel",
        "api_key",
        "a long test-only password",
    ] {
        assert!(!serialized.contains(secret));
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/diagnostics",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn catalog_paging_playlist_order_and_private_state_match_first_party() {
    let (_temp, state, first_party) = fixture().await;
    state.db.call(|db| {
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/music')",[])?;
        for (id,kind,parent) in [("artist","artist",None),("album","album",Some("artist")),("one","track",Some("album")),("two","track",Some("album"))] {
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'music',?2,?3,?1,?1,1)",rusqlite::params![id,kind,parent])?;
        }
        db.execute("INSERT INTO playlists(id,owner_id,name,created_at,updated_at) VALUES ('mix','alice','Shared mix',1,1)",[])?;
        for (position,media) in ["two","one","two"].iter().enumerate() {
            db.execute("INSERT INTO playlist_items VALUES ('mix',?1,?2)",rusqlite::params![position as i64,media])?;
        }
        Ok(())
    }).await.unwrap();
    let mut tokens = Vec::new();
    for name in ["alice", "bob"] {
        let (status, login) = call(
            &state,
            "/Users/AuthenticateByName",
            "POST",
            json!({"Username":name,"Pw":"a long test-only password"}),
            &[("Authorization", DEVICE)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        tokens.push(login["AccessToken"].as_str().unwrap().to_owned());
    }
    let alice = [("X-Emby-Token", tokens[0].as_str())];
    let bob = [("X-Emby-Token", tokens[1].as_str())];
    for query in [
        "IsFolder=false&Limit=0",
        "IncludeItemTypes=Audio&StartIndex=100",
        "ArtistIds=artist&IsFolder=false&Limit=0",
    ] {
        let (status, page) = call(
            &state,
            &format!("/Items?{query}"),
            "GET",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 2);
        assert_eq!(page["Items"], json!([]));
    }
    for path in ["/Playlists/mix/Items", "/Items?ParentId=mix"] {
        let (status, page) = call(&state, path, "GET", Value::Null, &bob).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["TotalRecordCount"], 3);
        let rows = page["Items"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|v| v["Id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["two", "one", "two"]
        );
        assert_ne!(rows[0]["PlaylistItemId"], rows[2]["PlaylistItemId"]);
    }
    let (status, page) = call(
        &state,
        "/Playlists/mix/Items?StartIndex=2&Limit=1",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["Items"].as_array().unwrap().len(), 1);
    assert_eq!(page["Items"][0]["Id"], "two");
    for id in ["mix", "one"] {
        let (status, data) = call(
            &state,
            &format!("/UserFavoriteItems/{id}"),
            "POST",
            Value::Null,
            &alice,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{data}");
        assert_eq!(data["IsFavorite"], true);
        let (_, other) = call(&state, &format!("/Items/{id}"), "GET", Value::Null, &bob).await;
        assert_eq!(other["UserData"]["IsFavorite"], false);
    }
    let (status, data) = call(&state, "/UserPlayedItems/one", "POST", Value::Null, &alice).await;
    assert_eq!(status, StatusCode::OK, "{data}");
    assert_eq!(data["Played"], true);
    for (headers, total) in [(&alice, 1), (&bob, 0)] {
        let (status, page) = call(
            &state,
            "/Items?IncludeItemTypes=Playlist&Filters=IsFavorite&StartIndex=100",
            "GET",
            Value::Null,
            headers,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["TotalRecordCount"], total);
        assert_eq!(page["Items"], json!([]));
    }
    state
        .db
        .call(|db| {
            for index in 0..100 {
                db.execute(
                    "INSERT INTO compat_preferences VALUES ('alice','test',?1,'{}')",
                    [index.to_string()],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/DisplayPreferences/0?client=test",
            "POST",
            json!({"SortBy":"DateCreated"}),
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/DisplayPreferences/new?client=test",
            "POST",
            json!({}),
            &alice
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, saved) = call(
        &state,
        "/api/v1/catalog/one/state",
        "GET",
        Value::Null,
        &[("Authorization", &format!("Bearer {first_party}"))],
    )
    .await;
    assert_eq!(saved["favorite"], true);
    assert_eq!(saved["watched"], true);
    assert_eq!(
        call(
            &state,
            "/Users/bob/FavoriteItems/one",
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, data) = call(
        &state,
        "/UserFavoriteItems/one",
        "DELETE",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["IsFavorite"], false);
}
#[tokio::test]
async fn compatibility_credentials_are_private_revocable_and_transport_scoped() {
    let (_temp, state, first_party) = fixture().await;
    let (status, login) = call(
        &state,
        "/Users/AuthenticateByName",
        "POST",
        json!({"Username":"alice","Pw":"a long test-only password"}),
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token = login["AccessToken"].as_str().unwrap();
    for header in ["X-Emby-Token", "X-MediaBrowser-Token"] {
        assert_eq!(
            call(&state, "/Users/Me", "GET", Value::Null, &[(header, token)])
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}&ApiKey={}", "f".repeat(64)),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &format!("/Users/Me?ApiKey={token}&UserId=bob"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            "/Users/bob/Items",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let bearer = format!("Bearer {token}");
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/me",
            "GET",
            Value::Null,
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/Users/Me",
            "GET",
            Value::Null,
            &[("X-Emby-Token", &first_party)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let session = login["SessionInfo"]["Id"].as_str().unwrap();
    let bearer = format!("Bearer {first_party}");
    let sessions = call(
        &state,
        "/api/v1/auth/sessions",
        "GET",
        Value::Null,
        &[("Authorization", &bearer)],
    )
    .await
    .1;
    assert!(
        sessions["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session && s["name"].as_str().unwrap().contains("Test client"))
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/auth/sessions/{session}"),
            "DELETE",
            Value::Null,
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/Users/Me",
            "GET",
            Value::Null,
            &[("X-Emby-Token", token)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn quick_connect_requires_review_and_is_single_use() {
    let (_temp, state, first_party) = fixture().await;
    let (status, pending) = call(
        &state,
        "/QuickConnect/Initiate",
        "POST",
        Value::Null,
        &[("Authorization", DEVICE)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    let secret = pending["Secret"].as_str().unwrap();
    let code = pending["Code"].clone();
    assert_eq!(
        call(
            &state,
            "/Users/AuthenticateWithQuickConnect",
            "POST",
            json!({"Secret":secret}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let bearer = format!("Bearer {first_party}");
    let (status, review) = call(
        &state,
        "/api/v1/auth/quick-connect/inspect",
        "POST",
        json!({"code":code}),
        &[("Authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(review["device"], "Living room, TV");
    assert_eq!(review["client"], "Test client");
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code,"confirmation":review["confirmation"]}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &format!("/QuickConnect/Connect?secret={secret}"),
            "GET",
            Value::Null,
            &[]
        )
        .await
        .1["Authenticated"],
        true
    );
    let (status, session) = call(
        &state,
        "/Users/AuthenticateWithQuickConnect",
        "POST",
        json!({"Secret":secret}),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["User"]["Name"], "alice");
    assert_eq!(
        call(
            &state,
            "/Users/AuthenticateWithQuickConnect",
            "POST",
            json!({"Secret":secret}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/auth/quick-connect/approve",
            "POST",
            json!({"code":code,"confirmation":review["confirmation"]}),
            &[("Authorization", &bearer)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
