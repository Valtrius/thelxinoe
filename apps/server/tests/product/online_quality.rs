use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
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
    range: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .uri(path)
        .method(if body.is_some() { "POST" } else { "GET" })
        .header("host", "media.test")
        .header("x-emby-token", token)
        .header("content-type", "application/json");
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    let response = crate::router(state.clone())
        .oneshot(
            request
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
/// Fragmented MP4 shaped like YouTube's separate files: `ftyp`, `moov`, a
/// `sidx` index, then keyframe-aligned fragments.
const DASH: [&str; 2] = [
    "-movflags",
    "+frag_keyframe+empty_moov+default_base_moof+global_sidx",
];
fn fixture_media(
    directory: &Path,
    name: &str,
    size: &str,
    rate: &str,
    codec: &str,
) -> std::path::PathBuf {
    let path = directory.join(name);
    let input = format!(
        "color=c=blue:s={size}:r={rate},drawbox=x=10:y=10:w=96:h=96:color=white:t=fill:enable='gte(t,2)'"
    );
    let mut args = vec![
        "-v", "error", "-y", "-f", "lavfi", "-i", &input, "-t", "3", "-c:v", codec, "-threads",
        "2", "-g", rate, "-pix_fmt", "yuv420p", "-an",
    ];
    args.extend(DASH);
    args.extend(["-f", "mp4", path.to_str().unwrap()]);
    run(&args);
    path
}
pub(super) fn fixture_audio(directory: &Path, seconds: &str) -> std::path::PathBuf {
    let path = directory.join("audio.m4a");
    run(&[
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        seconds,
        "-c:a",
        "aac",
        "-vn",
        "-frag_duration",
        "1000000",
        "-movflags",
        "+empty_moov+default_base_moof+global_sidx",
        "-f",
        "mp4",
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
                        let range = request.lines().find_map(|line| {
                            let (start, end) = line
                                .to_ascii_lowercase()
                                .strip_prefix("range: bytes=")?
                                .split_once('-')
                                .map(|(a, b)| (a.to_owned(), b.to_owned()))?;
                            Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()))
                        });
                        let (start, end) = range.map_or((0, bytes.len() - 1), |(start, end)| {
                            (start, end.unwrap_or(bytes.len() - 1).min(bytes.len() - 1))
                        });
                        if start >= bytes.len() {
                            return;
                        };
                        let response = format!(
                            "HTTP/1.1 {}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            if range.is_some() {
                                "206 Partial Content"
                            } else {
                                "200 OK"
                            },
                            end + 1 - start,
                            start,
                            end,
                            bytes.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(&bytes[start..=end]);
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
/// A playlist reference as a router path with its query.
pub(super) fn resolve(base: &str, reference: &str) -> String {
    let url = reqwest::Url::parse("http://media.test")
        .unwrap()
        .join(base)
        .unwrap()
        .join(reference)
        .unwrap();
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}
/// The bytes a client assembles from a byte-range media playlist.
pub(super) async fn assemble(
    get: &impl AsyncFn(&str, Option<&str>) -> (StatusCode, Vec<u8>),
    playlist: &str,
) -> (Vec<u8>, usize) {
    let (status, body) = get(playlist, None).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("#EXT-X-PLAYLIST-TYPE:VOD") && text.ends_with("#EXT-X-ENDLIST\n"));
    let mut bytes = Vec::new();
    let mut range = None;
    let mut fragments = 0;
    for line in text.lines() {
        if let Some(init) = line
            .strip_prefix("#EXT-X-MAP:URI=\"")
            .and_then(|s| s.strip_suffix('"'))
        {
            let (status, body) = get(&resolve(playlist, init), None).await;
            assert_eq!(status, StatusCode::OK);
            bytes.extend(body);
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let (size, offset) = value.split_once('@').unwrap();
            let (size, offset) = (size.parse::<u64>().unwrap(), offset.parse::<u64>().unwrap());
            range = Some(format!("bytes={offset}-{}", offset + size - 1));
        } else if !line.starts_with('#') && !line.is_empty() {
            let (status, body) = get(&resolve(playlist, line), range.take().as_deref()).await;
            assert_eq!(status, StatusCode::PARTIAL_CONTENT);
            bytes.extend(body);
            fragments += 1;
        }
    }
    (bytes, fragments)
}
pub(super) fn identical(actual: &[u8], expected: &[u8]) {
    let first = actual.iter().zip(expected).position(|(a, b)| a != b);
    assert!(
        actual == expected,
        "{} and {} bytes differ from {first:?}",
        actual.len(),
        expected.len()
    );
}
/// A fixture file's initialization segment and fragments: what a byte-range
/// client receives. FFmpeg's trailing random-access box is not media.
pub(super) fn without_index(file: &[u8]) -> Vec<u8> {
    let mut offset = 0;
    let mut indexed = false;
    let mut output = Vec::new();
    while offset + 8 <= file.len() {
        let size = u32::from_be_bytes(file[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &file[offset + 4..offset + 8];
        indexed |= kind == b"sidx";
        if !indexed || kind == b"moof" || kind == b"mdat" {
            output.extend_from_slice(&file[offset..offset + size]);
        }
        offset += size;
    }
    output
}

#[tokio::test]
async fn jellyfin_live_playback_describes_tracks_and_delivers_playable_hls() {
    let (temp, state, _) = crate::test_support::fixture().await;
    tool_runtime::install(&state.tools.runtime, &state.config.state.join("tools"));
    let playlist = temp.path().join("live.m3u8");
    run(&[
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=160x90:r=24",
        "-f",
        "lavfi",
        "-i",
        "sine=sample_rate=48000",
        "-t",
        "6",
        "-c:v",
        "libx264",
        "-threads",
        "2",
        "-g",
        "48",
        "-sc_threshold",
        "0",
        "-c:a",
        "aac",
        "-f",
        "hls",
        "-hls_time",
        "2",
        "-hls_list_size",
        "0",
        playlist.to_str().unwrap(),
    ]);
    let files = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            entry.file_type().unwrap().is_file().then(|| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
        })
        .collect();
    let (proxy, task) = proxy(files);
    let mut prepared = select(
        &json!({"is_live":true,"formats":[
            {"url":"https://r1.googlevideo.com/live.m3u8","height":90,"width":160,"fps":24,
             "vcodec":"h264","acodec":"aac","protocol":"m3u8_native","ext":"mp4"}
        ]}),
        Some(proxy),
    )
    .unwrap();
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
        .insert("twitch:123".into(), prepared);
    state.db.write("test.fixture", |db| {
        db.execute("INSERT INTO twitch_streams(user_id,channel_id,login,display_name,title,category,viewers,started_at,snapshot,active) VALUES('alice','123','channel','Live channel','Live','Game',10,'2026-10-21T10:00:00Z','s',1)", [])?;
        Ok(())
    }).await.unwrap();
    let token = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "jellyfin".into(),
        "Live TV".into(),
    )
    .await
    .unwrap();
    let get = async |path: &str, range: Option<&str>| api(&state, &token, path, None, range).await;
    let (status, bytes) = get("/Items?ParentId=d7df0f21-caa6-45a0-9798-267150600002", None).await;
    assert_eq!(status, StatusCode::OK);
    let items: Value = serde_json::from_slice(&bytes).unwrap();
    let item = items["Items"][0]["Id"].as_str().unwrap();
    let path = format!("/Items/{item}/PlaybackInfo");
    for (container, convert) in [("ts", false), ("mp4", false), ("ts", true)] {
        let (status, bytes) = api(&state, &token, &path, Some(json!({
            "MediaSourceId":item,"EnableDirectPlay":true,"EnableDirectStream":!convert,"EnableTranscoding":true,
            "DeviceProfile":{"TranscodingProfiles":[{"Type":"Video","Protocol":"hls","Container":container,"VideoCodec":"h264","AudioCodec":"aac"}]}
        })), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        let source = &response["MediaSources"][0];
        assert_media_streams(source);
        assert_eq!(source["IsInfiniteStream"], true);
        assert_eq!(source["TranscodingContainer"], container);
        assert_eq!(source["SupportsDirectPlay"], false);
        let url = source["TranscodingUrl"].as_str().unwrap();
        let (status, bytes) = get(url, None).await;
        assert_eq!(status, StatusCode::OK);
        let playlist = String::from_utf8(bytes).unwrap();
        assert!(playlist.starts_with("#EXTM3U"), "{playlist}");
        let mut bytes = Vec::new();
        if container == "mp4" {
            let init = playlist
                .lines()
                .find(|line| line.starts_with("#EXT-X-MAP:"))
                .unwrap()
                .split('"')
                .nth(1)
                .unwrap();
            let (status, body) = get(&resolve(url, init), None).await;
            assert_eq!(status, StatusCode::OK);
            bytes.extend(body);
        }
        let segment = playlist
            .lines()
            .find(|line| line.starts_with("segment-"))
            .unwrap();
        let (status, body) = get(&resolve(url, segment), None).await;
        assert_eq!(status, StatusCode::OK);
        bytes.extend(body);
        let file = temp.path().join(format!("delivered.{container}"));
        std::fs::write(&file, bytes).unwrap();
        assert!(!hashes(&file, "v").is_empty());
        assert!(!hashes(&file, "a").is_empty());
        assert_eq!(api(&state, &token, "/Sessions/Playing/Stopped", Some(json!({"ItemId":item,"PlaySessionId":response["PlaySessionId"],"PositionTicks":10_000_000})), None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(get(url, None).await.0, StatusCode::UNAUTHORIZED);
    }
    task.abort();
}

fn assert_media_streams(source: &Value) {
    // Required fields in Jellyfin Kotlin SDK 1.7.1's MediaStream, used by Wholphin.
    // https://github.com/jellyfin/jellyfin-sdk-kotlin/blob/v1.7.1/jellyfin-model/src/commonMain/kotlin-generated/org/jellyfin/sdk/model/api/MediaStream.kt
    let streams = source["MediaStreams"].as_array().unwrap();
    assert_eq!(streams.len(), 2);
    for stream in streams {
        assert!(stream["Type"].is_string());
        assert!(stream["Index"].is_i64());
        for field in [
            "IsInterlaced",
            "IsDefault",
            "IsForced",
            "IsHearingImpaired",
            "IsExternal",
            "IsTextSubtitleStream",
            "SupportsExternalStream",
        ] {
            assert!(
                stream[field].is_boolean(),
                "Missing required {field}: {stream}"
            );
        }
        if stream["Type"] == "Audio" {
            assert_eq!(stream["SampleRate"], 48000);
            assert_eq!(source["DefaultAudioStreamIndex"], stream["Index"]);
        }
    }
}

#[tokio::test]
async fn jellyfin_provider_qualities_preserve_8k_and_60fps_packets_and_enforce_client_limits() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thelxinoe_playback=debug,thelxinoe_server=info")
        .with_test_writer()
        .try_init();
    let (temp, state, _) = crate::test_support::fixture().await;
    let qhd = fixture_media(temp.path(), "qhd.mp4", "2560x1440", "60", "libx264");
    let eight = fixture_media(temp.path(), "8k.mp4", "7680x4320", "2", "libvpx-vp9");
    let audio = fixture_audio(temp.path(), "3");
    let (proxy, task) = proxy(std::collections::HashMap::from([
        ("qhd.mp4".into(), std::fs::read(&qhd).unwrap()),
        ("8k.mp4".into(), std::fs::read(&eight).unwrap()),
        ("audio.m4a".into(), std::fs::read(&audio).unwrap()),
    ]));
    let video = "quality0001";
    state.db.write("test.fixture",move|db|{db.execute("INSERT INTO youtube_subscriptions(user_id,channel_id,title,snapshot,active) VALUES ('alice','quality-channel','Quality channel','s',1)",[])?;db.execute("INSERT INTO youtube_videos(user_id,video_id,channel_id,title,privacy,duration) VALUES ('alice',?1,'quality-channel','Quality fixture','public',3)",[video])?;Ok(())}).await.unwrap();
    let metadata = json!({"duration":3,"is_live":false,"formats":[
        {"format_id":"140","url":"https://r1.googlevideo.com/audio.m4a","vcodec":"none","acodec":"mp4a.40.2","protocol":"https","ext":"m4a","abr":128,"asr":48000,"audio_channels":1},
        {"format_id":"qhd","url":"https://r1.googlevideo.com/qhd.mp4","height":1440,"width":2560,"fps":60,"vcodec":"avc1.640032","acodec":"none","protocol":"https","ext":"mp4","tbr":10000},
        {"format_id":"eight","url":"https://r1.googlevideo.com/8k.mp4","height":4320,"width":7680,"fps":2,"vcodec":"vp09.00.60.08","acodec":"none","protocol":"https","ext":"mp4","tbr":20000}
    ]});
    state
        .online
        .streams
        .prepared
        .lock()
        .await
        .insert(video.into(), select(&metadata, Some(proxy)).unwrap());
    let token = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "jellyfin".into(),
        "Quality TV".into(),
    )
    .await
    .unwrap();
    let get = async |path: &str, range: Option<&str>| api(&state, &token, path, None, range).await;
    let (status, bytes) = get("/Items?ParentId=d7df0f21-caa6-45a0-9798-267150600001", None).await;
    assert_eq!(status, StatusCode::OK);
    let items: Value = serde_json::from_slice(&bytes).unwrap();
    let item = items["Items"][0]["Id"].as_str().unwrap();
    let path = format!("/Items/{item}/PlaybackInfo");
    let (status, bytes) = get(&path, None).await;
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
        let (status,bytes)=api(&state,&token,&path,Some(json!({"MediaSourceId":selected,"DeviceProfile":profile,"EnableTranscoding":false,"EnableDirectStream":true})),None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let session: Value = serde_json::from_slice(&bytes).unwrap();
        assert_media_streams(&session["MediaSources"][0]);
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
        // Fragmented MP4 HLS clients receive the provider's own files.
        assert_eq!(mode, "direct");
        let source = &session["MediaSources"][0];
        assert_eq!(source["TranscodingContainer"], "mp4");
        let url = source["TranscodingUrl"].as_str().unwrap();
        let (status, bytes) = get(url, None).await;
        assert_eq!(status, StatusCode::OK);
        let master = String::from_utf8(bytes).unwrap();
        assert!(master.contains("AUDIO=\"audio\""));
        assert!(master.contains(if original == &eight {
            "CODECS=\"vp09.00.60.08,mp4a.40.2\",RESOLUTION=7680x4320"
        } else {
            "CODECS=\"avc1.640032,mp4a.40.2\",RESOLUTION=2560x1440"
        }));
        let audio_uri = master
            .lines()
            .find_map(|l| l.split("URI=\"").nth(1)?.strip_suffix('"'))
            .unwrap();
        let video_uri = master
            .lines()
            .find(|l| !l.starts_with('#') && !l.is_empty())
            .unwrap();
        let mut packets = json!({});
        for (track, uri, file) in [("v", video_uri, original), ("a", audio_uri, &audio)] {
            let (bytes, fragments) = assemble(&get, &resolve(url, uri)).await;
            // The client receives the original file, byte for byte.
            identical(&bytes, &without_index(&std::fs::read(file).unwrap()));
            assert!(fragments >= 2);
            let output = temp.path().join("delivered.mp4");
            std::fs::write(&output, bytes).unwrap();
            let delivered = hashes(&output, &format!("0:{track}:0"));
            assert_eq!(delivered, hashes(file, &format!("0:{track}:0")));
            packets[track] = json!(delivered.len());
        }
        assert!(state.playback.runs().await.is_empty());
        evidence.push(json!({"source":source,"mode":mode,"packets":packets,"byte_identical":true}));
        let _ = api(
            &state,
            &token,
            "/Sessions/Playing/Stopped",
            Some(json!({"ItemId":item,"PlaySessionId":sid,"PositionTicks":0})),
            None,
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
        None,
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
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    task.abort();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/provider-quality");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("result.json"),serde_json::to_vec_pretty(&json!({"status":"passed","playbacks":evidence,"client_limit":constrained["MediaSources"][0]["MediaStreams"]})).unwrap()).unwrap();
}
