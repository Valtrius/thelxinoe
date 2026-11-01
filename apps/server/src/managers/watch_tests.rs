use super::*;
use crate::test_support::fixture;

async fn announced(state: &AppState) -> i64 {
    state
        .db
        .read("test.announced", |db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='stack.changed'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

fn script(state: &AppState, container: &str, updated_at: i64) {
    state.managers.docker.lock().unwrap().extend([
        (
            "containers".to_owned(),
            json!({"items":[{"id":"radarr-container","state":container}]}),
        ),
        (
            "stack".to_owned(),
            json!({"items":[{"id":"radarr","kind":"radarr","phase":"active","updated_at":updated_at}]}),
        ),
    ]);
}

#[tokio::test]
async fn announces_only_changes_the_services_page_shows() {
    let (_temp, state, _cookie) = fixture().await;
    state
        .db
        .write("test.manager", |db| {
            db.execute(
                "INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('radarr','Radarr','radarr','radarr-container',7878,'generation',X'00','/media','1',1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    script(&state, "running", 1);
    let mut seen = None;
    check(&state, &mut seen).await.unwrap();
    assert_eq!(announced(&state).await, 0, "the first check only records");

    script(&state, "running", 2);
    check(&state, &mut seen).await.unwrap();
    assert_eq!(announced(&state).await, 0, "a moved timestamp is no change");

    script(&state, "exited", 2);
    check(&state, &mut seen).await.unwrap();
    assert_eq!(
        announced(&state).await,
        1,
        "a container stopped outside Thelxinoe"
    );
    check(&state, &mut seen).await.unwrap();
    assert_eq!(announced(&state).await, 1);

    state
        .db
        .write("test.failed_check", |db| {
            db.execute(
                "UPDATE manager_services SET error='Radarr is unreachable' WHERE id='radarr'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    check(&state, &mut seen).await.unwrap();
    assert_eq!(announced(&state).await, 2, "a background check failed");

    state.managers.docker.lock().unwrap().insert(
        "stack".to_owned(),
        json!({"__error":["dependency_unavailable","Controller unavailable"]}),
    );
    check(&state, &mut seen).await.unwrap();
    assert_eq!(
        announced(&state).await,
        3,
        "the controller became unavailable"
    );
}
