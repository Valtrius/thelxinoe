use super::*;
use crate::test_support::{Docker, Fault};

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
