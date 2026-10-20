use super::*;
use crate::test_support::{Docker, Fault};

const KEY: &str = "33333333-3333-3333-3333-333333333333";
/// The original's restart policy as reviewed; a running transfer disables it.
const REVIEWED: &str = "unless-stopped";

fn original(running: bool, restart: &str) -> Value {
    json!({"Id":"original","Name":"/radarr","Image":"old-image",
        "State":{"Running":running,"StartedAt":"2026-10-01T00:00:00Z"},
        "Config":{"Image":"old-image","Labels":{}},
        "HostConfig":{"RestartPolicy":{"Name":restart,"MaximumRetryCount":0}},
        "NetworkSettings":{"Networks":{"media":{}}}})
}

fn replacement(running: bool) -> Value {
    json!({"Id":"replacement","Name":"/thelxinoe-radarr-replacement","Image":"old-image",
        "State":{"Running":running,"StartedAt":"2026-10-02T00:00:00Z"},
        "Config":{"Image":"old-image","Labels":{"app.thelxinoe.deployment":"deployment",
            "app.thelxinoe.managed-id":KEY,"app.thelxinoe.transfer":KEY}},
        "HostConfig":{},"NetworkSettings":{"Networks":{"media":{}}}})
}

fn managed() -> Managed {
    Managed {
        id: KEY.into(),
        kind: "radarr".into(),
        container: "replacement".into(),
        name: "thelxinoe-radarr-replacement".into(),
        image: "old-image".into(),
        phase: "active".into(),
        active_update: None,
        spec: json!({}),
        expected: Value::Null,
        hold: Some("transfer".into()),
        origin: "transferred".into(),
        transfer: Some(KEY.into()),
        error: None,
    }
}

/// A Radarr transfer of a running original, journaled in `stage` with the original stopped
/// and its restart disabled. `boundary` says whether the replacement already ran in production.
fn journal(stage: &str, boundary: bool) -> Transfer {
    let version = json!({"numbers":[1],"build":null,"text":"1"});
    let mount = json!({"kind":"bind","source":"/srv/config","destination":"/config","writable":true,"role":"appdata"});
    serde_json::from_value(json!({
        "id":KEY,"kind":"radarr","stage":stage,"error":null,
        "review":{"id":KEY,"kind":"radarr","service":"service","deployment":"deployment","engine":"engine",
            "original":original(true, REVIEWED),"source_image":"old-image","source_version":version,
            "decision":{"decision":"same","image":"old-image"},"target_version":version,
            "layout":{"appdata":mount,"retained":[],"supplementary":[]},"source_identity":[1000,1000],
            "target_identity":[10001,10001],"aliases":[],"published":null,"automation":[],"compose":null,
            "inspection":{},"access":{},"changes":[],"settings":[],"transformations":{},"blockers":[],
            "revision":"revision","created_at":0},
        "source_running":true,"restart_disabled":true,"snapshot":{"digest":"snapshot"},
        "source_state":null,"qualification":null,
        "production":boundary.then(|| json!({"digest":"first","generation":"one"})),
        "intent":null,"replacement":boundary.then_some("replacement"),
        "boundary_at":boundary.then_some(100),"activated":boundary,"committed":false,
        "created_at":0,"updated_at":0
    }))
    .unwrap()
}

fn command(action: &str, phase: &str) -> Command {
    Command {
        operation: thelxinoe_core::id(),
        action: action.into(),
        approval: json!({}),
        from: "recovery-required".into(),
        phase: phase.into(),
        state: "running".into(),
        error: None,
        generation: None,
        started_at: 0,
        finished_at: None,
    }
}

// The first irreversible steps survive a crash: the restart stays disabled, the original is
// stopped and copied once, and a failure before the boundary then gives it back running.
#[tokio::test]
async fn an_interrupted_start_copies_the_stopped_original_and_returns_it_on_failure() {
    let docker = Docker::new();
    docker.insert("original", original(true, REVIEWED));
    let root = docker.root().to_owned();
    std::fs::write(root.join("appdata/marker"), "original-state").unwrap();
    docker
        .scope(async {
            let d: Deployment = serde_json::from_value(json!({"id":"deployment","generation":1,
                "version":"test","server":{},"controller":{},"network":"media",
                "media_source":"/media","appdata_source":root}))
            .unwrap();
            let mut t = journal("approved", false);
            t.restart_disabled = false;
            t.review.layout.appdata.source = root.join("appdata").to_string_lossy().into();
            write(&mut t).unwrap();
            docker.fault(Fault::JournalAfter("stopping"));
            assert!(execute(&d, &mut t).await.is_err());
            assert_eq!(
                docker.container("original")["HostConfig"]["RestartPolicy"]["Name"],
                "no"
            );
            let mut t = read(KEY).unwrap();
            assert_eq!((t.stage.as_str(), t.restart_disabled), ("stopping", true));
            // Resumed, the original stops and is copied; then validation can't proceed.
            docker.fault(Fault::JournalBefore("validating"));
            run(&d, &mut t).await;
            assert_eq!(t.stage, "returned", "{:?}", t.error);
            assert_eq!(t.snapshot.as_ref().unwrap()["files"], 1);
            // With the original back, nothing can use the snapshot.
            assert!(!directory(KEY).join("snapshot").exists());
        })
        .await;
    assert_eq!(docker.copies(), 1);
    assert!(docker.running("original"));
    assert_eq!(
        docker.container("original")["HostConfig"]["RestartPolicy"]["Name"],
        REVIEWED
    );
    assert!(
        docker
            .names()
            .iter()
            .all(|name| !name.starts_with("/thelxinoe-transfer-"))
    );
}

// Before the boundary a refused transfer gives the original back as reviewed: its restart
// policy returns, and it runs again if it was running, whoever started it meanwhile.
#[tokio::test]
async fn an_original_that_can_write_again_returns_the_transfer_before_the_boundary() {
    for (running, restart) in [(true, "no"), (false, REVIEWED)] {
        let docker = Docker::new();
        docker.insert("original", original(running, restart));
        docker
            .scope(async {
                let (d, _): (Deployment, Value) = docker.update();
                let mut t = journal("creating", false);
                write(&mut t).unwrap();
                run(&d, &mut t).await;
                assert_eq!(t.stage, "returned", "{:?}", t.error);
                assert_eq!(read(KEY).unwrap().stage, "returned");
            })
            .await;
        assert_eq!(docker.creates(), 0);
        assert!(docker.running("original"));
        assert_eq!(
            docker.container("original")["HostConfig"]["RestartPolicy"]["Name"],
            REVIEWED
        );
    }
}

// A restart while the original is being given back leaves `restoring` in the journal, so
// resuming finishes the restore instead of continuing the transfer that failed.
#[tokio::test]
async fn an_interrupted_restore_finishes_instead_of_resuming_the_transfer() {
    let docker = Docker::new();
    // Its restart policy is enabled again, so the replacement can't be created.
    docker.insert("original", original(false, REVIEWED));
    docker
        .scope(async {
            let (d, _): (Deployment, Value) = docker.update();
            store::write_json(&store::root().join("desired-state.json"), &d).unwrap();
            let mut t = journal("creating", false);
            write(&mut t).unwrap();
            // The restore stops before the original starts, and its outcome is never journaled.
            docker.fault(Fault::StartBefore("original"));
            docker.fault(Fault::JournalBefore("failed"));
            run(&d, &mut t).await;
            assert_eq!(read(KEY).unwrap().stage, "restoring");
            assert!(!docker.running("original"));
            let runtime = Runtime(Default::default());
            let resumed = resume(State(runtime.clone()), Path(KEY.into()))
                .await
                .unwrap();
            assert_eq!(resumed.0["operation_active"], true);
            drop(runtime.0.service("radarr").await);
            let t = read(KEY).unwrap();
            assert_eq!(t.stage, "returned", "{:?}", t.error);
            assert!(t.error.unwrap().contains("restart policy is enabled again"));
        })
        .await;
    assert_eq!(docker.creates(), 0);
    assert!(docker.running("original"));
    assert_eq!(
        docker.container("original")["HostConfig"]["RestartPolicy"]["Name"],
        REVIEWED
    );
}

// A cancel while a start holds the service is refused and keeps the review. Once the cancel
// removed it, a start finds nothing to begin and writes no journal.
#[tokio::test]
async fn a_cancel_never_interleaves_with_a_start() {
    let docker = Docker::new();
    docker
        .scope(async {
            let review = journal("approved", false).review;
            let path = directory(KEY).join("review.json");
            store::write_secret(&path, &format!("review:{KEY}"), &review).unwrap();
            let runtime = Runtime(Default::default());
            // A start rechecking the original before it journals.
            let starting = runtime.0.try_service("radarr").unwrap();
            let refused = cancel(State(runtime.clone()), Path(KEY.into()))
                .await
                .expect_err("cancel refused while a start runs");
            assert_eq!(refused.0, StatusCode::CONFLICT);
            assert!(path.exists());
            drop(starting);
            let cancelled = cancel(State(runtime.clone()), Path(KEY.into()))
                .await
                .unwrap();
            assert_eq!(cancelled.0["stage"], "cancelled");
            let input = serde_json::from_value(json!({"revision":review.revision})).unwrap();
            let started = start(State(runtime), Path(KEY.into()), Json(input)).await;
            assert_eq!(started.err().unwrap().0, StatusCode::NOT_FOUND);
            assert!(!directory(KEY).exists());
        })
        .await;
    assert_eq!(docker.creates(), 0);
}

// Once the replacement is being retired a return can only continue. A lost journal
// acknowledgement in any of its phases resumes from the journal and keeps the replacement's
// appdata exactly once.
#[tokio::test]
async fn a_return_to_the_original_resumes_from_its_journal() {
    for phase in ["retiring-replacement", "releasing", "starting-original"] {
        let docker = Docker::new();
        docker.insert("original", original(false, "no"));
        docker.insert("replacement", replacement(false));
        docker
            .scope(async {
                let (d, _): (Deployment, Value) = docker.update();
                save(&managed()).unwrap();
                let appdata = service_path(KEY).with_file_name("appdata");
                std::fs::create_dir_all(&appdata).unwrap();
                std::fs::write(appdata.join("marker"), "replacement-state").unwrap();
                let mut t = journal("recovering", true);
                t.command = Some(command("return_original", "retiring-replacement"));
                write(&mut t).unwrap();
                docker.fault(Fault::PhaseAfter(phase));
                run_command(&d, &mut t).await;
                let mut t = read(KEY).unwrap();
                assert_eq!(t.stage, "recovering", "{phase}");
                assert!(!docker.running("original"), "{phase}");
                run_command(&d, &mut t).await;
                assert_eq!(t.stage, "returned", "{phase}: {:?}", t.error);
                assert_eq!(t.command.as_ref().unwrap().state, "applied");
                assert_eq!(t.generations.len(), 1, "{phase}");
                assert_eq!(t.generations[0]["from"], "returned");
                assert_eq!(
                    std::fs::read_to_string(directory(KEY).join("generation-1/marker")).unwrap(),
                    "replacement-state"
                );
                assert!(!appdata.exists());
                let s = load(KEY).unwrap();
                assert_eq!((s.phase.as_str(), s.hold), ("returned", None));
            })
            .await;
        assert!(
            !docker
                .names()
                .contains(&"/thelxinoe-radarr-replacement".to_owned()),
            "{phase}"
        );
        assert!(docker.running("original"), "{phase}");
        assert_eq!(
            docker.container("original")["HostConfig"]["RestartPolicy"]["Name"],
            REVIEWED
        );
    }
}

/// An unrelated running container with Docker access.
fn docker_access(name: &str, image: &str, env: Value) -> Value {
    json!({"Id":name,"Name":format!("/{name}"),"State":{"Running":true},
        "Config":{"Image":image,"Labels":{},"Env":env},"HostConfig":{},
        "Mounts":[{"Type":"bind","Source":"/var/run/docker.sock","Destination":"/var/run/docker.sock","RW":true}],
        "NetworkSettings":{"Networks":{}}})
}

// A return checks what could restart the original before the replacement stops. An updater
// that would restart it refuses the return and the replacement keeps running; a dashboard
// with Docker access that appeared since the review was declared and doesn't block it.
#[tokio::test]
async fn a_return_checks_what_could_restart_the_original_before_the_replacement_stops() {
    let docker = Docker::new();
    docker.insert("original", original(false, "no"));
    docker.insert("replacement", replacement(true));
    docker.insert(
        "dashboard",
        docker_access("dashboard", "ghcr.io/example/dashboard:1", json!([])),
    );
    docker.insert(
        "watchtower",
        docker_access("watchtower", "containrrr/watchtower:1.7", json!([])),
    );
    docker
        .scope(async {
            let (d, _): (Deployment, Value) = docker.update();
            save(&managed()).unwrap();
            let mut t = journal("recovering", true);
            t.command = Some(command("return_original", "accepted"));
            write(&mut t).unwrap();
            run_command(&d, &mut t).await;
            assert_eq!(t.stage, "recovery-required");
            let refused = t.command.as_ref().unwrap();
            assert_eq!(refused.state, "failed");
            assert!(
                refused
                    .error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("watchtower updates containers")),
                "{:?}",
                refused.error
            );
            assert!(docker.running("replacement"));
            // Limited to labelled containers, the updater leaves the original alone.
            docker.insert(
                "watchtower",
                docker_access(
                    "watchtower",
                    "containrrr/watchtower:1.7",
                    json!(["WATCHTOWER_LABEL_ENABLE=true"]),
                ),
            );
            t.stage = "recovering".into();
            t.command = Some(command("return_original", "accepted"));
            write(&mut t).unwrap();
            run_command(&d, &mut t).await;
            assert_eq!(t.stage, "returned", "{:?}", t.error);
        })
        .await;
    assert!(docker.running("original"));
    assert!(
        !docker
            .names()
            .contains(&"/thelxinoe-radarr-replacement".to_owned())
    );
}

// Completion keeps only what may still be needed: the snapshot and its supplementary copies
// go, the appdata generations a replacement wrote stay until they are discarded explicitly.
#[tokio::test]
async fn completion_discards_the_snapshot_and_keeps_preserved_generations() {
    let docker = Docker::new();
    docker.insert("original", original(false, "no"));
    docker.insert("replacement", replacement(true));
    let root = docker.root().to_owned();
    docker
        .scope(async {
            let d: Deployment = serde_json::from_value(json!({"id":"deployment","generation":1,
                "version":"test","server":{},"controller":{},"network":"media",
                "media_source":"/media","appdata_source":root}))
            .unwrap();
            store::write_json(&root.join("desired-state.json"), &d).unwrap();
            save(&managed()).unwrap();
            let mut t = journal("committed", true);
            t.committed = true;
            t.generations = vec![json!({"name":"generation-1","from":"replacement"})];
            write(&mut t).unwrap();
            for leaf in ["snapshot", "supplementary/0", "generation-1"] {
                std::fs::create_dir_all(directory(KEY).join(leaf)).unwrap();
                std::fs::write(directory(KEY).join(leaf).join("marker"), leaf).unwrap();
            }
            let completed = complete(State(Runtime(Default::default())), Path(KEY.into()))
                .await
                .unwrap();
            assert_eq!(completed.0["stage"], "complete");
            assert!(!directory(KEY).join("snapshot").exists());
            assert!(!directory(KEY).join("supplementary").exists());
            assert_eq!(
                std::fs::read_to_string(directory(KEY).join("generation-1/marker")).unwrap(),
                "generation-1"
            );
            assert!(read(KEY).unwrap().completed());
            assert_eq!(load(KEY).unwrap().hold, None);
        })
        .await;
    // The cleanup workers ran and were removed.
    assert!(
        docker
            .names()
            .iter()
            .all(|name| !name.starts_with("/thelxinoe-transfer-"))
    );
}

// Repair starts the retained replacement again only while the original can't write; a
// refusal leaves everything as it was.
#[tokio::test]
async fn repair_starts_the_replacement_only_while_the_original_is_quiet() {
    let docker = Docker::new();
    docker.insert("original", original(true, "no"));
    docker.insert("replacement", replacement(false));
    docker
        .scope(async {
            let (d, _): (Deployment, Value) = docker.update();
            save(&managed()).unwrap();
            let mut t = journal("recovering", true);
            t.command = Some(command("repair", "accepted"));
            write(&mut t).unwrap();
            run_command(&d, &mut t).await;
            assert_eq!(t.stage, "recovery-required");
            assert_eq!(t.command.as_ref().unwrap().state, "failed");
            assert!(!docker.running("replacement"));
            assert_eq!(docker.starts(), 0);
            docker.insert("original", original(false, "no"));
            t.command = Some(command("repair", "accepted"));
            t.stage = "recovering".into();
            write(&mut t).unwrap();
            run_command(&d, &mut t).await;
            assert_eq!(t.stage, "activated", "{:?}", t.error);
            assert!(docker.running("replacement"));
            assert_eq!(load(KEY).unwrap().phase, "active");
            // The health checker ran once and was removed.
            assert!(docker.names().iter().all(|name| !name.ends_with("-health")));
        })
        .await;
}
