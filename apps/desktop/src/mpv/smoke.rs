use super::*;

// Uses the actual loadfile/IPC path and MPV decoder, with isolated generated media.
// Run with THELXINOE_TEST_MPV set to an mpv executable.
async fn encode(
    path: &std::path::Path,
    dimensions: &str,
    extra: &[&std::ffi::OsStr],
) -> Result<()> {
    ensure!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=size={dimensions}:rate=24"),
            ])
            .args(extra)
            .args([
                "-t",
                "20",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p"
            ])
            .arg(path)
            .creation_flags(0x08000000)
            .status()
            .await?
            .success()
    );
    Ok(())
}
async fn launch(name: &str) -> Result<(Child, Ipc, mpsc::Receiver<Value>)> {
    let executable = std::env::var("THELXINOE_TEST_MPV")?;
    let pipe = format!(r"\\.\pipe\thelxinoe-{name}-test-{}", uuid::Uuid::new_v4());
    let child = Command::new(executable)
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
    let client = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(client) = ClientOptions::new().open(&pipe) {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    let (ipc, events) = Ipc::new(client);
    Ok((child, ipc, events))
}
fn entry(path: std::path::PathBuf, data: Value) -> Prepared {
    Prepared {
        playlist_id: -1,
        id: "test".into(),
        url: path.to_string_lossy().into(),
        data,
        position: 7.0,
        sequence: 0,
        finished: false,
        activity: Default::default(),
    }
}

#[tokio::test]
#[ignore = "requires MPV and FFmpeg"]
async fn mpv_quality_reload_keeps_process_position_and_pause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    for (name, dimensions) in [("high", "1280x720"), ("low", "640x360")] {
        encode(
            &directory.path().join(format!("{name}.mp4")),
            dimensions,
            &[],
        )
        .await?;
    }
    let (mut child, mut ipc, mut events) = launch("quality").await?;
    let pid = child.id();
    let mut previous = None;
    for (name, height) in [("high", 720), ("low", 360), ("high", 720)] {
        let mut entry = entry(
            directory.path().join(format!("{name}.mp4")),
            json!({"mode":"direct","timeline_start":0,"replay_gain":"off"}),
        );
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

#[tokio::test]
#[ignore = "requires MPV and FFmpeg"]
async fn mpv_takes_server_chapters_only_for_media_without_its_own() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let metadata = directory.path().join("chapters.txt");
    tokio::fs::write(
        &metadata,
        ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=20000\ntitle=Own\n",
    )
    .await?;
    encode(&directory.path().join("plain.mp4"), "640x360", &[]).await?;
    encode(
        &directory.path().join("chaptered.mp4"),
        "640x360",
        &[
            "-i".as_ref(),
            metadata.as_os_str(),
            "-map_chapters".as_ref(),
            "1".as_ref(),
        ],
    )
    .await?;
    let (mut child, mut ipc, mut events) = launch("chapters").await?;
    let server = json!([{"start":0.0,"title":"Opening"},{"start":12.5,"title":null},{"start":40.0,"title":"Beyond"}]);
    for (name, timeline_start, expected) in [
        // Chapters past the media's end are left out.
        (
            "plain",
            0.0,
            json!([{"time":0.0,"title":"Opening"},{"time":12.5,"title":""}]),
        ),
        ("chaptered", 0.0, json!([{"time":0.0,"title":"Own"}])),
        // A stream that starts later keeps the server's chapters on its own timeline.
        ("plain", 10.0, json!([{"time":2.5,"title":""}])),
    ] {
        let mut entry = entry(
            directory.path().join(format!("{name}.mp4")),
            json!({"mode":"direct","timeline_start":timeline_start,"replay_gain":"off","chapters":server}),
        );
        load(&mut ipc, &mut entry, "Chapter test", "replace").await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if event["event"] == "file-loaded" {
                    break;
                }
            }
        })
        .await?;
        chapters(&mut ipc, &entry.data).await?;
        let list = ipc.call(json!(["get_property", "chapter-list"])).await?;
        let list: Vec<_> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|chapter| json!({"time":chapter["time"].as_f64().unwrap(),"title":chapter["title"]}))
            .collect();
        assert_eq!(json!(list), expected, "{name} from {timeline_start}");
    }
    ipc.call(json!(["quit"])).await?;
    tokio::time::timeout(Duration::from_secs(5), child.wait()).await??;
    Ok(())
}
