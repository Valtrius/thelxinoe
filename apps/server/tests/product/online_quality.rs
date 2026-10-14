use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    ssl::{SslAcceptor, SslMethod},
    x509::{X509, X509NameBuilder},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::Arc,
};
use tower::ServiceExt;

async fn api(
    state: &AppState,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let response = crate::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .method(if body.is_some() { "POST" } else { "GET" })
                .header("host", "media.test")
                .header("x-emby-token", token)
                .header("content-type", "application/json")
                .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
fn run(args: &[&str]) -> Vec<u8> {
    let mut command = Command::new("ffmpeg");
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn fixture_media(
    directory: &Path,
    name: &str,
    size: &str,
    rate: &str,
    codec: &str,
) -> std::path::PathBuf {
    let path = directory.join(name);
    run(&[
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        &format!(
            "color=c=blue:s={size}:r={rate},drawbox=x=10:y=10:w=96:h=96:color=white:t=fill:enable='gte(t,2)'"
        ),
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "3",
        "-c:v",
        codec,
        "-threads",
        "2",
        "-g",
        rate,
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-movflags",
        "+faststart",
        path.to_str().unwrap(),
    ]);
    path
}
fn proxy(
    files: std::collections::HashMap<String, Vec<u8>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "r1.googlevideo.com")
        .unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    let mut tls = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    tls.set_private_key(&key).unwrap();
    tls.set_certificate(&cert.build()).unwrap();
    let tls = Arc::new(tls.build());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let files = Arc::new(files);
    let task = tokio::spawn(async move {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let tls = tls.clone();
                    let files = files.clone();
                    tokio::task::spawn_blocking(move || {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .unwrap();
                        fn head(input: &mut impl Read) -> std::io::Result<String> {
                            let mut bytes = Vec::new();
                            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 8192 {
                                let mut b = [0];
                                input.read_exact(&mut b)?;
                                bytes.push(b[0]);
                            }
                            Ok(String::from_utf8_lossy(&bytes).into_owned())
                        }
                        if head(&mut stream).is_err() {
                            return;
                        }
                        let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
                        let Ok(mut stream) = tls.accept(stream) else {
                            return;
                        };
                        let Ok(request) = head(&mut stream) else {
                            return;
                        };
                        let path = request
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .trim_start_matches('/');
                        let Some(bytes) = files.get(path) else { return };
                        let start = request
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("Range: bytes=")
                                    .and_then(|s| s.split('-').next()?.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if start >= bytes.len() {
                            return;
                        };
                        let response = format!(
                            "HTTP/1.1 {}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            if start > 0 {
                                "206 Partial Content"
                            } else {
                                "200 OK"
                            },
                            bytes.len() - start,
                            start,
                            bytes.len() - 1,
                            bytes.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(&bytes[start..]);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(_) => break,
            }
        }
    });
    (address, task)
}
fn hashes(path: &Path, track: &str) -> Vec<String> {
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-select_streams",
            track.trim_start_matches("0:"),
            "-show_packets",
            "-show_entries",
            "packet=data_hash",
            "-show_data_hash",
            "sha256",
            "-of",
            "json",
        ])
        .arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    value["packets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["data_hash"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn jellyfin_provider_qualities_preserve_8k_and_60fps_packets_and_enforce_client_limits() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thelxinoe_playback=debug,thelxinoe_server=info")
        .with_test_writer()
        .try_init();
    let (temp, state, _) = crate::test_support::fixture().await;
    tool_runtime::install(&state.tools.runtime, &temp.path().join("state/tools"));
    let qhd = fixture_media(temp.path(), "qhd.mp4", "2560x1440", "60", "libx264");
    let eight = fixture_media(temp.path(), "8k.mp4", "7680x4320", "2", "libvpx-vp9");
    let (proxy, task) = proxy(std::collections::HashMap::from([
        ("qhd.mp4".into(), std::fs::read(&qhd).unwrap()),
        ("8k.mp4".into(), std::fs::read(&eight).unwrap()),
    ]));
    let video = "quality0001";
    state.db.write("test.fixture",move|db|{db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES ('alice','quality-channel','Quality channel','s',1)",[])?;db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration) VALUES ('alice',?1,'quality-channel','Quality fixture','public',3)",[video])?;Ok(())}).await.unwrap();
    let metadata = json!({"duration":3,"is_live":false,"formats":[
        {"format_id":"qhd","url":"https://r1.googlevideo.com/qhd.mp4","height":1440,"width":2560,"fps":60,"vcodec":"avc1.640033","acodec":"mp4a.40.2","protocol":"https","ext":"mp4","tbr":10000},
        {"format_id":"eight","url":"https://r1.googlevideo.com/8k.mp4","height":4320,"width":7680,"fps":2,"vcodec":"vp09.00.60.08","acodec":"mp4a.40.2","protocol":"https","ext":"mp4","tbr":20000}
    ]});
    let mut prepared = select(&metadata, Some(proxy)).unwrap();
    prepared.probe = prepared
        .source
        .probe(&state.tools.runtime.media().unwrap())
        .await
        .unwrap();
    state
        .online
        .streams
        .prepared
        .lock()
        .await
        .insert(video.into(), prepared);
    let token = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "jellyfin".into(),
        "Quality TV".into(),
    )
    .await
    .unwrap();
    let (status, bytes) = api(
        &state,
        &token,
        "/Items?ParentId=d7df0f21-caa6-45a0-9798-267150600001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items: Value = serde_json::from_slice(&bytes).unwrap();
    let item = items["Items"][0]["Id"].as_str().unwrap();
    let path = format!("/Items/{item}/PlaybackInfo");
    let (status, bytes) = api(&state, &token, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let sources: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("googlevideo"));
    let sources = sources["MediaSources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["MediaStreams"][0]["Height"], 4320);
    let qhd_id = sources
        .iter()
        .find(|s| s["MediaStreams"][0]["Height"] == 1440)
        .unwrap()["Id"]
        .clone();
    let profile = json!({"TranscodingProfiles":[{"Type":"Video","Protocol":"hls","Container":"mp4","VideoCodec":"h264,vp9","AudioCodec":"aac"}]});
    let mut evidence = Vec::new();
    for (selected, original) in [(Value::Null, &eight), (qhd_id.clone(), &qhd)] {
        let (status,bytes)=api(&state,&token,&path,Some(json!({"MediaSourceId":selected,"DeviceProfile":profile,"EnableTranscoding":false,"EnableDirectStream":true}))).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let session: Value = serde_json::from_slice(&bytes).unwrap();
        let sid = session["PlaySessionId"].as_str().unwrap();
        let mode: String = state
            .db
            .read("test.proof", {
                let sid = sid.to_owned();
                move |db| {
                    Ok(db.query_row(
                        "SELECT mode FROM playback_sessions WHERE id=?1",
                        [sid],
                        |r| r.get(0),
                    )?)
                }
            })
            .await
            .unwrap();
        assert_eq!(mode, "remux");
        let url = session["MediaSources"][0]["TranscodingUrl"]
            .as_str()
            .unwrap();
        let (status, bytes) = api(&state, &token, url, None).await;
        assert_eq!(status, StatusCode::OK);
        let playlist = String::from_utf8(bytes).unwrap();
        assert!(playlist.contains("#EXT-X-ENDLIST"));
        let parent = url.split('?').next().unwrap().rsplit_once('/').unwrap().0;
        let query = url.split_once('?').unwrap().1;
        let last = playlist
            .lines()
            .rfind(|l| !l.starts_with('#') && !l.is_empty())
            .unwrap();
        let (status, bytes) = api(&state, &token, &format!("{parent}/{last}"), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let mut init_bytes = Vec::new();
        let mut video_packets = Vec::new();
        let mut audio_packets = Vec::new();
        let output = temp.path().join("delivered.mp4");
        for line in playlist.lines() {
            if line.starts_with("#EXT-X-MAP") {
                let name = line.split('"').nth(1).unwrap();
                let (status, bytes) = api(
                    &state,
                    &token,
                    &format!(
                        "{parent}/{name}{}",
                        if name.contains('?') {
                            String::new()
                        } else {
                            format!("?{query}")
                        }
                    ),
                    None,
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                init_bytes = bytes;
            } else if !line.starts_with('#') && !line.is_empty() {
                let (status, bytes) = api(
                    &state,
                    &token,
                    &format!(
                        "{parent}/{line}{}",
                        if line.contains('?') {
                            String::new()
                        } else {
                            format!("?{query}")
                        }
                    ),
                    None,
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                let mut combined = init_bytes.clone();
                combined.extend(bytes);
                std::fs::write(&output, combined).unwrap();
                video_packets.extend(hashes(&output, "0:v:0"));
                audio_packets.extend(hashes(&output, "0:a:0"));
            }
        }
        let original_video = hashes(original, "0:v:0");
        let original_audio = hashes(original, "0:a:0");
        assert_eq!(video_packets, original_video);
        assert_eq!(audio_packets, original_audio);
        evidence.push(json!({"source":session["MediaSources"][0],"mode":mode,"video_packets":video_packets,"original_packets":original_video,"audio_packets":audio_packets,"original_audio_packets":original_audio,"late_segment_first":true}));
        let _ = api(
            &state,
            &token,
            "/Sessions/Playing/Stopped",
            Some(json!({"ItemId":item,"PlaySessionId":sid,"PositionTicks":0})),
        )
        .await;
    }
    let mut limited = profile.clone();
    limited["CodecProfiles"] = json!([{"Type":"Video","Codec":"h264,vp9","Conditions":[{"Property":"Height","Condition":"LessThanEqual","Value":"1440","IsRequired":true}]}]);
    let (status, bytes) = api(
        &state,
        &token,
        &path,
        Some(json!({"DeviceProfile":limited,"EnableTranscoding":false})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let constrained: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(constrained["MediaSources"][0]["Id"], qhd_id);
    let (status, _) = api(
        &state,
        &token,
        &path,
        Some(json!({"MediaSourceId":"https://other.test/video","DeviceProfile":profile})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    task.abort();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/provider-quality");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","playbacks":evidence,"client_limit":constrained["MediaSources"][0]["MediaStreams"]})).unwrap()).unwrap();
}
