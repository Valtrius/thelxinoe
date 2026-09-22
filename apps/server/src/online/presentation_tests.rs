use super::oauth::tests::{call, fixture};
use axum::http::StatusCode;
use rusqlite::params;
use serde_json::{Value, json};
use thelxinoe_core::now;

#[tokio::test]
async fn live_cards_precede_pagination_and_unknown_shorts_never_flash_in_filtered_feed() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES('alice','UCaaaaaaaaaaaaaaaaaaaaaa','Channel','s',1)", [])?;
        for (id, published, short, broadcast) in [
            ("aaaaaaaaaaa",10,Some(0),"none"),
            ("bbbbbbbbbbb",20,Some(0),"none"),
            ("ccccccccccc",30,None,"none"),
            ("ddddddddddd",40,Some(1),"none"),
            ("eeeeeeeeeee",1,None,"live"),
        ] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title,channel_id,published_at,is_short,broadcast,duration) VALUES('alice',?1,?1,'UCaaaaaaaaaaaaaaaaaaaaaa',?2,?3,?4,60)", params![id,published,short,broadcast])?;
        }
        Ok(())
    }).await.unwrap();
    let mut query = json!({"search":"","watchStates":["unwatched","in_progress","watched"],"includeShorts":false,"includeLive":true,"includeLiveReplays":false,"includeUpcoming":false,"channelId":null,"durationFilter":"any","publishedFilter":"any","sortField":"date","sortDirection":"desc","grouping":"smart","downloadFilter":"all"});
    let page = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":0,"pageSize":2}),
        &alice,
    )
    .await;
    assert_eq!(page.0, StatusCode::OK);
    assert_eq!(page.2["items"][0]["videoId"], "eeeeeeeeeee");
    assert_eq!(page.2["items"][1]["videoId"], "bbbbbbbbbbb");
    assert_eq!(page.2["counts"]["all"], 3);
    query["includeLive"] = json!(false);
    let page = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":0,"pageSize":20}),
        &alice,
    )
    .await
    .2;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    query["includeShorts"] = json!(true);
    let page = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":0,"pageSize":20}),
        &alice,
    )
    .await
    .2;
    assert_eq!(page["items"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn named_watchlists_keep_private_memberships_and_retention_consistent() {
    let (_temp, state, alice) = fixture().await;
    let bob = thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
        .await
        .unwrap();
    let bob = format!("thelxinoe_session={bob}");
    let default = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(default.0, StatusCode::OK);
    assert_eq!(default.2[0]["name"], "Watch Later");
    let id = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "POST",
        json!({"name":"Documentaries"}),
        &alice,
    )
    .await
    .2
    .as_i64()
    .unwrap();
    let base = format!("/api/v1/online/youtube/watchlists/{id}");
    let input = json!({"videoId":"abcdefghijk","manualPosition":0});
    assert_eq!(
        call(
            &state,
            &format!("{base}/items"),
            "POST",
            input.clone(),
            &bob
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let added = call(
        &state,
        &format!("{base}/items"),
        "POST",
        input.clone(),
        &alice,
    )
    .await;
    assert_eq!(added.0, StatusCode::OK, "{:?}", added.2);
    assert_eq!(added.2["video"]["videoId"], "abcdefghijk");
    assert_eq!(added.2["video"]["metadataPending"], true);
    assert_eq!(
        call(&state, &format!("{base}/items"), "POST", input, &alice)
            .await
            .2["added"],
        false
    );
    let legacy = call(
        &state,
        "/api/v1/online/youtube/feed?watchlist=true",
        "GET",
        Value::Null,
        &alice,
    )
    .await;
    assert_eq!(legacy.2["total"], 1);
    let private = call(
        &state,
        "/api/v1/online/youtube/watchlists",
        "GET",
        Value::Null,
        &bob,
    )
    .await;
    assert_eq!(private.2.as_array().unwrap().len(), 1);
    assert!(private.2[0]["items"].as_array().unwrap().is_empty());
    assert_eq!(
        call(&state, &base, "DELETE", Value::Null, &alice).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/feed?watchlist=true",
            "GET",
            Value::Null,
            &alice
        )
        .await
        .2["total"],
        0
    );
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/online/youtube/watchlists/{}", default.2[0]["id"]),
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn feed_filters_sort_and_paginate_before_returning_cards() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{
        for user in ["alice","bob"] {
            db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES(?1,'UCaaaaaaaaaaaaaaaaaaaaaa','Science','s',1)",[user])?;
            for (id,title,duration,short,watched,position) in [("aaaaaaaaaaa","Short",60,1,0,0),("bbbbbbbbbbb","Long watched",2000,0,1,0),("ccccccccccc","Long in progress",1800,0,0,80),("ddddddddddd","Regular",800,0,0,0)] {
                db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,channel_title,published_at,duration,is_short,metadata_at,privacy) VALUES(?1,?2,'UCaaaaaaaaaaaaaaaaaaaaaa',?3,'Science',?4,?5,?6,?4,'public')",params![user,id,title,now(),duration,short])?;
                db.execute("INSERT INTO youtube_state(user_id,video_id,watched,position,updated_at) VALUES(?1,?2,?3,?4,?5)",params![user,id,watched,position,now()])?;
            }
        } Ok(())
    }).await.unwrap();
    let mut query = json!({"search":"","watchStates":["unwatched","in_progress","watched"],"includeShorts":false,"includeLive":false,"includeLiveReplays":false,"includeUpcoming":false,"channelId":null,"durationFilter":"any","publishedFilter":"any","sortField":"duration","sortDirection":"asc","grouping":"none","downloadFilter":"all"});
    let first = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":0,"pageSize":2}),
        &alice,
    )
    .await;
    assert_eq!(first.0, StatusCode::OK, "{:?}", first.2);
    assert_eq!(first.2["counts"]["all"], 3);
    assert_eq!(first.2["counts"]["shorts"], 1);
    assert_eq!(first.2["items"][0]["title"], "Regular");
    assert_eq!(first.2["hasMore"], true);
    let second = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":1,"pageSize":2}),
        &alice,
    )
    .await;
    assert_eq!(second.2["items"][0]["title"], "Long watched");
    assert_eq!(second.2["hasMore"], false);
    query["watchStates"] = json!(["in_progress"]);
    query["durationFilter"] = json!("30_plus");
    let filtered = call(
        &state,
        "/api/v1/online/youtube/browse",
        "POST",
        json!({"query":query,"page":0,"pageSize":60}),
        &alice,
    )
    .await;
    assert_eq!(filtered.2["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered.2["items"][0]["title"], "Long in progress");
    query["sortField"] = json!("v.title;DROP TABLE users");
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/browse",
            "POST",
            json!({"query":query,"page":0,"pageSize":60}),
            &alice
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn download_delete_protects_other_users() {
    let (_temp, state, alice) = fixture().await;
    state.db.write("test.fixture", |db|{
        for user in ["alice","bob"] {
            db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES(?1,'abcdefghijk','Video')",[user])?;
            db.execute("INSERT INTO youtube_state(user_id,video_id,pinned,updated_at) VALUES(?1,'abcdefghijk',1,1)",[user])?;
        }
        db.execute("INSERT INTO youtube_downloads(video_id,generation,state,tools,requested_at,updated_at) VALUES('abcdefghijk',?1,'queued','{}',1,1)",[thelxinoe_core::id()])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/videos/abcdefghijk/download",
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE youtube_state SET pinned=0 WHERE user_id='bob'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            "/api/v1/online/youtube/videos/abcdefghijk/download",
            "DELETE",
            Value::Null,
            &alice
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        call(
            &state,
            "/api/v1/online/youtube/videos/abcdefghijk/download",
            "GET",
            Value::Null,
            &alice
        )
        .await
        .2["download"]
            .is_null()
    );
}
