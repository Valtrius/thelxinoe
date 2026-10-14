use super::*;
use crate::test_support::{call, fixture};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tower::ServiceExt;

async fn command(program: &str, args: &[&str]) -> Vec<u8> {
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
async fn delivered(state: &AppState, url: &str) -> Vec<u8> {
    let mut request = Request::builder()
        .uri(url)
        .header("host", "internal:8484")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = crate::router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    assert_eq!(
        status,
        StatusCode::OK,
        "{url}: {}",
        String::from_utf8_lossy(&bytes)
    );
    bytes
}
fn hashes(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.rsplit(',').next().unwrap().trim().to_owned())
        .collect()
}

#[tokio::test]
async fn server_policy_allows_existing_provider_resolutions_without_encoding() {
    let (temp, state, cookie) = fixture().await;
    let root = temp.path();
    state.db.write("test.fixture", |db| {
        db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
        db.execute("UPDATE sessions SET verified_at=?1 WHERE user_id='alice'", [now()-301])?;
        db.execute("INSERT INTO youtube_videos(user_id,video_id,title) VALUES ('alice','abcdefghijk','Offline provider variants')", [])?;
        Ok(())
    }).await.unwrap();
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    for (height, size) in [(180, "320x180"), (90, "160x90")] {
        let file = root.join(format!("{height}.mp4"));
        command(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size={size}:rate=24"),
                "-f",
                "lavfi",
                "-i",
                "sine=sample_rate=48000",
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-g",
                "48",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
                file.to_str().unwrap(),
            ],
        )
        .await;
        command(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-y",
                "-i",
                file.to_str().unwrap(),
                "-c",
                "copy",
                "-f",
                "hls",
                "-hls_time",
                "2",
                "-hls_list_size",
                "0",
                root.join(format!("{height}.m3u8")).to_str().unwrap(),
            ],
        )
        .await;
        let playlist = tokio::fs::read_to_string(root.join(format!("{height}.m3u8")))
            .await
            .unwrap();
        let blocked = playlist
            .lines()
            .map(|line| {
                if line.is_empty() || line.starts_with('#') {
                    line
                } else {
                    "https://fixture.googlevideo.com/denied.mp4"
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(root.join(format!("blocked-{height}.m3u8")), blocked)
            .await
            .unwrap();
    }
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/helpers/online-cdn.py");
    let mut cdn = Command::new("python3")
        .arg(helper)
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(cdn.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let proxy: Value = serde_json::from_str(&line).unwrap();
    let defaults = call(&state, "/api/v1/admin/settings", "GET", json!({}), &cookie)
        .await
        .2;
    assert_eq!(defaults["video_transcoding"], true);
    assert_eq!(defaults["audio_transcoding"], true);
    let display = call(
        &state,
        "/api/v1/admin/settings",
        "PUT",
        json!({"timezone":"Europe/Paris"}),
        &cookie,
    )
    .await;
    assert_eq!(display.0, StatusCode::OK, "{}", display.2);
    assert_eq!(display.2["timezone"], "Europe/Paris");
    assert_eq!(display.2["video_transcoding"], true);
    assert_eq!(display.2["audio_transcoding"], true);
    let sensitive = call(
        &state,
        "/api/v1/users",
        "POST",
        json!({"username":"verification-check","password":"product test passphrase","role":"user"}),
        &cookie,
    )
    .await;
    assert_eq!(sensitive.0, StatusCode::FORBIDDEN);
    assert_eq!(sensitive.2["error"]["code"], "verification_required");
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/settings",
            "PUT",
            json!({"video_transcoding":false,"audio_transcoding":false}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut evidence = Vec::new();
    let principal = crate::security::principal(
        &state,
        &[(axum::http::header::COOKIE, cookie.parse().unwrap())]
            .into_iter()
            .collect(),
    )
    .await
    .unwrap();
    for (live, blocked_segments) in [(true, false), (false, false), (false, true)] {
        let extension = if live && !blocked_segments {
            "m3u8"
        } else {
            "mp4"
        };
        let mut metadata = json!({"is_live":live,"duration":8.0,"formats":[
            {"vcodec":"h264","acodec":"aac","height":90,"fps":24,"protocol":if live{"m3u8_native"}else{"https"},"ext":"mp4","url":format!("https://fixture.googlevideo.com/90.{extension}")},
            {"vcodec":"h264","acodec":"aac","height":180,"fps":24,"protocol":if live{"m3u8_native"}else{"https"},"ext":"mp4","url":format!("https://fixture.googlevideo.com/180.{extension}")}
        ]});
        if blocked_segments {
            for format in metadata["formats"].as_array_mut().unwrap() {
                format["protocol"] = json!("https");
            }
            for height in [90, 180] {
                metadata["formats"].as_array_mut().unwrap().insert(0, json!({"vcodec":"h264","acodec":"aac","height":height,"fps":24,"protocol":"m3u8_native","ext":"mp4","url":format!("https://fixture.googlevideo.com/blocked-{height}.m3u8")}));
            }
        }
        let mut prepared =
            select(&metadata, Some(proxy["proxy"].as_str().unwrap().into())).unwrap();
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
            .insert("abcdefghijk".into(), prepared.clone());
        for height in [180, 90] {
            let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":format!("{height}p"),"capabilities":{"containers":[],"video":["h264"],"audio":["aac"],"hls":true,"allow_conversion":true}}});
            // The Linux runner's FFmpeg 5.1 supports the seekable delivery used
            // by Jellyfin; continuous VOD uses the product's bundled FFmpeg.
            let session = if live {
                let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
                assert_eq!(
                    response.0,
                    StatusCode::OK,
                    "live={live}, height={height}: {}",
                    response.2
                );
                response.2
            } else {
                crate::playback::create_with_delivery(
                    &state,
                    &principal,
                    serde_json::from_value(input).unwrap(),
                    true,
                )
                .await
                .unwrap()
            };
            assert_eq!(session["mode"], "remux");
            assert_eq!(session["encoding"], json!({"video":false,"audio":false}));
            assert_eq!(session["quality"], format!("{height}p"));
            let url = session["url"].as_str().unwrap();
            let playlist = String::from_utf8(delivered(&state, url).await).unwrap();
            let base = url.rsplit_once('/').unwrap().0;
            let mut bytes = Vec::new();
            if let Some(line) = playlist
                .lines()
                .find(|line| line.starts_with("#EXT-X-MAP:URI="))
            {
                let init = line.split('"').nth(1).unwrap();
                bytes.extend(delivered(&state, &format!("{base}/{init}")).await);
            }
            let segment = playlist
                .lines()
                .find(|line| line.starts_with("segment-"))
                .unwrap();
            bytes.extend(delivered(&state, &format!("{base}/{segment}")).await);
            let output = root.join("delivered.mp4");
            tokio::fs::write(&output, bytes).await.unwrap();
            let probe: Value = serde_json::from_slice(
                &command(
                    "ffprobe",
                    &[
                        "-v",
                        "error",
                        "-show_streams",
                        "-of",
                        "json",
                        output.to_str().unwrap(),
                    ],
                )
                .await,
            )
            .unwrap();
            assert!(
                probe["streams"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|stream| stream["codec_name"] == "h264" && stream["height"] == height)
            );
            let original = hashes(
                &command(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-i",
                        root.join(format!("{height}.mp4")).to_str().unwrap(),
                        "-map",
                        "0:v:0",
                        "-fps_mode",
                        "passthrough",
                        "-f",
                        "framemd5",
                        "-",
                    ],
                )
                .await,
            );
            let frames = hashes(
                &command(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-i",
                        output.to_str().unwrap(),
                        "-map",
                        "0:v:0",
                        "-fps_mode",
                        "passthrough",
                        "-f",
                        "framemd5",
                        "-",
                    ],
                )
                .await,
            );
            assert!(frames.len() >= 24);
            let first = original.iter().position(|hash| hash == &frames[0]).unwrap();
            assert_eq!(frames, original[first..first + frames.len()]);
            evidence.push(json!({"live":live,"blocked_segments":blocked_segments,"resolution":height,"copied_frames":frames.len(),"encoding":session["encoding"]}));
            assert_eq!(
                call(
                    &state,
                    &format!("/api/v1/playback/{}", session["id"].as_str().unwrap()),
                    "DELETE",
                    json!({}),
                    &cookie
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        for (video, audio, expected) in [
            (json!([]), json!(["aac"]), "Video"),
            (json!(["h264"]), json!([]), "Audio"),
        ] {
            let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":"auto","capabilities":{"containers":[],"video":video,"audio":audio,"hls":true,"allow_conversion":true}}});
            let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
            assert_eq!(response.0, StatusCode::FORBIDDEN, "{}", response.2);
            assert!(
                response.2["error"]["message"]
                    .as_str()
                    .unwrap()
                    .starts_with(expected)
            );
        }
        if !live {
            // An unusable provider index must not silently enable a video encoder.
            prepared.source.segments = vec![1.0];
            for format in &mut prepared.formats {
                format.source.segments = prepared.source.segments.clone();
            }
            state
                .online
                .streams
                .prepared
                .lock()
                .await
                .insert("abcdefghijk".into(), prepared);
            let options = serde_json::from_value(json!({"quality":"auto","capabilities":{"containers":[],"video":["h264"],"audio":["aac"],"hls":true,"allow_conversion":true,"conversion":{"width":320,"height":180,"frame_rate":24.0,"profile":"high","level":40}}})).unwrap();
            let result =
                create_with_delivery(&state, &principal, "abcdefghijk", options, Some(0.0), true)
                    .await;
            assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
            assert!(state.playback.runs().await.is_empty());
        }
    }
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/settings",
            "PUT",
            json!({"video_transcoding":true,"audio_transcoding":true}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut prepared = select(&json!({"duration":8.0,"formats":[{"vcodec":"h264","acodec":"aac","height":180,"fps":24,"protocol":"https","ext":"mp4","url":"https://fixture.googlevideo.com/180.mp4"}]}), Some(proxy["proxy"].as_str().unwrap().into())).unwrap();
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
        .insert("abcdefghijk".into(), prepared);
    let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":"180p","capabilities":{"containers":[],"video":[],"audio":["aac"],"hls":true,"hls_container":"mp4","allow_conversion":true,"conversion":{"width":160,"height":90,"frame_rate":24.0,"profile":"high","level":41}}}});
    let session = crate::playback::create_with_delivery(
        &state,
        &principal,
        serde_json::from_value(input).unwrap(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(session["encoding"], json!({"video":true,"audio":false}));
    let url = session["url"].as_str().unwrap();
    let playlist = String::from_utf8(delivered(&state, url).await).unwrap();
    let base = url.rsplit_once('/').unwrap().0;
    let mut init = Vec::new();
    let mut segments = 0;
    for line in playlist.lines() {
        if line.starts_with("#EXT-X-MAP:URI=") {
            init = delivered(
                &state,
                &format!("{base}/{}", line.split('"').nth(1).unwrap()),
            )
            .await;
        } else if line.starts_with("segment-") {
            let mut bytes = init.clone();
            bytes.extend(delivered(&state, &format!("{base}/{line}")).await);
            let output = root.join("converted.mp4");
            tokio::fs::write(&output, bytes).await.unwrap();
            let probe: Value = serde_json::from_slice(
                &command(
                    "ffprobe",
                    &[
                        "-v",
                        "error",
                        "-show_streams",
                        "-of",
                        "json",
                        output.to_str().unwrap(),
                    ],
                )
                .await,
            )
            .unwrap();
            assert!(
                probe["streams"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|stream| stream["codec_name"] == "h264" && stream["height"] == 90)
            );
            command(
                "ffmpeg",
                &[
                    "-v",
                    "error",
                    "-i",
                    output.to_str().unwrap(),
                    "-f",
                    "null",
                    "-",
                ],
            )
            .await;
            segments += 1;
        }
    }
    assert_eq!(segments, 2);
    evidence.push(json!({"conversion":session["encoding"],"hls_container":"mp4","decoded_segments":segments,"prefetched_initializations":true}));
    assert_eq!(
        call(
            &state,
            &format!("/api/v1/playback/{}", session["id"].as_str().unwrap()),
            "DELETE",
            json!({}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "/api/v1/admin/settings",
            "PUT",
            json!({"video_transcoding":false,"audio_transcoding":false}),
            &cookie
        )
        .await
        .0,
        StatusCode::OK
    );
    let reopened = AppState::open(state.config.as_ref().clone()).await.unwrap();
    let persisted = call(
        &reopened,
        "/api/v1/admin/settings",
        "GET",
        json!({}),
        &cookie,
    )
    .await;
    assert_eq!(persisted.0, StatusCode::OK);
    assert_eq!(persisted.2["video_transcoding"], false);
    assert_eq!(persisted.2["audio_transcoding"], false);
    let evidence_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/online-transcoding-policy.json");
    tokio::fs::create_dir_all(evidence_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(evidence_path, serde_json::to_vec_pretty(&json!({"variants":evidence,"disabled_encoding_rejected":true,"fallback_did_not_encode":true,"persisted_after_reopen":true,"settings_after_verification_window":true,"sensitive_action_still_requires_verification":true})).unwrap()).await.unwrap();
}
