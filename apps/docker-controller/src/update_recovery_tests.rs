use super::*;
use crate::test_support::{Docker, Fault};

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
