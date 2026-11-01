use super::*;
use crate::test_support::fixture;

#[test]
fn status_follows_the_first_rule_that_applies() {
    let transfer =
        |state: &str| json!({"stage":"owned","operation":{"type":"transfer","state":state}});
    let owned = |runtime: Value| json!({"stage":"owned","runtime":runtime});
    for (service, unavailable, expected) in [
        (
            json!({"runtime":{"registered":false},"operation":{"type":"transfer","state":"running"}}),
            false,
            ("Setup mismatch", "warn"),
        ),
        (transfer("queued"), false, ("Transfer queued", "busy")),
        (transfer("running"), false, ("Transferring", "busy")),
        (
            transfer("committed"),
            false,
            ("Owned · finishing transfer", "busy"),
        ),
        (transfer("recovery"), false, ("Recovering", "busy")),
        (
            transfer("failed"),
            false,
            ("Transfer needs attention", "bad"),
        ),
        (transfer("suspended"), false, ("Transfer suspended", "warn")),
        (
            json!({"stage":"connected","connection":{"verified":true,"error":null},"provision":{"state":"blocked"}}),
            false,
            ("Connected", "ok"),
        ),
        (
            json!({"stage":"connected","connection":{"verified":true,"error":"Unreachable"}}),
            false,
            ("Unverified", "warn"),
        ),
        (
            json!({"stage":"connected","connection":{"verified":false,"error":null}}),
            false,
            ("Unverified", "warn"),
        ),
        (
            json!({"provision":{"state":"queued"}}),
            false,
            ("Installing", "busy"),
        ),
        (
            json!({"provision":{"state":"installing"}}),
            false,
            ("Installing", "busy"),
        ),
        (
            json!({"provision":{"state":"connecting"}}),
            false,
            ("Connecting API", "busy"),
        ),
        (
            json!({"provision":{"state":"retiring"}}),
            false,
            ("Retiring", "busy"),
        ),
        (
            json!({"provision":{"state":"blocked"},"runtime":{"running":true}}),
            false,
            ("Setup blocked", "bad"),
        ),
        (
            json!({"update":{"latest":{"state":"activating"}},"runtime":{"running":true}}),
            false,
            ("Connecting API", "busy"),
        ),
        (
            owned(json!({"status":"unavailable","running":false})),
            false,
            ("Status unavailable", "warn"),
        ),
        (
            owned(json!({"status":"missing"})),
            false,
            ("Container missing", "bad"),
        ),
        (
            owned(json!({"status":"ready","running":false})),
            false,
            ("Ready", "ok"),
        ),
        (
            json!({"runtime":{"running":false},"integration":{"error":"Unreachable"}}),
            false,
            ("Stopped", "muted"),
        ),
        (
            json!({"runtime":{"running":true},"integration":{"error":"Unreachable"}}),
            false,
            ("API unavailable", "warn"),
        ),
        (
            json!({"runtime":{"running":true},"integration":{"error":""}}),
            false,
            ("Running", "ok"),
        ),
        (
            json!({"integration":{"error":null}}),
            true,
            ("Connected", "ok"),
        ),
        (json!({}), true, ("Status unavailable", "warn")),
        (json!({}), false, ("Not connected", "muted")),
    ] {
        assert_eq!(label(&service, unavailable), expected, "{service}");
    }
}

#[tokio::test]
async fn each_service_joins_its_parts_and_survives_a_controller_outage() {
    let (_temp, state, _cookie) = fixture().await;
    state
        .db
        .write("test.radarr", |db| {
            db.execute(
                "INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('radarr','Radarr','radarr','radarr-container',7878,'generation',X'00','/media','1',1)",
                [],
            )?;
            crate::test_support::own(db)?;
            Ok(())
        })
        .await
        .unwrap();
    state.managers.docker.lock().unwrap().extend([
        (
            "containers".to_owned(),
            json!({"items":[{"id":"radarr-container","state":"running"}]}),
        ),
        (
            "stack".to_owned(),
            json!({"items":[{"id":"owned-radarr","kind":"radarr","status":"running","running":true,"existence":"present"}]}),
        ),
    ]);
    let view = listing(&state).await;
    let radarr = |view: &Value| {
        view["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "radarr")
            .cloned()
            .unwrap()
    };
    let service = radarr(&view);
    assert_eq!(
        view["items"].as_array().unwrap().len(),
        transfers::KINDS.len()
    );
    assert_eq!(service["stage"], "owned");
    assert_eq!(service["integration"]["id"], "radarr");
    assert_eq!(service["provision"]["id"], "owned-radarr");
    assert_eq!(service["runtime"]["registered"], true);
    assert_eq!(service["status"], json!({"label":"Running","tone":"ok"}));
    assert_eq!(view["containers"][0]["id"], "radarr-container");
    assert!(view["errors"].get("stack").is_none());

    state.managers.docker.lock().unwrap().insert(
        "stack".to_owned(),
        json!({"__error":["dependency_unavailable","Controller unavailable"]}),
    );
    let view = listing(&state).await;
    let service = radarr(&view);
    assert_eq!(view["errors"]["stack"], "Controller unavailable");
    assert_eq!(service["provision"]["id"], "owned-radarr");
    assert_eq!(service["runtime"]["existence"], "unknown");
    assert_eq!(
        service["status"],
        json!({"label":"Status unavailable","tone":"warn"})
    );
}
