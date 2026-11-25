#[path = "../../../tests/helpers/auth-session.rs"]
mod auth_session;
#[path = "../../../tests/helpers/server-config.rs"]
mod configuration;
#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
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
    let config = configuration::config(temp.path());
    let state = AppState::open(config.clone()).await.unwrap();
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    state.db.write("test.fixture", move|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Movies','movies',?1)",[root.to_string_lossy().to_string()])?;
        db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','viewer','unused','user',?1)",[now()])?;
        db.execute("INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen) VALUES ('device','user','unused','device','test',?1,?2,?1)",rusqlite::params![now(),now()+3600])?;Ok(())
    }).await.unwrap();
    let root = thelxinoe_catalog::roots(&state.db).await.unwrap().remove(0);
    thelxinoe_catalog::scan(
        &state.db,
        root.clone(),
        state.tools.runtime.media().unwrap(),
    )
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
    let (media_id,file_id,generation)=state.db.write("test.fixture", |db|Ok(db.query_row("SELECT m.id,f.id,f.generation FROM media m JOIN media_sources s ON s.media_id=m.id JOIN media_files f ON f.id=s.file_id",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?)).await.unwrap();
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities {
            containers: vec!["mp4".into()],
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            hls: true,
            native_tracks: false,
            native_remote: false,
            conversion: None,
            ..Default::default()
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
            orientation: None,
        },
    )
    .await
    .unwrap();
    report(
        &state,
        &principal,
        active["id"].as_str().unwrap(),
        Progress {
            active_seconds: None,
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
    thelxinoe_catalog::scan(&state.db, root, state.tools.runtime.media().unwrap())
        .await
        .unwrap();
    let fid = file_id.clone();
    let new_generation = state
        .db
        .write("test.fixture", move |db| {
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
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    let resumed = create_for(
        &state,
        &principal,
        Create {
            media_id: media_id.clone(),
            file_id: Some(file_id),
            position: None,
            options,
            orientation: None,
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
                active_seconds: None,
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
                active_seconds: None,
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
        .write("test.fixture", move |db| {
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

async fn send(state: &AppState, path: &str, method: &str, body: Value, token: &str) -> Value {
    let mut request = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "media.test")
        .header("content-type", "application/json")
        .header("x-thelxinoe-client", "1")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:8000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(status, StatusCode::OK, "{path}: {body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    // The body is the canonical serialization: sorted keys, floats kept as floats.
    assert_eq!(body, value.to_string());
    value
}
/// `session` with the values that differ between runs replaced by placeholders.
fn stable(session: &Value) -> Value {
    let mut session = session.clone();
    let id = session["id"].as_str().unwrap().to_owned();
    let grant = session["grant"].as_str().unwrap().to_owned();
    let mut text = session
        .to_string()
        .replace(&id, "<id>")
        .replace(&grant, "<grant>");
    if let Some(revision) = session["url"]
        .as_str()
        .and_then(|url| url.split("/hls/").nth(1))
        .and_then(|rest| rest.split('/').next())
    {
        text = text.replace(&format!("/hls/{revision}/"), "/hls/<revision>/");
    }
    session = serde_json::from_str(&text).unwrap();
    if let Some(object) = session.as_object_mut() {
        for key in ["probe", "file_id", "generation"] {
            if let Some(value) = object.get_mut(key) {
                assert!(!value.is_null(), "{key}");
                *value = json!(format!("<{key}>"));
            }
        }
        if let Some(duration) = object.get_mut("duration") {
            assert!(duration.is_f64() && duration.as_f64().unwrap() > 17.0);
            *duration = json!("<duration>");
        }
    }
    session
}
/// The web API's playback session for each playback mode, as the web app reads it.
#[tokio::test]
async fn web_playback_session_shape_for_each_playback_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("media");
    std::fs::create_dir(&root).unwrap();
    media(&root.join("Shape (2020).mp4"), "440");
    std::fs::write(
        root.join("Shape (2020).en.srt"),
        "1
00:00:01,000 --> 00:00:02,000
Hello
",
    )
    .unwrap();
    let state = AppState::open(configuration::config(temp.path()))
        .await
        .unwrap();
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    state.db.write("test.fixture", move|db|{
        db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Movies','movies',?1)",[root.to_string_lossy().to_string()])?;
        db.execute("INSERT INTO users(id,username,password_hash,role,created_at) VALUES ('user','viewer','unused','user',?1)",[now()])?;Ok(())
    }).await.unwrap();
    let root = thelxinoe_catalog::roots(&state.db).await.unwrap().remove(0);
    thelxinoe_catalog::scan(&state.db, root, state.tools.runtime.media().unwrap())
        .await
        .unwrap();
    let token =
        auth_session::issue_session(&state.db, "user".into(), "device".into(), "test".into())
            .await
            .unwrap();
    let media_id = state
        .db
        .write("test.fixture", |db| {
            Ok(db.query_row("SELECT id FROM media", [], |r| r.get::<_, String>(0))?)
        })
        .await
        .unwrap();
    let start = async |position: Value, capabilities: Value| {
        let input = json!({"media_id":media_id,"position":position,"options":{"quality":"auto","capabilities":capabilities}});
        send(&state, "/api/v1/playback", "POST", input, &token).await
    };
    let sidecar = "sidecar-5368617065202832303230292e656e2e737274";
    let tracks = json!([
        {"codec":"aac","default":true,"forced":false,"id":"embedded-1","index":1,"kind":"audio","language":"und","supported":true,"title":""},
        {"codec":"srt","default":false,"forced":false,"id":sidecar,"index":null,"kind":"subtitle","language":"en","supported":true,"title":"Shape (2020).en.srt"}
    ]);
    let subtitles = json!([{"codec":"srt","forced":false,"id":sidecar,"language":"en","title":"Shape (2020).en.srt","url":format!("/api/v1/playback/<id>/subtitles/{sidecar}?grant=<grant>")}]);
    let capabilities = |containers: Value, video: Value, extra: Value| {
        let mut all = json!({"allow_conversion":null,"audio":["aac"],"burn_subtitles":false,"containers":containers,"conversion":null,"hls":true,"hls_audio":null,"hls_container":null,"hls_media":null,"hls_video":null,"native_remote":false,"native_tracks":false,"strip_dolby_vision":false,"supported_media":null,"video":video});
        for (key, value) in extra.as_object().unwrap() {
            all[key] = value.clone();
        }
        all
    };
    let expected = |mode: &str,
                    encoding: Value,
                    reason: Value,
                    position: Value,
                    url: &str,
                    capabilities: Value| {
        json!({"chapters":[],"conversion_reason":reason,"duration":"<duration>","encoding":encoding,"file_id":"<file_id>","fonts":[],"generation":"<generation>","grant":"<grant>","id":"<id>","mode":mode,"options":{"audio":null,"capabilities":capabilities,"quality":"auto","subtitle":null},"position":position,"probe":"<probe>","replay_gain":"track","selected_subtitle":null,"subtitles":subtitles,"timeline_start":0.0,"tracks":tracks,"trickplay":null,"url":url,"video":true}).to_string()
    };
    let copied = json!({"audio":false,"video":false});
    let hls = "/api/v1/playback/<id>/hls/<revision>/index.m3u8?grant=<grant>";

    let direct = capabilities(json!(["mp4"]), json!(["h264"]), json!({}));
    let session = start(Value::Null, direct.clone()).await;
    assert_eq!(
        stable(&session).to_string(),
        expected(
            "direct",
            copied.clone(),
            Value::Null,
            json!(0.0),
            "/api/v1/playback/<id>/stream?grant=<grant>",
            direct
        )
    );

    let remux = capabilities(json!([]), json!(["h264"]), json!({}));
    let session = start(json!(3), remux.clone()).await;
    assert_eq!(
        stable(&session).to_string(),
        expected("remux", copied, Value::Null, json!(3.0), hls, remux)
    );
    let id = session["id"].as_str().unwrap();
    let seek = send(
        &state,
        &format!("/api/v1/playback/{id}/seek"),
        "POST",
        json!({"position":6}),
        &token,
    )
    .await;
    let grant = seek["url"]
        .as_str()
        .unwrap()
        .split("grant=")
        .nth(1)
        .unwrap();
    assert_eq!(
        stable(&json!({"id":id,"grant":grant,"url":seek["url"],"position":seek["position"],"timeline_start":seek["timeline_start"]})).to_string(),
        json!({"grant":"<grant>","id":"<id>","position":6.0,"timeline_start":0.0,"url":hls}).to_string()
    );
    assert_eq!(seek.as_object().unwrap().len(), 3);
    send(
        &state,
        &format!("/api/v1/playback/{id}"),
        "DELETE",
        json!({}),
        &token,
    )
    .await;

    let transcode = capabilities(
        json!([]),
        json!([]),
        json!({"allow_conversion":true,"hls_container":"mp4","conversion":{"deinterlace":false,"fit":false,"frame_rate":24.0,"height":90,"level":41,"profile":"high","width":160}}),
    );
    let session = start(json!(0), transcode.clone()).await;
    assert_eq!(
        stable(&session).to_string(),
        expected(
            "transcode",
            json!({"audio":false,"video":true}),
            json!("Video format is unsupported by this client"),
            json!(0.0),
            hls,
            transcode
        )
    );
    let id = session["id"].as_str().unwrap();
    send(
        &state,
        &format!("/api/v1/playback/{id}"),
        "DELETE",
        json!({}),
        &token,
    )
    .await;
}
