use super::*;
use crate::test_support::{Docker, Fault};

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
