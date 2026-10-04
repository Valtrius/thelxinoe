use super::*;
use crate::test_support::{Docker, Fault};

#[tokio::test]
async fn startup_recovers_early_restore_failure_and_accepts_the_same_archive() {
    for fault in [
        Fault::StartBefore("original"),
        Fault::StartReply("original"),
    ] {
        let fixture = Docker::new();
        fixture
            .scope(async {
                let d: Deployment = fixture.backup_deployment();
                store::write_json(&store::root().join("desired-state.json"), &d).unwrap();
                store::write_json(&store::root().join("generations/1/desired-state.json"), &d)
                    .unwrap();
                let (_, mut components) = inspect(&d).await.unwrap();
                components[0].running = true;
                let key = thelxinoe_core::id();
                let mut r = Record {
                    id: key.clone(),
                    stage: "restore-queued".into(),
                    created_at: 1,
                    error: None,
                    components: components.clone(),
                    recovery: Some(format!("recovery-{}", thelxinoe_core::id())),
                    recovery_ready: false,
                    rollback_phase: Some(RollbackPhase::Copying),
                    release_restore: None,
                };
                let snapshot = work(&key).join("archive-source");
                std::fs::create_dir_all(snapshot.join("server")).unwrap();
                std::fs::write(snapshot.join("server/marker"), "valid-archive").unwrap();
                let manifest = Manifest {
                    format: 1,
                    created_at: 1,
                    deployment: d.clone(),
                    services: vec![],
                    components,
                    compose: json!({}),
                };
                store::write_json(&snapshot.join("manifest.json"), &manifest).unwrap();
                let archive = archive_root().join(format!("{key}.age"));
                let passphrase = "early restore regression passphrase";
                thelxinoe_backup::encrypt(&snapshot, &archive, passphrase.into()).unwrap();
                let archive_bytes = std::fs::read(&archive).unwrap();
                record(&r).unwrap();
                fixture.fault(fault);
                assert!(rollback_original(&d, &mut r, false).await.is_err());
                let interrupted: Record = store::read(&work(&key).join("operation.json")).unwrap();
                assert_eq!(interrupted.stage, "rollback-activating");
                assert!(!interrupted.recovery_ready);
                if fixture.running("original") {
                    std::fs::write(fixture.root().join("appdata/marker"), "new-production-data")
                        .unwrap();
                }
                recover_interrupted().await.unwrap();
                let recovered: Record = store::read(&work(&key).join("operation.json")).unwrap();
                assert_eq!(recovered.stage, "restore-failed");
                assert_eq!(fixture.copies(), 0);
                if fault == Fault::StartReply("original") {
                    assert_eq!(
                        std::fs::read_to_string(fixture.root().join("appdata/marker")).unwrap(),
                        "new-production-data"
                    );
                }
                assert_eq!(std::fs::read(&archive).unwrap(), archive_bytes);
                let response = restore(
                    State(Runtime(Default::default())),
                    Path(key.clone()),
                    Json(Input {
                        passphrase: passphrase.into(),
                    }),
                )
                .await
                .unwrap();
                assert_eq!(response.0["stage"], "restore-queued");
                tokio::time::timeout(std::time::Duration::from_secs(15), async {
                    loop {
                        let retried: Record =
                            store::read(&work(&key).join("operation.json")).unwrap();
                        if retried.stage == "restored" {
                            break;
                        }
                        assert!(
                            matches!(
                                retried.stage.as_str(),
                                "restore-queued" | "restoring" | "restore-activating"
                            ),
                            "{}: {:?}",
                            retried.stage,
                            retried.error
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                assert!(fixture.running("original"));
                assert_eq!(fixture.copies(), 2);
            })
            .await;
    }
}

#[tokio::test]
async fn partial_backup_restart_recovery_preserves_new_writes() {
    for fault in [
        Fault::JournalBefore("rollback-activating"),
        Fault::JournalAfter("rollback-activating"),
        Fault::StartBefore("original"),
        Fault::StartReply("second"),
    ] {
        let fixture = Docker::new();
        fixture.scope(async {
            let (mut d, _): (Deployment,Value) = fixture.update();
            d.appdata_source = fixture.root().to_string_lossy().into_owned();
            let second = fixture.add_component("second");
            let mut r: Record = serde_json::from_value(json!({"id":"22222222-2222-2222-2222-222222222222","stage":"restoring","created_at":1,"error":null,"components":[{"key":"server","container":"original","source":fixture.root().join("appdata"),"running":true},{"key":"service","container":"second","source":second,"running":true}],"recovery":"recovery-33333333-3333-3333-3333-333333333333","recovery_ready":true,"rollback_phase":"copying"})).unwrap();
            for key in ["server","service"] {
                let source = work(&r.id).join(r.recovery.as_ref().unwrap()).join(key);
                std::fs::create_dir_all(&source).unwrap();
                std::fs::write(source.join("marker"),"recovery-snapshot").unwrap();
            }
            record(&r).unwrap();
            fixture.fault(fault);
            assert!(rollback_original(&d,&mut r,true).await.is_err());
            let copies = fixture.copies();
            if fixture.running("second") {
                std::fs::write(fixture.root().join("second/marker"),"new-production-data").unwrap();
            }
            let mut recovered: Record = store::read(&work(&r.id).join("operation.json")).unwrap();
            rollback_original(&d,&mut recovered,true).await.unwrap();
            let expected_copies = if fault == Fault::JournalBefore("rollback-activating") { copies+2 } else { copies };
            assert_eq!(fixture.copies(),expected_copies);
            for source in ["appdata","second"] { std::fs::write(fixture.root().join(source).join("marker"),"new-production-data").unwrap(); }
            let mut recovered: Record = store::read(&work(&r.id).join("operation.json")).unwrap();
            rollback_original(&d,&mut recovered,true).await.unwrap();
            assert_eq!(fixture.copies(),expected_copies);
            for source in ["appdata","second"] { assert_eq!(std::fs::read_to_string(fixture.root().join(source).join("marker")).unwrap(),"new-production-data"); }
            assert!(fixture.running("original") && fixture.running("second"));
        }).await;
    }
}
