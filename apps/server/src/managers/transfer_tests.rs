//! Lifecycle transitions of a transfer whose controller replies are lost, refused or late.
//! The controller is scripted; each test checks that this record follows the controller's
//! durable outcome and never infers one from a failed reply.
use super::*;
use crate::test_support::{CONTROLLER_FAILURE, call, fixture};
use axum::http::StatusCode;

const ORIGINAL: &str = "original";
const REPLACEMENT: &str = "replacement";

/// A transfer after the production boundary, committed to `REPLACEMENT` when `committed`.
async fn seeded(kind: &'static str, state_name: &'static str, committed: bool) -> Seeded {
    let (temp, state, cookie) = fixture().await;
    let id = thelxinoe_core::id();
    let key = id.clone();
    state.db.write("test.transfer", move |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        let table = if kind == "radarr" { "manager_services" } else { "support_services" };
        let container = if committed { REPLACEMENT } else { ORIGINAL };
        db.execute(&format!("INSERT INTO {table}(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at,operational) VALUES ('svc','Service',?1,?2,7878,'g',X'01','/media','1',1,0)"), rusqlite::params![kind, container])?;
        if committed {
            db.execute("INSERT INTO stack_provisions(id,kind,actor_id,state,container_id,service_id,origin,hold,transfer_id,created_at,updated_at) VALUES (?1,?2,'alice','complete',?3,'svc','transferred','transfer',?1,1,1)", rusqlite::params![key, kind, REPLACEMENT])?;
        }
        let review = json!({"blockers":[],"setup_actions":[{"action":"seerr.managed","label":"Managed policy"}],
            "source_connection":{"container_id":ORIGINAL,"container_name":"original","port":7878,"url_base":"","media_source":"/media","evidence":{}}});
        let recovery = json!({"revision":"plan-1","options":[
            {"action":"return_original","available":true,"requires":["state_loss"]},
            {"action":"restore_snapshot","available":true,"requires":[]},
            {"action":"repair","available":false,"unavailable":"The original is running; a replacement never starts beside it"},
            {"action":"suspend","available":true}]});
        let setup = json!([{"action":"seerr.managed","state":"applied","generation":"first"}]);
        db.execute("INSERT INTO service_transfers(id,kind,service_id,actor_id,container_id,engine,connection_revision,review_revision,review,state,step,setup,recovery,replacement_id,approved_by,created_at,updated_at) VALUES (?1,?2,'svc','alice',?3,'engine','revision','review',?4,?5,'checks',?6,?7,?8,'alice',1,1)",
            rusqlite::params![key, kind, ORIGINAL, review.to_string(), state_name, setup.to_string(), recovery.to_string(), committed.then_some(REPLACEMENT)])?;
        Ok(())
    }).await.unwrap();
    Seeded {
        _temp: temp,
        state,
        cookie,
        id,
    }
}
struct Seeded {
    _temp: tempfile::TempDir,
    state: AppState,
    cookie: String,
    id: String,
}
impl Seeded {
    fn script(&self, path: &str, value: Value) {
        self.state
            .managers
            .docker
            .lock()
            .unwrap()
            .insert(format!("stack/transfers/{}{path}", self.id), value);
    }
    async fn row(&self) -> storage::Row {
        storage::load(&self.state.db, self.id.clone())
            .await
            .unwrap()
            .unwrap()
    }
    async fn job(&self) -> anyhow::Result<bool> {
        run_job(
            &self.state,
            &thelxinoe_jobs::Job {
                id: "job".into(),
                kind: "service.transfer".into(),
                subject: self.id.clone(),
                payload: json!({"id":self.id}),
                state: "running".into(),
                attempts: 1,
                error: None,
            },
        )
        .await
    }
    async fn recover(&self, action: &str) -> (StatusCode, Value) {
        let response = call(
            &self.state,
            &format!("/api/v1/admin/transfers/{}/recovery", self.id),
            "POST",
            json!({"action":action,"revision":"plan-1","approval":{"state_loss":true}}),
            &self.cookie,
        )
        .await;
        (response.0, response.2)
    }
    async fn query<T: rusqlite::types::FromSql + Send + 'static>(&self, sql: &'static str) -> T {
        self.state
            .db
            .read("test.query", move |db| {
                Ok(db.query_row(sql, [], |r| r.get(0))?)
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_lost_return_reply_is_reconciled_to_the_original_and_never_reopened() {
    let t = seeded("radarr", "failed", true).await;
    // The controller ran the return, but its reply never reached this server.
    let (status, view) = CONTROLLER_FAILURE
        .scope(
            ("dependency_unavailable", "connection reset"),
            t.recover("return_original"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["state"], "recovery");
    let operation = t.row().await.recovery["command"]["operation"]
        .as_str()
        .unwrap()
        .to_owned();
    // An outage while following the command is waited out; nothing is inferred from it.
    let waited = CONTROLLER_FAILURE
        .scope(("dependency_unavailable", "connection reset"), t.job())
        .await
        .unwrap();
    assert!(!waited);
    assert_eq!(t.row().await.state, "recovery");
    t.script(
        "",
        json!({"stage":"returned","boundary_at":1,"replacement":REPLACEMENT,
            "command":{"operation":operation,"action":"return_original","state":"applied","phase":"starting-original"}}),
    );
    assert!(t.job().await.unwrap());
    let row = t.row().await;
    assert_eq!(row.state, "returned");
    assert_eq!(row.recovery["command"]["outcome"], "applied");
    assert_eq!(
        t.query::<i64>("SELECT COUNT(*) FROM stack_provisions")
            .await,
        0
    );
    assert_eq!(
        t.query::<String>("SELECT container_id FROM manager_services WHERE id='svc'")
            .await,
        ORIGINAL
    );
    // A returned transfer offers no recovery, so nothing can start the replacement again.
    for (method, body) in [
        ("GET", Value::Null),
        ("POST", json!({"action":"repair","revision":"plan-1"})),
    ] {
        let again = call(
            &t.state,
            &format!("/api/v1/admin/transfers/{}/recovery", t.id),
            method,
            body,
            &t.cookie,
        )
        .await;
        assert_eq!(again.0, StatusCode::CONFLICT, "{}", again.2);
    }
}

#[tokio::test]
async fn a_failed_server_reconciles_a_return_the_controller_already_made() {
    let t = seeded("radarr", "failed", true).await;
    t.script(
        "",
        json!({"stage":"returned","boundary_at":1,"command":{"operation":thelxinoe_core::id(),"state":"applied"}}),
    );
    let plan = call(
        &t.state,
        &format!("/api/v1/admin/transfers/{}/recovery", t.id),
        "GET",
        Value::Null,
        &t.cookie,
    )
    .await;
    assert_eq!(plan.0, StatusCode::CONFLICT, "{}", plan.2);
    assert_eq!(t.row().await.state, "returned");
    assert_eq!(
        t.query::<String>("SELECT container_id FROM manager_services WHERE id='svc'")
            .await,
        ORIGINAL
    );
}

#[tokio::test]
async fn a_refused_recovery_leaves_the_transfer_where_it_was() {
    let t = seeded("radarr", "failed", true).await;
    t.script(
        "",
        json!({"stage":"recovery-required","boundary_at":1,"command":null}),
    );
    t.script(
        "/recovery",
        json!({"__error":["revision_changed","The recovery evidence changed; review the plan again"]}),
    );
    let (status, body) = t.recover("return_original").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let row = t.row().await;
    assert_eq!(row.state, "failed");
    assert_eq!(row.recovery["command"]["outcome"], "refused");
    assert_eq!(
        t.query::<Option<String>>("SELECT hold FROM stack_provisions")
            .await
            .as_deref(),
        Some("transfer")
    );
    // Options the plan marks unavailable are refused before anything is recorded.
    let (status, _) = t.recover("repair").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(t.row().await.state, "failed");
}

#[tokio::test]
async fn a_failed_command_returns_to_its_state_and_an_interrupted_one_resumes() {
    let t = seeded("radarr", "suspended", true).await;
    t.script(
        "",
        json!({"stage":"suspended","boundary_at":1,"command":null}),
    );
    t.script(
        "/recovery",
        json!({"stage":"recovering","operation_active":false,"command":{"state":"running","phase":"stopping-replacement"}}),
    );
    t.script("/resume", json!({}));
    let (status, view) = t.recover("return_original").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["state"], "recovery");
    let operation = t.row().await.recovery["command"]["operation"]
        .as_str()
        .unwrap()
        .to_owned();
    t.script(
        "",
        json!({"stage":"recovering","operation_active":false,
            "command":{"operation":operation,"state":"running","phase":"checking-original","error":null}}),
    );
    assert!(!t.job().await.unwrap());
    assert_eq!(t.row().await.progress["phase"], "checking-original");
    t.script(
        "",
        json!({"stage":"suspended","boundary_at":1,
            "command":{"operation":operation,"state":"failed","error":"Automation around the original changed"}}),
    );
    assert!(t.job().await.unwrap());
    let row = t.row().await;
    assert_eq!(row.state, "suspended");
    assert_eq!(
        row.error.as_deref(),
        Some("Automation around the original changed")
    );
    assert_eq!(row.recovery["command"]["outcome"], "failed");
}

#[tokio::test]
async fn a_restored_candidate_repeats_setup_for_its_new_appdata() {
    let t = seeded("seerr", "failed", true).await;
    t.script(
        "",
        json!({"stage":"recovery-required","boundary_at":1,"command":null}),
    );
    t.script(
        "/recovery",
        json!({"stage":"activated","boundary_at":1,"replacement":"candidate","production":{"generation":"second"},
            "command":{"state":"applied","phase":"activating"}}),
    );
    let (status, view) = t.recover("restore_snapshot").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let row = t.row().await;
    assert_eq!(row.state, "committed");
    assert_eq!(row.replacement_id.as_deref(), Some("candidate"));
    // Setup and checks of the discarded appdata no longer count, but are kept as evidence.
    assert_eq!(row.setup, json!([]));
    assert_eq!(row.checks, json!([]));
    assert_eq!(
        row.recovery["superseded"][0]["setup"][0]["generation"],
        "first"
    );
    assert_eq!(
        t.query::<String>("SELECT container_id FROM stack_provisions")
            .await,
        "candidate"
    );
    assert!(
        !t.query::<bool>("SELECT operational FROM support_services")
            .await
    );
    let setup = json!([{"action":"seerr.managed","state":"applied","generation":"first"}]);
    assert!(checks::applied(&setup, "seerr.managed", "first").is_some());
    assert!(checks::applied(&setup, "seerr.managed", "second").is_none());
}

#[tokio::test]
async fn completion_releases_this_server_only_after_the_controller_agrees() {
    let t = seeded("radarr", "committed", true).await;
    // The controller can't be reached: both sides stay held and the job waits.
    let waited = CONTROLLER_FAILURE
        .scope(("dependency_unavailable", "connection reset"), t.job())
        .await
        .unwrap();
    assert!(!waited);
    let row = t.row().await;
    assert_eq!(row.state, "committed");
    assert!(row.progress["waiting"].is_string());
    assert_eq!(
        t.query::<Option<String>>("SELECT hold FROM stack_provisions")
            .await
            .as_deref(),
        Some("transfer")
    );
    assert!(
        !t.query::<bool>("SELECT operational FROM manager_services")
            .await
    );
    // The controller completed and the reply was lost: the next pass records agreement.
    t.script(
        "",
        json!({"stage":"complete","completed":true,"committed":true}),
    );
    assert!(t.job().await.unwrap());
    assert_eq!(t.row().await.state, "complete");
    assert_eq!(
        t.query::<Option<String>>("SELECT hold FROM stack_provisions")
            .await,
        None
    );
    assert!(
        t.query::<bool>("SELECT operational FROM manager_services")
            .await
    );
    // A finished transfer is never demoted by a later error.
    storage::fail(&t.state.db, t.id.clone(), None, "late".into())
        .await
        .unwrap();
    assert_eq!(t.row().await.state, "complete");
}

#[tokio::test]
async fn a_cancelled_transfer_is_never_reopened_by_a_job_already_under_way() {
    let t = seeded("radarr", "queued", false).await;
    t.script("/cancel", json!({"id":t.id,"stage":"cancelled"}));
    let cancelled = call(
        &t.state,
        &format!("/api/v1/admin/transfers/{}/cancel", t.id),
        "POST",
        json!({}),
        &t.cookie,
    )
    .await;
    assert_eq!(cancelled.0, StatusCode::OK, "{}", cancelled.2);
    // The transfer job read the record before the cancel; its accepted start, or a start the
    // controller refused because the review is gone, is recorded afterwards.
    storage::update(
        &t.state.db,
        t.id.clone(),
        Some("running".into()),
        Some("stop".into()),
        Some(json!({})),
        Some(None),
    )
    .await
    .unwrap();
    storage::outcome(
        &t.state.db,
        t.id.clone(),
        "returned".into(),
        Some("Review not found; nothing changed".into()),
        None,
    )
    .await
    .unwrap();
    let row = t.row().await;
    assert_eq!((row.state.as_str(), row.error), ("cancelled", None));
    assert_eq!(
        t.query::<i64>("SELECT COUNT(*) FROM audit WHERE action='transfer.returned'")
            .await,
        0
    );
}

#[tokio::test]
async fn a_controller_outage_while_driving_is_observed_again_not_failed() {
    let t = seeded("radarr", "running", false).await;
    let waited = CONTROLLER_FAILURE
        .scope(("dependency_unavailable", "connection reset"), t.job())
        .await
        .unwrap();
    assert!(!waited);
    assert_eq!(t.row().await.state, "running");
    assert_eq!(
        t.query::<String>("SELECT state FROM jobs WHERE kind='service.transfer'")
            .await,
        "queued"
    );
    // The controller returned the original before activation meanwhile.
    t.script(
        "",
        json!({"stage":"returned","boundary_at":null,"error":"Isolated validation failed"}),
    );
    assert!(t.job().await.unwrap());
    let row = t.row().await;
    assert_eq!(row.state, "returned");
    assert_eq!(row.error.as_deref(), Some("Isolated validation failed"));
}
