use crate::test_support::call;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[tokio::test]
async fn prepared_first_install_reports_running_while_verification_is_blocked() {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

    assert!(
        std::path::Path::new(super::PYTHON).is_file(),
        "Package operations run {}; scripts/Dockerfile.verify links it to the system Python",
        super::PYTHON
    );
    let (_temp, state, cookie) = crate::test_support::fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let generation_id = "0123456789abcdef0123456789abcdef";
    let directory = state.tools.root.join("packages").join(generation_id);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let entered = directory.join("verification-entered");
    let resume = directory.join("verification-resume");
    let ffmpeg = format!(
        "#!/bin/sh\ncase \"$1\" in\n-version)\n touch '{}'\n while [ ! -e '{}' ]; do sleep 0.02; done\n echo 'ffmpeg version n1.0'\n ;;\n*) echo 'libx264 aac zscale tonemap yadif https tls crypto chromaprint';;\nesac\n",
        entered.display(),
        resume.display()
    );
    let ffprobe = "#!/bin/sh\ncase \"$1\" in\n-version) echo 'ffprobe version n1.0';;\n*) echo '{\"streams\":[{},{}]}';;\nesac\n";
    let mut files = BTreeMap::new();
    for (name, script) in [("ffmpeg", ffmpeg.as_str()), ("ffprobe", ffprobe)] {
        let path = directory.join(name);
        tokio::fs::write(&path, script).await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        files.insert(
            name.into(),
            Sha256::digest(script.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        );
    }
    let candidate = super::Candidate {
        id: "fixture-ffmpeg".into(),
        tool: "ffmpeg".into(),
        channel: "stable".into(),
        version: "1.0".into(),
        platform: "linux-x86_64".into(),
        notes_url: String::new(),
        source: String::new(),
        license: String::new(),
        artifacts: vec![],
        python_abi: None,
    };
    let generation = super::Generation {
        id: generation_id.into(),
        candidate: candidate.clone(),
        files,
        executables: BTreeMap::from([
            ("ffmpeg".into(), "ffmpeg".into()),
            ("ffprobe".into(), "ffprobe".into()),
        ]),
        python_abi: String::new(),
    };
    let tool = super::storage::inventory(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|tool| tool.id == "ffmpeg")
        .unwrap();
    let id = super::storage::enqueue(
        &state.db,
        tool,
        "bootstrap".into(),
        Some(candidate),
        true,
        None,
    )
    .await
    .unwrap();
    super::storage::prepared(&state.db, id.clone(), generation)
        .await
        .unwrap();
    let job = super::storage::jobs(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    let worker_state = state.clone();
    let mut worker = tokio::spawn(async move { super::operation(&worker_state, &job).await });
    tokio::time::timeout(Duration::from_secs(20), async {
        while !entered.exists() {
            // An operation that fails first reports why instead of timing out.
            tokio::select! {
                result = &mut worker => panic!(
                    "The operation ended before verification started: {result:?}"
                ),
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    })
    .await
    .expect("Package verification must start");
    let running = call(&state, "/api/v1/me/attention", "GET", Value::Null, &cookie).await;
    let inventory = call(&state, "/api/v1/admin/tools", "GET", Value::Null, &cookie).await;
    tokio::fs::write(&resume, "resume").await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let complete = call(&state, "/api/v1/me/attention", "GET", Value::Null, &cookie).await;
    eprintln!(
        "verification activity: {}; completed activity: {}",
        running.2, complete.2
    );
    assert_eq!(running.0, StatusCode::OK);
    assert_eq!(inventory.0, StatusCode::OK);
    let tool_activity = running.2["activity"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["resource"] == "ffmpeg")
        .unwrap();
    assert_eq!(
        tool_activity["state"], "running",
        "Verification is active work: {}",
        running.2
    );
    assert_eq!(
        inventory.2["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "ffmpeg")
            .unwrap()["job"]["stage"],
        "verifying"
    );
    assert!(
        complete.2["activity"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["resource"] != "ffmpeg")
    );
}

#[tokio::test]
async fn twitch_connection_and_followed_channels_do_not_wait_for_tool_installation() {
    let (_temp, state, cookie, server) = crate::online::twitch_startup_fixture().await;
    state.tools.ready.send_replace(false);
    let response = call(
        &state,
        "/api/v1/online/twitch/connect",
        "POST",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let started = Instant::now();
    let worker_state = state.clone();
    let worker = tokio::spawn(crate::online::run(worker_state));
    let connected = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let account = call(&state, "/api/v1/online/twitch", "GET", Value::Null, &cookie)
                .await
                .2;
            let feed = call(
                &state,
                "/api/v1/online/twitch/feed",
                "GET",
                Value::Null,
                &cookie,
            )
            .await
            .2;
            if account["account"]["status"] == "connected"
                && account["sync"]["in_progress"] == false
                && feed["items"].as_array().is_some_and(|items| items.len() == 2)
            {
                eprintln!(
                    "Twitch connected and both followed channels loaded in {:.2}s with tool installation still pending",
                    started.elapsed().as_secs_f64()
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    worker.abort();
    server.abort();
    assert!(!*state.tools.ready.borrow());
    connected.expect(
        "Twitch approval and followed channels must load while playback tools are still installing",
    );
}

#[tokio::test]
async fn playback_reports_preparing_tools_to_viewers_and_stops_waiting_after_failure() {
    let (_temp, state, cookie, server) = crate::online::twitch_startup_fixture().await;
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES ('alice','42','fixture','Fixture','Live fixture','Science',10,'today','s',1)", [])?;
        db.execute("INSERT INTO tool_jobs(id,tool,action,manual,revision,stage,created_at,updated_at) VALUES ('install-streamlink','streamlink','bootstrap',0,0,'queued',1,1)", [])?;
        Ok(())
    }).await.unwrap();
    let metadata = call(
        &state,
        "/api/v1/catalog/twitch:42/playback",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(
        metadata.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        metadata.2
    );
    assert_eq!(metadata.2["error"]["code"], "tools_preparing");
    state.db.write("test.fixture", |db| {
        db.execute("UPDATE tool_jobs SET stage='failed',error='Download failed' WHERE id='install-streamlink'", [])?;
        Ok(())
    }).await.unwrap();
    let failed = call(
        &state,
        "/api/v1/catalog/twitch:42/playback",
        "GET",
        Value::Null,
        &cookie,
    )
    .await;
    assert_eq!(failed.0, StatusCode::CONFLICT, "{}", failed.2);
    assert_ne!(failed.2["error"]["code"], "tools_preparing");
    assert!(
        failed.2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Settings")
    );
    server.abort();
}
