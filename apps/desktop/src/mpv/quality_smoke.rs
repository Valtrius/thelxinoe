use super::*;

// Uses the actual loadfile/IPC path and MPV decoder, with isolated generated media.
// Run with THELXINOE_TEST_MPV set to an mpv executable.
#[tokio::test]
#[ignore = "requires MPV and FFmpeg"]
async fn mpv_quality_reload_keeps_process_position_and_pause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let executable = std::env::var("THELXINOE_TEST_MPV")?;
    let pipe = format!(r"\\.\pipe\thelxinoe-quality-test-{}", uuid::Uuid::new_v4());
    for (name, dimensions) in [("high", "1280x720"), ("low", "640x360")] {
        ensure!(
            Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=size={dimensions}:rate=24"),
                    "-t",
                    "20",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-pix_fmt",
                    "yuv420p"
                ])
                .arg(directory.path().join(format!("{name}.mp4")))
                .creation_flags(0x08000000)
                .status()
                .await?
                .success()
        );
    }
    let mut child = Command::new(executable)
        .args([
            "--no-config",
            "--idle=yes",
            "--vo=null",
            "--ao=null",
            "--pause=yes",
            "--ytdl=no",
        ])
        .arg(format!("--input-ipc-server={pipe}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id();
    let client = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(client) = ClientOptions::new().open(&pipe) {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    let (mut ipc, mut events) = Ipc::new(client);
    let mut previous = None;
    for (name, height) in [("high", 720), ("low", 360), ("high", 720)] {
        let mut entry = Prepared {
            playlist_id: -1,
            id: name.into(),
            url: directory
                .path()
                .join(format!("{name}.mp4"))
                .to_string_lossy()
                .into(),
            data: json!({"mode":"direct","timeline_start":0,"replay_gain":"off"}),
            position: 7.0,
            sequence: 0,
            finished: false,
            activity: Default::default(),
        };
        load(&mut ipc, &mut entry, "Quality transition test", "replace").await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if event["event"] == "end-file" {
                    assert!(reload_event(&event));
                    assert_eq!(event["playlist_entry_id"].as_i64(), previous);
                    assert_ne!(event["playlist_entry_id"].as_i64(), Some(entry.playlist_id));
                }
                if event["event"] == "file-loaded" {
                    break;
                }
            }
        })
        .await?;
        previous = Some(entry.playlist_id);
        assert_eq!(
            ipc.call(json!(["get_property", "video-params/h"])).await?,
            height
        );
        assert_eq!(ipc.call(json!(["get_property", "pause"])).await?, true);
        let position = ipc
            .call(json!(["get_property", "time-pos"]))
            .await?
            .as_f64()
            .unwrap();
        assert!((position - 7.0).abs() < 0.1);
        assert_eq!(child.id(), pid);
        assert!(child.try_wait()?.is_none());
    }
    ipc.call(json!(["set_property", "pause", false])).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ipc.call(json!(["get_property", "time-pos"]))
            .await?
            .as_f64()
            .unwrap()
            > 7.1
    );
    ipc.call(json!(["quit"])).await?;
    tokio::time::timeout(Duration::from_secs(5), child.wait()).await??;
    Ok(())
}
