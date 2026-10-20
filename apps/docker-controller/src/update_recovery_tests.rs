use super::*;
use crate::test_support::{Docker, Fault};

#[tokio::test]
async fn update_api_reports_live_operation_ownership_without_persisting_it() {
    let fixture = Docker::new();
    fixture
        .scope(async {
            let (_, mut update): (Deployment, Update) = fixture.update();
            update.stage = "rollback-activating".into();
            update.rollback_phase = Some(RollbackPhase::Activating);
            write(&update).unwrap();
            let runtime = Runtime(Default::default());
            let context = crate::test_support::DOCKER.with(Clone::clone);
            let app = axum::Router::new()
                .route("/stack/updates", axum::routing::get(list))
                .with_state(runtime.clone())
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| {
                        let context = context.clone();
                        async move {
                            crate::test_support::DOCKER
                                .scope(context, next.run(request))
                                .await
                        }
                    },
                ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/stack/updates", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client = reqwest::Client::new();
            let owner = runtime.0.try_service("radarr").unwrap();
            let live: Value = client.get(&url).send().await.unwrap().json().await.unwrap();
            assert_eq!(live["items"][0]["operation_active"], true);
            drop(owner);
            let unrelated = runtime.0.try_service("sonarr").unwrap();
            let orphan: Value = client.get(&url).send().await.unwrap().json().await.unwrap();
            assert_eq!(orphan["items"][0]["operation_active"], false);
            drop(unrelated);
            let deployment = runtime.0.try_lock_owned().unwrap();
            let protected: Value = client.get(&url).send().await.unwrap().json().await.unwrap();
            assert_eq!(protected["items"][0]["operation_active"], true);
            drop(deployment);
            assert_eq!(
                serde_json::to_value(read(&update.id).unwrap())
                    .unwrap()
                    .get("operation_active"),
                None
            );
            server.abort();
        })
        .await;
}

#[tokio::test]
async fn rollback_restart_and_journal_faults_never_replay_resumed_appdata() {
    for fault in [
        Fault::JournalBefore("rollback-activating"),
        Fault::JournalAfter("rollback-activating"),
        Fault::StartBefore("original"),
        Fault::StartReply("original"),
    ] {
        let fixture = Docker::new();
        fixture
            .scope(async {
                let (mut d, mut u): (Deployment, Update) = fixture.update();
                d.appdata_source = fixture.root().to_string_lossy().into_owned();
                u.recovery_complete = true;
                let mut value = serde_json::to_value(&u).unwrap();
                value["rollback_phase"] = json!("copying");
                u = serde_json::from_value(value).unwrap();
                let snapshot = path(&u.id).join("recovery");
                std::fs::create_dir_all(&snapshot).unwrap();
                std::fs::write(snapshot.join("marker"), "recovery-snapshot").unwrap();
                write(&u).unwrap();
                fixture.fault(fault);
                assert!(rollback(&d, &mut u).await.is_err());
                let copies = fixture.copies();
                if fixture.running("original") {
                    std::fs::write(fixture.root().join("appdata/marker"), "new-production-data")
                        .unwrap();
                }
                let mut recovered = read(&u.id).unwrap();
                rollback(&d, &mut recovered).await.unwrap();
                let expected_copies = if fault == Fault::JournalBefore("rollback-activating") {
                    copies + 1
                } else {
                    copies
                };
                assert_eq!(fixture.copies(), expected_copies);
                std::fs::write(fixture.root().join("appdata/marker"), "new-production-data")
                    .unwrap();
                let mut recovered = read(&u.id).unwrap();
                rollback(&d, &mut recovered).await.unwrap();
                assert_eq!(fixture.copies(), expected_copies);
                assert_eq!(
                    std::fs::read_to_string(fixture.root().join("appdata/marker")).unwrap(),
                    "new-production-data"
                );
                assert!(fixture.running("original"));
            })
            .await;
    }
}

#[tokio::test]
async fn ambiguous_old_rollback_records_require_manual_recovery() {
    let fixture = Docker::new();
    fixture
        .scope(async {
            let (d, mut u): (Deployment, Update) = fixture.update();
            u.recovery_complete = true;
            let mut value = serde_json::to_value(&u).unwrap();
            value.as_object_mut().unwrap().remove("rollback_phase");
            let mut legacy = serde_json::from_value(value).unwrap();
            assert_eq!(
                rollback(&d, &mut legacy).await.unwrap_err().0,
                StatusCode::CONFLICT
            );
            assert_eq!(fixture.copies(), 0);
            assert!(!fixture.running("original"));
        })
        .await;
}

#[tokio::test]
async fn replacement_reply_loss_and_journal_loss_recover_the_exact_attempt() {
    for fault in [
        Fault::CreateReply,
        Fault::CreateReplyAndInspection,
        Fault::JournalWrite,
    ] {
        let fixture = Docker::new();
        fixture
            .scope(async {
                let (deployment, mut update) = fixture.update();
                write(&update).unwrap();
                fixture.fault(fault);
                let result = prepare_replacement(&deployment, &mut update).await;
                if fault != Fault::CreateReply {
                    assert!(result.is_err());
                }
                let mut recovered = read(&update.id).unwrap();
                fixture.reconnect();
                rollback(&deployment, &mut recovered).await.unwrap();
                assert_eq!(fixture.names(), vec!["/service"]);
                assert!(fixture.running("original"));
                assert_eq!(fixture.creates(), 1);
                assert!(!recovered.activation_crossed);
            })
            .await;
    }
}

#[tokio::test]
async fn replacement_recovery_refuses_foreign_names_and_changed_attempts() {
    for changed in [false, true] {
        let fixture = Docker::new();
        fixture
            .scope(async {
                let (deployment, mut update) = fixture.update();
                write(&update).unwrap();
                fixture.fault(Fault::CreateReplyAndInspection);
                assert!(prepare_replacement(&deployment, &mut update).await.is_err());
                let mut recovered = read(&update.id).unwrap();
                fixture.reconnect();
                fixture.change_replacement(changed);
                assert_eq!(
                    rollback(&deployment, &mut recovered).await.unwrap_err().0,
                    StatusCode::CONFLICT
                );
                assert!(fixture.names().contains(&"/service".to_owned()));
                assert_eq!(fixture.deletes(), 0);
            })
            .await;
    }
}

#[tokio::test]
async fn copy_workers_report_their_own_failure() {
    let fixture = Docker::new();
    fixture
        .scope(async {
            fixture.insert(
                "worker",
                json!({"Id":"worker","Name":"/worker","State":{"Running":false,"ExitCode":1}}),
            );
            let report = store::root().join("progress.json");
            std::fs::write(
                &report,
                r#"{"phase":"failed","error":"Restoring needs 10 MiB free"}"#,
            )
            .unwrap();
            assert_eq!(
                wait_copy("worker", &report).await.unwrap_err().1,
                "Restoring needs 10 MiB free"
            );
            std::fs::remove_file(&report).unwrap();
            assert_eq!(
                wait_copy("worker", &report).await.unwrap_err().1,
                "Isolated worker failed its contract"
            );
        })
        .await;
}

// Settled attempts keep only what the newest committed one can recover to: its recovery
// copy and stopped original. Everything an older commit or a preflight left goes.
#[tokio::test]
async fn settled_updates_keep_only_what_the_newest_commit_can_recover_to() {
    const OLDER: &str = "00000000-0000-0000-0000-000000000001";
    const NEWER: &str = "00000000-0000-0000-0000-000000000002";
    const BLOCKED: &str = "00000000-0000-0000-0000-000000000003";
    let fixture = Docker::new();
    fixture
        .scope(async {
            let d: Deployment = fixture.backup_deployment();
            store::write_json(&store::root().join("desired-state.json"), &d).unwrap();
            store::write_json(&store::root().join("generations/1/desired-state.json"), &d).unwrap();
            let (_, template): (Deployment, Update) = fixture.update();
            for (key, stage, original) in [
                // The fixture's stopped original is what the older commit retained.
                (OLDER, "committed", "original"),
                (NEWER, "committed", "newer-prior"),
                (BLOCKED, "blocked", "newer-prior"),
            ] {
                let mut u = template.clone();
                u.id = key.into();
                u.stage = stage.into();
                u.old.container = original.into();
                u.activation_crossed = stage == "committed";
                // Only an activation takes a recovery copy.
                let leaves: &[&str] = if stage == "committed" {
                    &["snapshot", "clone", "scratch-data", "recovery"]
                } else {
                    &["snapshot", "clone", "scratch-data"]
                };
                for leaf in leaves {
                    std::fs::create_dir_all(path(key).join(leaf)).unwrap();
                    std::fs::write(path(key).join(leaf).join("config.xml"), "api-key").unwrap();
                }
                write(&u).unwrap();
                // Journal times order the commits.
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            prune_settled().await.unwrap();
            let left = |key: &str| {
                ["snapshot", "clone", "scratch-data", "recovery"]
                    .into_iter()
                    .filter(|leaf| path(key).join(leaf).exists())
                    .collect::<Vec<_>>()
            };
            assert_eq!(left(OLDER), Vec::<&str>::new());
            assert_eq!(left(NEWER), ["recovery"]);
            assert_eq!(left(BLOCKED), Vec::<&str>::new());
            assert!(!fixture.names().contains(&"/service".to_owned()));
            // Journals stay, so a lost reply still finds its outcome.
            assert!(read(OLDER).is_ok() && read(NEWER).is_ok());
        })
        .await;
}
