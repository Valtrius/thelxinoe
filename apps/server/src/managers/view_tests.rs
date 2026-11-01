use super::*;
use crate::test_support::fixture;

#[derive(Deserialize)]
struct Case {
    service: Value,
    #[serde(default)]
    unavailable: bool,
    status: (String, String),
}

/// The mocks' copy of these rules is held to the same cases.
#[test]
fn status_follows_the_first_rule_that_applies() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("status_cases.json")).unwrap();
    for case in cases {
        assert_eq!(
            label(&case.service, case.unavailable),
            (case.status.0.as_str(), case.status.1.as_str()),
            "{}",
            case.service
        );
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
