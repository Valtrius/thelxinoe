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
                let (_, mut components, _) = inspect(&d).await.unwrap();
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
                    automatic: false,
                    contents: vec![],
                    excluded: vec![],
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
                    excluded: vec![],
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
                    Json(RestoreInput {
                        passphrase: passphrase.into(),
                        components: None,
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

#[tokio::test]
async fn automatic_rotation_spares_manual_imported_and_unfinished_backups() {
    let fixture = Docker::new();
    fixture
        .scope(async {
            let operation = |n: u8, stage: &str, automatic: bool| {
                let r: Record = serde_json::from_value(json!({"id":format!("00000000-0000-0000-0000-{n:012}"),"stage":stage,"created_at":n,"error":null,"components":[],"automatic":automatic})).unwrap();
                private_dir(&work(&r.id)).unwrap();
                record(&r).unwrap();
                if archived(stage) {
                    std::fs::write(archive_root().join(format!("{}.age", r.id)), "archive").unwrap();
                }
                r
            };
            let manual = operation(1, "complete", false);
            let oldest = operation(2, "complete", true);
            let older = operation(3, "complete", true);
            let restored = operation(4, "restored", true);
            let failed = operation(5, "failed", true);
            let unfinished = operation(6, "recovery-required", true);
            let imported = "00000000-0000-0000-0000-000000000099";
            std::fs::write(archive_root().join(format!("{imported}.age")), "archive").unwrap();
            let exists = |id: &str| work(id).join("operation.json").is_file() || archive_root().join(format!("{id}.age")).is_file();
            // A failed attempt only supersedes older failures; every archive survives it.
            let attempt = operation(7, "failed", true);
            prune(&attempt, 1).unwrap();
            assert!(!exists(&failed.id));
            assert!([&oldest, &older, &restored, &attempt].iter().all(|r| exists(&r.id)));
            let current = operation(8, "complete", true);
            prune(&current, 2).unwrap();
            for removed in [&oldest, &older, &attempt] {
                assert!(!exists(&removed.id), "{}", removed.id);
            }
            for kept in [&manual, &restored, &unfinished, &current] {
                assert!(exists(&kept.id), "{}", kept.id);
            }
            assert!(exists(imported));
            assert_eq!(list().await.unwrap().0["items"].as_array().unwrap().len(), 5);
        })
        .await;
}

#[tokio::test]
async fn retried_automatic_backup_returns_the_original_operation() {
    let fixture = Docker::new();
    fixture
        .scope(async {
            let key = thelxinoe_core::id();
            let r: Record = serde_json::from_value(json!({"id":key,"stage":"snapshotting","created_at":1,"error":null,"components":[],"automatic":true})).unwrap();
            private_dir(&work(&key)).unwrap();
            record(&r).unwrap();
            let runtime = Runtime(Default::default());
            // The original backup still owns the operation lock.
            let _held = runtime.0.try_lock_owned().unwrap();
            let request = |retain: usize| {
                Json(serde_json::from_value::<Creation>(json!({"passphrase":"retried automatic passphrase","automatic":{"id":key,"retain":retain}})).unwrap())
            };
            let response = create(State(runtime.clone()), request(7)).await.unwrap();
            assert_eq!(response.0["id"], key);
            assert_eq!(response.0["stage"], "snapshotting");
            assert_eq!(response.0["automatic"], true);
            assert!(create(State(runtime.clone()), request(0)).await.is_err());
            let manual = Json(serde_json::from_value::<Creation>(json!({"passphrase":"retried automatic passphrase"})).unwrap());
            assert!(create(State(runtime), manual).await.is_err());
        })
        .await;
}

#[tokio::test]
async fn backups_leave_unsupported_imported_services_running_and_restore_them_only_when_chosen() {
    const INSTALLED: &str = "44444444-4444-4444-4444-444444444444";
    const KEPT: &str = "55555555-5555-5555-5555-555555555555";
    const SKIPPED: &str = "66666666-6666-6666-6666-666666666666";
    let fixture = Docker::new();
    fixture
        .scope(async {
            let d: Deployment = fixture.backup_deployment();
            store::write_json(&store::root().join("desired-state.json"), &d).unwrap();
            store::write_json(&store::root().join("generations/1/desired-state.json"), &d)
                .unwrap();
            store::write_json(&store::root().join("compose.override.yaml"), &json!({})).unwrap();
            updates::start("original").await.unwrap();
            let capability = json!({"available":true,"reason":null});
            let imported = json!({"engine":"engine","deployment":"deployment","storage":[],
                "capabilities":{"lifecycle":capability,"backup":capability,"update":capability,
                    "recreate":capability,"release":capability}});
            for (key, container, kind, config, labels, import) in [
                (INSTALLED, "installed", "prowlarr", json!({"Type":"bind","Source":fixture.root().join("installed"),"Destination":"/config","RW":true}),
                    json!({"app.thelxinoe.deployment":"deployment","app.thelxinoe.managed-id":INSTALLED}), Value::Null),
                (KEPT, "kept", "radarr", json!({"Type":"bind","Source":fixture.root().join("kept"),"Destination":"/config","RW":true}),
                    json!({}), imported.clone()),
                // A Docker volume can't be backed up, so this service stays out of every archive.
                (SKIPPED, "skipped", "sonarr", json!({"Type":"volume","Name":"sonarr_config","Source":"/var/lib/docker/volumes/sonarr_config/_data","Destination":"/config","RW":true}),
                    json!({}), imported.clone()),
            ] {
                if config["Type"] == "bind" {
                    let source = std::path::PathBuf::from(config["Source"].as_str().unwrap());
                    std::fs::create_dir_all(&source).unwrap();
                    std::fs::write(source.join("marker"), format!("{container}-before")).unwrap();
                }
                let raw = json!({"Id":container,"Name":format!("/{container}"),"Image":"sha256:image",
                    "Config":{"Image":"image","Labels":labels},"HostConfig":{},"Mounts":[config],
                    "State":{"Running":true,"StartedAt":"2026-10-01T00:00:00Z"},
                    "NetworkSettings":{"Networks":{"media":{}}}});
                fixture.insert(container, raw.clone());
                let service: Managed = serde_json::from_value(json!({"id":key,"kind":kind,
                    "container":container,"name":container,"image":"image","phase":"active",
                    "spec":{},"expected":policy::fingerprint(&raw),"imported":import})).unwrap();
                save(&service).unwrap();
            }
            std::fs::write(fixture.root().join("appdata/marker"), "server-before").unwrap();
            let skipped_started = fixture.started_at("skipped");
            let passphrase = "partial backup regression passphrase";
            let created = create(
                State(Runtime(Default::default())),
                Json(Creation {
                    passphrase: passphrase.into(),
                    automatic: None,
                }),
            )
            .await
            .unwrap()
            .0;
            assert_eq!(created["excluded"][0]["key"], SKIPPED);
            assert!(
                created["excluded"][0]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("Docker volume sonarr_config")
            );
            let contents = |value: &Value| {
                value["components"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| (c["key"].as_str().unwrap().to_owned(), c["imported"] == true))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                contents(&created),
                [
                    ("server".to_owned(), false),
                    (INSTALLED.to_owned(), false),
                    (KEPT.to_owned(), true)
                ]
            );
            let key = created["id"].as_str().unwrap().to_owned();
            let finished = |stage: &'static str| {
                let key = key.clone();
                async move {
                    tokio::time::timeout(std::time::Duration::from_secs(15), async {
                        loop {
                            let r: Record =
                                store::read(&work(&key).join("operation.json")).unwrap();
                            if r.stage == stage {
                                return r;
                            }
                            assert!(r.error.is_none(), "{}: {:?}", r.stage, r.error);
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap()
                }
            };
            finished("complete").await;
            assert_eq!(fixture.copies(), 3);
            let restore_with = |components: Option<Vec<String>>| {
                restore(
                    State(Runtime(Default::default())),
                    Path(key.clone()),
                    Json(RestoreInput {
                        passphrase: passphrase.into(),
                        components,
                    }),
                )
            };
            // Without a choice, the adopted service keeps running with its live configuration.
            let kept_started = fixture.started_at("kept");
            assert_eq!(
                restore_with(None).await.unwrap().0["stage"],
                "restore-queued"
            );
            finished("restored").await;
            assert_eq!(fixture.copies(), 3 + 4);
            assert_eq!(fixture.started_at("kept"), kept_started);
            for (rejected, status) in [
                (vec![], StatusCode::BAD_REQUEST),
                (vec![KEPT.to_owned(), KEPT.to_owned()], StatusCode::BAD_REQUEST),
                (vec![SKIPPED.to_owned()], StatusCode::CONFLICT),
            ] {
                assert_eq!(restore_with(Some(rejected)).await.unwrap_err().0, status);
            }
            // An explicit choice restores only that service; the server keeps running.
            let server_started = fixture.started_at("original");
            assert_eq!(
                restore_with(Some(vec![KEPT.to_owned()])).await.unwrap().0["stage"],
                "restore-queued"
            );
            let r = finished("restored").await;
            assert_eq!(fixture.copies(), 3 + 4 + 2);
            assert_ne!(fixture.started_at("kept"), kept_started);
            assert_eq!(fixture.started_at("original"), server_started);
            assert_eq!(contents(&public(&r)).len(), 3);
            assert!(fixture.running("skipped"));
            assert_eq!(fixture.started_at("skipped"), skipped_started);
        })
        .await;
}
