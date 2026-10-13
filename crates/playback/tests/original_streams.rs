// Server CI uses Linux; Windows Schannel rejects this self-signed fixture CDN.
#![cfg(unix)]

#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;
use std::{path::Path, process::Stdio, time::Duration};
use thelxinoe_playback::{Capabilities, Options, Pipelines, RemoteSource, Source, decision};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

async fn command(program: &str, args: &[&str]) -> Vec<u8> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let result = tokio::time::timeout(Duration::from_secs(30), command.output())
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
fn hashes(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.rsplit(',').next().unwrap().trim().to_owned())
        .collect()
}
#[tokio::test]
async fn original_live_and_seekable_online_video_keep_the_original_frames() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = root.join("source.mp4");
    command(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000",
            "-t",
            "18",
            "-c:v",
            "libx264",
            "-g",
            "48",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
            path.to_str().unwrap(),
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
            path.to_str().unwrap(),
            "-c",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "2",
            "-hls_list_size",
            "0",
            root.join("live.m3u8").to_str().unwrap(),
        ],
    )
    .await;
    // A finite fixture playlist exercises the live branch without depending on
    // an online channel, credentials, CDN availability or a runner's public IP.
    let tools = thelxinoe_tools::Runtime::new(root.join("tools"));
    tool_runtime::install(&tools, &root.join("tools"));
    let pipeline = Pipelines::open(&root.join("cache"), tools.clone())
        .await
        .unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/helpers/online-cdn.py");
    let mut python = Command::new(if cfg!(windows) { "python" } else { "python3" });
    python
        .arg(helper)
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    python.creation_flags(0x08000000);
    let mut fixture = python.spawn().unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(fixture.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let proxy: serde_json::Value = serde_json::from_str(&line).unwrap();
    let denied = RemoteSource {
        video: "https://fixture.googlevideo.com/denied.mp4?signed-secret=must-not-appear".into(),
        audio: None,
        live: false,
        proxy: Some(proxy["proxy"].as_str().unwrap().into()),
        segments: Vec::new(),
    };
    let denied_error = denied.probe(&tools.media().unwrap()).await.unwrap_err();
    assert!(denied_error.is::<thelxinoe_playback::MediaAccessDenied>());
    assert!(!format!("{denied_error:#}").contains("must-not-appear"));
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities {
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            hls: true,
            ..Default::default()
        },
    };
    let original = hashes(
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
    );
    let mut evidence = Vec::new();
    for (live, indexed) in [(true, false), (false, false), (false, true)] {
        let remote = RemoteSource {
            video: format!(
                "https://fixture.googlevideo.com/{}",
                if live || indexed {
                    "live.m3u8"
                } else {
                    "source.mp4"
                }
            ),
            audio: None,
            live,
            proxy: Some(proxy["proxy"].as_str().unwrap().into()),
            segments: if indexed {
                (0..9).map(|index| f64::from(index) * 2.0).collect()
            } else {
                Vec::new()
            },
        };
        let mut probe = remote.probe(&tools.media().unwrap()).await.unwrap();
        probe["format"]["duration"] = serde_json::json!("18");
        let source = Source {
            id: "source".into(),
            media_id: "media".into(),
            generation: "fixture".into(),
            edition: "public".into(),
            path: "source.ts".into(),
            root: root.into(),
            size: 0,
            modified: String::new(),
            probe,
        };
        let planned = decision(&source, &options).unwrap();
        assert_eq!(planned.mode, "remux");
        let (revision, _) = if live {
            pipeline
                .start_remote("stream", &remote, &options, &planned, 0.0)
                .await
                .unwrap()
        } else {
            pipeline
                .start_remote_vod("stream", &remote, &source, &options, &planned)
                .await
                .unwrap()
        };
        let manifest = pipeline
            .file("stream", &revision, "index.m3u8")
            .await
            .unwrap()
            .unwrap();
        let playlist = if live {
            // Decode a continuous stream: fragment boundaries can retain B
            // frames that need packets from the following fragment.
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let playlist = std::fs::read_to_string(&manifest).unwrap();
                    if playlist.contains("#EXT-X-ENDLIST") {
                        break playlist;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap()
        } else {
            std::fs::read_to_string(&manifest).unwrap()
        };
        let segments: Vec<_> = playlist
            .lines()
            .filter(|line| line.starts_with("segment-"))
            .collect();
        assert!(!segments.is_empty());
        if live {
            let init = pipeline
                .file("stream", &revision, "init.mp4")
                .await
                .unwrap()
                .unwrap();
            let mut bytes = std::fs::read(init).unwrap();
            for name in &segments {
                let segment = pipeline
                    .file("stream", &revision, name)
                    .await
                    .unwrap()
                    .unwrap();
                bytes.extend(std::fs::read(segment).unwrap());
            }
            let delivered = root.join("delivered.mp4");
            std::fs::write(&delivered, bytes).unwrap();
            let artifacts =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/original-stream-evidence");
            std::fs::create_dir_all(&artifacts).unwrap();
            std::fs::copy(&path, artifacts.join("source.mp4")).unwrap();
            std::fs::copy(&delivered, artifacts.join("delivered.mp4")).unwrap();
            std::fs::copy(root.join("live0.ts"), artifacts.join("upstream.ts")).unwrap();
            std::fs::write(artifacts.join("playlist.m3u8"), &playlist).unwrap();
            let actual = hashes(
                &command(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-i",
                        delivered.to_str().unwrap(),
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
            assert!(!actual.is_empty());
            assert_eq!(actual, original);
            evidence.push(serde_json::json!({"live":true,"unchanged_frames":actual.len()}));
        } else {
            for index in [segments.len() - 1, 0] {
                let segment = pipeline
                    .file("stream", &revision, segments[index])
                    .await
                    .unwrap()
                    .unwrap();
                let actual = hashes(
                    &command(
                        "ffmpeg",
                        &[
                            "-v",
                            "error",
                            "-i",
                            segment.to_str().unwrap(),
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
                assert!(!actual.is_empty());
                let first = original.iter().position(|hash| hash == &actual[0]).unwrap();
                assert_eq!(actual, original[first..first + actual.len()]);
                evidence.push(serde_json::json!({"live":false,"indexed":indexed,"segment":index,"unchanged_frames":actual.len()}));
            }
        }
        pipeline.stop("stream").await;
    }
    fixture.kill().await.unwrap();
    let output =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/original-online-playback.json");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    std::fs::write(output, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
}
