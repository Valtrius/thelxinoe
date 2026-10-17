use super::compatibility_tests::{assemble, fixture_audio, identical, resolve, without_index};
use super::*;
use crate::test_support::{call, fixture};
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
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
async fn request(
    state: &AppState,
    url: &str,
    range: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::builder().uri(url).header("host", "internal:8484");
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    let mut request = request.body(Body::empty()).unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = crate::router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}
async fn delivered(state: &AppState, url: &str) -> Vec<u8> {
    let (status, _, bytes) = request(state, url, None).await;
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
async fn frames(path: &Path) -> Vec<String> {
    hashes(
        &command(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-i",
                path.to_str().unwrap(),
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
    )
}
/// The fixture CDN's upstream requests since the previous call.
async fn upstream(proxy: &str) -> Vec<Value> {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).unwrap())
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
        .post("https://fixture.googlevideo.com/log")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
fn insert(state: &AppState, metadata: &Value, proxy: &str) -> Prepared {
    let prepared = select(metadata, Some(proxy.into())).unwrap();
    state
        .online
        .streams
        .prepared
        .try_lock()
        .unwrap()
        .insert("abcdefghijk".into(), prepared.clone());
    prepared
}

#[tokio::test]
async fn recorded_videos_play_original_files_and_conversion_follows_server_policy() {
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
        let source = format!("testsrc2=size={size}:rate=24");
        // Separate video files shaped like YouTube's, with two-second fragments.
        // Like YouTube's, they present their first frame at the index's
        // earliest time, which FFmpeg's index would not offset by B-frame delay.
        command(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &source,
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-g",
                "48",
                "-bf",
                "0",
                "-an",
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof+global_sidx",
                "-f",
                "mp4",
                root.join(format!("{height}.mp4")).to_str().unwrap(),
            ],
        )
        .await;
        // Live streams arrive as HLS playlists.
        let muxed = root.join(format!("{height}-live.mp4"));
        command(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &source,
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
                muxed.to_str().unwrap(),
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
                muxed.to_str().unwrap(),
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
    }
    let audio = fixture_audio(root, "8");
    let big = (0..12 * 1024 * 1024 + 123)
        .map(|i: u32| (i * 7 % 251) as u8)
        .collect::<Vec<_>>();
    tokio::fs::write(root.join("big.mp4"), &big).await.unwrap();
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
    let proxy = proxy["proxy"].as_str().unwrap();
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
    let browser = |video: Value, audio: Value| json!({"containers":[],"video":video,"audio":audio,"hls":true,"allow_conversion":true});
    let delete = async |session: &Value| {
        let path = format!("/api/v1/playback/{}", session["id"].as_str().unwrap());
        assert_eq!(
            call(&state, &path, "DELETE", json!({}), &cookie).await.0,
            StatusCode::OK
        );
    };

    // Live streams remux the provider's playlist without encoding.
    let mut live = insert(
        &state,
        &json!({"is_live":true,"duration":8.0,"formats":[
            {"vcodec":"h264","acodec":"aac","height":90,"fps":24,"protocol":"m3u8_native","ext":"mp4","url":"https://fixture.googlevideo.com/90.m3u8"},
            {"vcodec":"h264","acodec":"aac","height":180,"fps":24,"protocol":"m3u8_native","ext":"mp4","url":"https://fixture.googlevideo.com/180.m3u8"}
        ]}),
        proxy,
    );
    live.probe = live
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
        .insert("abcdefghijk".into(), live);
    for height in [180, 90] {
        let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":format!("{height}p"),"capabilities":browser(json!(["h264"]),json!(["aac"]))}});
        let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
        assert_eq!(response.0, StatusCode::OK, "{height}: {}", response.2);
        let session = response.2;
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
        let copied = frames(&output).await;
        let original = frames(&root.join(format!("{height}-live.mp4"))).await;
        assert!(copied.len() >= 24);
        let first = original.iter().position(|hash| hash == &copied[0]).unwrap();
        assert_eq!(copied, original[first..first + copied.len()]);
        evidence.push(json!({"live":true,"resolution":height,"copied_frames":copied.len()}));
        delete(&session).await;
    }

    // Recorded videos: browsers receive the original files as byte-range HLS.
    let recorded = json!({"duration":8.0,"formats":[
        {"format_id":"140","vcodec":"none","acodec":"mp4a.40.2","abr":128,"protocol":"https","ext":"m4a","url":"https://fixture.googlevideo.com/audio.m4a"},
        {"vcodec":"avc1.64000C","acodec":"none","height":90,"width":160,"fps":24,"protocol":"https","ext":"mp4","url":"https://fixture.googlevideo.com/90.mp4"},
        {"vcodec":"avc1.64000D","acodec":"none","height":180,"width":320,"fps":24,"protocol":"https","ext":"mp4","url":"https://fixture.googlevideo.com/180.mp4"}
    ]});
    insert(&state, &recorded, proxy);
    let get = async |path: &str, range: Option<&str>| {
        let (status, _, bytes) = request(&state, path, range).await;
        (status, bytes)
    };
    for height in [180, 90] {
        let input = json!({"media_id":"youtube:abcdefghijk","position":3,"options":{"quality":format!("{height}p"),"capabilities":browser(json!(["h264"]),json!(["aac"]))}});
        let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
        assert_eq!(response.0, StatusCode::OK, "{height}: {}", response.2);
        let session = response.2;
        assert_eq!(session["mode"], "direct");
        assert_eq!(session["encoding"], json!({"video":false,"audio":false}));
        assert_eq!(session["quality"], format!("{height}p"));
        assert_eq!(session["position"], 3.0);
        assert_eq!(session["external_audio"], Value::Null);
        assert_eq!(
            session["qualities"],
            json!([{"value":"180p","label":"180p"},{"value":"90p","label":"90p"}])
        );
        let url = session["url"].as_str().unwrap();
        assert!(url.contains("/hls/source/index.m3u8?grant="));
        let master = String::from_utf8(delivered(&state, url).await).unwrap();
        let audio_uri = master
            .lines()
            .find_map(|l| l.split("URI=\"").nth(1)?.strip_suffix('"'))
            .unwrap();
        let video_uri = master
            .lines()
            .find(|l| !l.starts_with('#') && !l.is_empty())
            .unwrap();
        for (uri, file, count) in [
            (video_uri, root.join(format!("{height}.mp4")), 4),
            (audio_uri, audio.clone(), 8),
        ] {
            let (bytes, fragments) = assemble(&get, &resolve(url, uri)).await;
            identical(&bytes, &without_index(&std::fs::read(file).unwrap()));
            assert!(fragments >= count, "{fragments} fragments");
        }
        // No conversion process runs, and seeking stays within the client.
        assert!(state.playback.runs().await.is_empty());
        let id = session["id"].as_str().unwrap();
        let seek = call(
            &state,
            &format!("/api/v1/playback/{id}/seek"),
            "POST",
            json!({"position":6}),
            &cookie,
        )
        .await;
        assert_eq!(seek.0, StatusCode::OK);
        assert_eq!(seek.2["timeline_start"], 0.0);
        assert_eq!(
            seek.2["url"].as_str().unwrap().split('?').next(),
            url.split('?').next()
        );
        evidence.push(json!({"live":false,"resolution":height,"delivery":"byte-range HLS","byte_identical":true}));
        delete(&session).await;
    }
    // A browser that plays no original rendition is not given a conversion.
    for (video, audio) in [(json!([]), json!(["aac"])), (json!(["h264"]), json!([]))] {
        let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":"auto","capabilities":browser(video,audio)}});
        let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
        assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.2);
    }
    assert!(state.playback.runs().await.is_empty());

    // Clients without fragmented MP4 HLS remux, seeking at the provider's keyframes.
    let options = |video: Value| -> thelxinoe_playback::Options {
        serde_json::from_value(json!({"quality":"180p","capabilities":{"containers":[],"video":video,"audio":["aac"],"hls":true,"hls_container":"ts","allow_conversion":true}})).unwrap()
    };
    let session = create_with_delivery(
        &state,
        &principal,
        "abcdefghijk",
        options(json!(["h264"])),
        Some(0.0),
        true,
    )
    .await
    .unwrap();
    assert_eq!(session["mode"], "remux");
    assert_eq!(session["encoding"], json!({"video":false,"audio":false}));
    assert_eq!(session["quality"], "180p");
    let url = session["url"].as_str().unwrap();
    let playlist = String::from_utf8(delivered(&state, url).await).unwrap();
    assert_eq!(playlist.matches("#EXTINF:2.000000,").count(), 4);
    let base = url.rsplit_once('/').unwrap().0;
    let segment = playlist
        .lines()
        .rfind(|line| line.starts_with("segment-"))
        .unwrap();
    let output = root.join("remuxed.ts");
    tokio::fs::write(
        &output,
        delivered(&state, &format!("{base}/{segment}")).await,
    )
    .await
    .unwrap();
    let copied = frames(&output).await;
    let original = frames(&root.join("180.mp4")).await;
    assert_eq!(copied.len(), 48);
    assert_eq!(copied, original[original.len() - 48..]);
    evidence.push(json!({"delivery":"remux","hls_container":"ts","copied_frames":copied.len()}));
    delete(&session).await;
    // Disabled conversion is refused rather than replaced.
    let refused = create_with_delivery(
        &state,
        &principal,
        "abcdefghijk",
        options(json!([])),
        Some(0.0),
        true,
    )
    .await;
    assert_eq!(refused.unwrap_err().0, StatusCode::FORBIDDEN);
    assert!(state.playback.runs().await.is_empty());
    // A file without the provider's fragment index is refused, not encoded.
    let mut unindexed = recorded.clone();
    unindexed["formats"][2]["url"] = json!("https://fixture.googlevideo.com/180-live.mp4");
    insert(&state, &unindexed, proxy);
    let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":"180p","capabilities":browser(json!(["h264"]),json!(["aac"]))}});
    let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
    assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.2);
    assert!(state.playback.runs().await.is_empty());

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
    insert(&state, &recorded, proxy);
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
    evidence.push(
        json!({"conversion":session["encoding"],"hls_container":"mp4","decoded_segments":segments}),
    );
    delete(&session).await;

    // Separate-file clients read large files in bounded upstream ranges, which
    // YouTube serves at full speed.
    let mut native = recorded.clone();
    native["formats"][2]["url"] = json!("https://fixture.googlevideo.com/big.mp4");
    insert(&state, &native, proxy);
    let input = json!({"media_id":"youtube:abcdefghijk","position":0,"options":{"quality":"180p","capabilities":{"containers":[],"video":["h264"],"audio":["aac"],"hls":true,"native_remote":true}}});
    let response = call(&state, "/api/v1/playback", "POST", input, &cookie).await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let session = response.2;
    let url = session["url"].as_str().unwrap();
    assert!(url.contains("/remote/video?grant="));
    upstream(proxy).await;
    let total = big.len();
    let (status, headers, body) = request(&state, url, Some("bytes=0-")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 0-{}/{total}", total - 1).as_str()
    );
    assert!(body == big);
    let (status, headers, body) = request(&state, url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_LENGTH], total.to_string().as_str());
    assert!(body == big);
    let (status, _, body) = request(&state, url, Some("bytes=100-199")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, big[100..200]);
    let ranges: Vec<_> = upstream(proxy)
        .await
        .into_iter()
        .filter(|r| r["path"] == "/big.mp4")
        .map(|r| r["range"].as_str().unwrap().to_owned())
        .collect();
    let chunked = [
        "bytes=0-10485759".to_owned(),
        format!("bytes=10485760-{}", total - 1),
    ];
    assert_eq!(
        ranges,
        [&chunked[..], &chunked[..], &["bytes=100-199".into()]].concat()
    );
    evidence.push(json!({"delivery":"relay","upstream_ranges":ranges}));
    delete(&session).await;

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
    tokio::fs::write(evidence_path, serde_json::to_vec_pretty(&json!({"variants":evidence,"disabled_encoding_rejected":true,"unindexed_file_refused":true,"persisted_after_reopen":true,"settings_after_verification_window":true,"sensitive_action_still_requires_verification":true})).unwrap()).await.unwrap();
}
