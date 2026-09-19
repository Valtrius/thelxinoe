use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use serde_json::json;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    path::Path,
    process::{Command, Stdio},
};
use thelxinoe_core::{Principal, Role, User, now};
use thelxinoe_playback::{Capabilities, Options};
use thelxinoe_server::{
    AppState,
    config::Config,
    playback::{Create, Progress, create_for, report},
    router,
};
use tower::ServiceExt;

fn media(path: &Path, frequency: &str) {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=s=160x90:r=24",
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency={frequency}:sample_rate=48000"),
        "-t",
        "18",
        "-c:v",
        "libx264",
        "-c:a",
        "aac",
        "-movflags",
        "+faststart",
    ])
    .arg(path)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let result = cmd.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[tokio::test]
async fn replacement_and_restart_keep_resume_but_invalidate_old_playback_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("media");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("Resume (2020).mp4");
    media(&path, "440");
    let config = Config {
        state: temp.path().join("state"),
        cache: temp.path().join("cache"),
        web: temp.path().join("web"),
        media: root.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec![],
        controller_socket: temp.path().join("socket"),
    };
    let state = AppState::open(config.clone()).await.unwrap();
    state.db.call(move|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Movies','movies',?1)",[root.to_string_lossy().to_string()])?;
        db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','viewer','unused','user',?1)",[now()])?;
        db.execute("INSERT INTO sessions VALUES ('device','user','unused','device','test',?1,?2,?1)",rusqlite::params![now(),now()+3600])?;Ok(())
    }).await.unwrap();
    let root = thelxinoe_catalog::roots(&state.db).await.unwrap().remove(0);
    thelxinoe_catalog::scan(&state.db, root.clone())
        .await
        .unwrap();
    let principal = Principal {
        user: User {
            id: "user".into(),
            username: "viewer".into(),
            role: Role::User,
            timezone: "UTC".into(),
        },
        session_id: "device".into(),
        transport: "device".into(),
    };
    let (media_id,file_id,generation)=state.db.call(|db|Ok(db.query_row("SELECT m.id,f.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?)).await.unwrap();
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities {
            containers: vec!["mp4".into()],
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            hls: true,
        },
    };
    let active = create_for(
        &state,
        &principal,
        Create {
            media_id: media_id.clone(),
            file_id: None,
            position: None,
            options: options.clone(),
        },
    )
    .await
    .unwrap();
    report(
        &state,
        &principal,
        active["id"].as_str().unwrap(),
        Progress {
            sequence: 0,
            position: 7.0,
            state: "paused".into(),
        },
    )
    .await
    .unwrap();
    media(&path, "880");
    let mut request = Request::builder()
        .uri(active["url"].as_str().unwrap())
        .header("host", "media.test")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:8000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    thelxinoe_catalog::scan(&state.db, root).await.unwrap();
    let fid = file_id.clone();
    let new_generation = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT generation FROM media_files WHERE id=?1",
                [fid],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap();
    assert_ne!(new_generation, generation);
    drop(state);
    let state = AppState::open(config).await.unwrap();
    let resumed = create_for(
        &state,
        &principal,
        Create {
            media_id: media_id.clone(),
            file_id: Some(file_id),
            position: None,
            options,
        },
    )
    .await
    .unwrap();
    assert_eq!(resumed["position"], json!(7.0));
    let mut request = Request::builder()
        .uri(active["url"].as_str().unwrap())
        .header("host", "media.test")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:8000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        router(state.clone())
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        report(
            &state,
            &principal,
            resumed["id"].as_str().unwrap(),
            Progress {
                sequence: 0,
                position: 9.0,
                state: "stopped".into()
            }
        )
        .await
        .unwrap()["accepted"],
        true
    );
    assert_eq!(
        report(
            &state,
            &principal,
            resumed["id"].as_str().unwrap(),
            Progress {
                sequence: 1,
                position: 1.0,
                state: "playing".into()
            }
        )
        .await
        .unwrap()["accepted"],
        false
    );
    let progress = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT position FROM edition_progress WHERE media_id=?1",
                [media_id],
                |r| r.get::<_, f64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(progress, 9.0);
}
