use super::*;
use crate::maintenance_tests::tool_runtime;
use crate::{CleanupReason, Encoding, Pipelines, RunHandle, Source, StoppedRun};
use std::time::UNIX_EPOCH;

fn run(program: &str, args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn probe(path: &Path) -> Value {
    serde_json::from_slice(
        &run(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_format",
                "-show_streams",
                "-show_frames",
                "-read_intervals",
                "%+#1",
                "-of",
                "json",
                path.to_str().unwrap(),
            ],
        )
        .stdout,
    )
    .unwrap()
}
fn gpu() -> Accelerator {
    Accelerator {
        device: "/dev/dri/renderD128".into(),
        driver: "iHD".into(),
        description: "Test".into(),
        scale: true,
        h264: true,
        hevc: true,
        hevc_10bit: true,
        tone_map: true,
        deinterlace: true,
    }
}

#[test]
fn gpu_conversions_use_only_what_the_probe_verified() {
    let convert = |accelerator: Option<&Accelerator>, stream: &Value, options: &Options| {
        let args =
            crate::video_conversion_args(options, &json!({ "streams": [stream] }), accelerator)
                .unwrap();
        let filter = args.output[args.output.iter().position(|a| a == "-vf").unwrap() + 1].clone();
        (
            args.input.contains(&"-hwaccel".to_owned()),
            filter,
            args.output,
        )
    };
    let hdr10 = json!({"codec_type":"video","codec_name":"hevc","pix_fmt":"yuv420p10le","width":3840,"height":2160,"color_transfer":"smpte2084","color_primaries":"bt2020","color_space":"bt2020nc","side_data_list":[{"side_data_type":"Mastering display metadata"}]});
    let hd = quality("1080p");
    let (decoded, filter, output) = convert(Some(&gpu()), &hdr10, &hd);
    assert!(decoded);
    assert_eq!(
        filter,
        "scale_vaapi=w=-2:h='min(1080,ih)':format=p010,tonemap_vaapi=format=nv12:t=bt709:m=bt709:p=bt709,setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709"
    );
    // Untagged primaries and matrix are BT.2020's, as in the software graph.
    let mut untagged = hdr10.clone();
    untagged["color_primaries"] = json!("unknown");
    untagged.as_object_mut().unwrap().remove("color_space");
    let (decoded, filter, _) = convert(Some(&gpu()), &untagged, &hd);
    assert!(decoded);
    assert!(
        filter.contains(
            ":format=p010,setparams=color_primaries=bt2020:colorspace=bt2020nc,tonemap_vaapi="
        ),
        "{filter}"
    );
    // The software conversion's rate limits and SDR tags.
    for pair in [
        ["-c:v", "h264_vaapi"],
        ["-rc_mode", "VBR"],
        ["-b:v", "8000000"],
        ["-maxrate", "8000000"],
        ["-bufsize", "16000000"],
        ["-color_trc", "bt709"],
    ] {
        assert!(output.windows(2).any(|w| w == pair), "{pair:?} {output:?}");
    }
    // Software decodes and filters what the GPU can't, and the GPU still encodes: HDR it can't
    // tone-map (tonemap_vaapi needs PQ with mastering display metadata), other codecs and
    // rotated pictures.
    let (_, software, _) = convert(None, &hdr10, &hd);
    let mut pq = hdr10.clone();
    pq["side_data_list"] = json!([]);
    let mut hlg = hdr10.clone();
    hlg["color_transfer"] = json!("arib-std-b67");
    let mut av1 = hdr10.clone();
    av1["codec_name"] = json!("av1");
    let mut rotated = json!({"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":1920,"height":1080,"side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]});
    let sdr_only = Accelerator {
        tone_map: false,
        ..gpu()
    };
    for (accelerator, stream) in [
        (&sdr_only, &hdr10),
        (&gpu(), &pq),
        (&gpu(), &hlg),
        (&gpu(), &av1),
        (&gpu(), &rotated),
    ] {
        let (decoded, filter, output) = convert(Some(accelerator), stream, &hd);
        assert!(!decoded, "{stream}");
        assert!(output.windows(2).any(|w| w == ["-c:v", "h264_vaapi"]));
        if stream != &rotated {
            assert_eq!(filter, format!("{software},format=nv12,hwupload"));
        }
    }
    // A Jellyfin client's limits, deinterlaced on the GPU only when it can.
    let mut tv = quality("auto");
    tv.capabilities.conversion = Some(Conversion {
        width: 1280,
        height: 720,
        frame_rate: 23.976,
        profile: "baseline".into(),
        level: 31,
        deinterlace: true,
        fit: false,
    });
    rotated["side_data_list"] = json!([]);
    let (decoded, filter, output) = convert(Some(&gpu()), &rotated, &tv);
    assert!(decoded);
    assert_eq!(
        filter,
        "deinterlace_vaapi,scale_vaapi=w=1280:h=720:format=nv12,setsar=1,fps=23.976000"
    );
    // Frames a lower rate drops are neither scaled nor tone mapped, as in the software graph.
    let mut fast = rotated.clone();
    fast["avg_frame_rate"] = json!("50/1");
    assert_eq!(
        convert(Some(&gpu()), &fast, &tv).1,
        "deinterlace_vaapi,fps=23.976000,scale_vaapi=w=1280:h=720:format=nv12,setsar=1"
    );
    // One reference frame, which h264_vaapi only keeps without B-frames.
    assert!(
        output.ends_with(
            &[
                "-profile:v",
                "constrained_baseline",
                "-level:v",
                "31",
                "-bf",
                "0"
            ]
            .map(str::to_owned)
        ),
        "{output:?}"
    );
    let progressive = Accelerator {
        deinterlace: false,
        ..gpu()
    };
    let (decoded, filter, _) = convert(Some(&progressive), &rotated, &tv);
    assert!(!decoded);
    assert!(filter.starts_with("yadif=") && filter.ends_with(",format=nv12,hwupload"));
    // Without hardware, the conversion is unchanged.
    let (decoded, _, output) = convert(None, &rotated, &tv);
    assert!(!decoded);
    assert_eq!(
        output[..8],
        [
            "-c:v", "libx264", "-preset", "veryfast", "-threads", "2", "-pix_fmt", "yuv420p"
        ]
    );
    // Dolby Vision without a base layer stays refused.
    let mut dovi = hdr10;
    dovi["side_data_list"] = json!([{"side_data_type":"DOVI configuration record","dv_profile":5,"dv_bl_signal_compatibility_id":0,"rpu_present_flag":1,"bl_present_flag":1}]);
    assert!(
        crate::video_conversion_args(&hd, &json!({ "streams": [dovi] }), Some(&gpu())).is_err()
    );
}

#[test]
fn a_burned_subtitle_converts_in_software() {
    let probe = json!({"streams":[
        {"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":1920,"height":1080},
        {"index":3,"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle"}]});
    let options = quality("720p");
    let burn = Encoding {
        video: true,
        audio: false,
        subtitle: Some(3),
    };
    // No GPU device or decoding: the burn graph's overlay draws on software frames.
    let args = burn.args(&options, &probe, Some(&gpu())).unwrap();
    assert!(args.input.is_empty(), "{:?}", args.input);
    let filter = &args.output[args.output.iter().position(|a| a == "-vf").unwrap() + 1];
    let graph = crate::burn_graph(filter, 3, 2.0);
    assert!(
        !graph.contains("vaapi") && !graph.contains("hwupload"),
        "{graph}"
    );
    assert!(args.output.windows(2).any(|w| w == ["-c:v", "libx264"]));
    // Without the subtitle, the same conversion runs on the GPU.
    let plain = Encoding {
        subtitle: None,
        ..burn
    };
    let args = plain.args(&options, &probe, Some(&gpu())).unwrap();
    assert!(args.input.contains(&"-hwaccel".to_owned()));
}

/// Pipelines and a 12 s H.264 clip, for a browser without H.264 so the video converts.
async fn clip(root: &Path) -> (Pipelines, Source, Options) {
    let media = root.join("clip.mkv");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=128x96:r=10:d=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:d=12",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "10",
            "-c:a",
            "aac",
            media.to_str().unwrap(),
        ],
    );
    let meta = std::fs::metadata(&media).unwrap();
    let source = Source {
        id: "source".into(),
        media_id: "movie".into(),
        generation: "one".into(),
        edition: String::new(),
        path: media.clone(),
        root: root.into(),
        size: meta.len(),
        modified: meta
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string(),
        probe: probe(&media),
    };
    let tools = thelxinoe_tools::Runtime::new(root.join("tools"));
    tool_runtime::install(&tools, &root.join("tools"));
    let pipelines = Pipelines::open(&root.join("cache"), tools).await.unwrap();
    let options = Options {
        capabilities: Capabilities {
            containers: vec!["mp4".into()],
            audio: vec!["aac".into()],
            hls: true,
            ..Capabilities::default()
        },
        ..quality("auto")
    };
    (pipelines, source, options)
}

#[tokio::test]
async fn a_failing_gpu_leaves_conversions_to_software() {
    let temp = tempfile::tempdir().unwrap();
    let (pipelines, source, options) = clip(temp.path()).await;
    // A node the probe once verified, now gone.
    *pipelines.hardware.status.lock().unwrap() = HardwareStatus::Ready(Accelerator {
        device: "/dev/dri/renderD200".into(),
        ..gpu()
    });
    pipelines.use_hardware(true);
    let (revision, _) = pipelines
        .start_vod("vod", &source, &options, "transcode")
        .await
        .unwrap();
    for segment in ["segment-000000.ts", "segment-000001.ts"] {
        let path = pipelines
            .file("vod", &revision, segment)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            probe(&path)["streams"][0]["codec_name"],
            "h264",
            "{segment}"
        );
    }
    // The session's later conversions skip the GPU, until it ends.
    assert!(pipelines.hardware.accelerator("vod").is_none());
    pipelines.stop("vod").await;
    assert!(pipelines.hardware.accelerator("vod").is_some());
    let (revision, _) = pipelines
        .start("live", &source, &options, "transcode", 0.0)
        .await
        .unwrap();
    assert!(
        pipelines
            .file("live", &revision, "index.m3u8")
            .await
            .unwrap()
            .is_some_and(|path| path.is_file())
    );
    pipelines.stop("live").await;
}

#[tokio::test]
async fn a_gpu_failing_mid_playback_leaves_the_session_to_software() {
    let temp = tempfile::tempdir().unwrap();
    let (pipelines, source, options) = clip(temp.path()).await;
    // Each run ends after its first segment as if the GPU ran it: one on the GPU, the other
    // on its input.
    let mut handles = Vec::new();
    for (session, error) in [
        (
            "gpu",
            "[h264_vaapi @ 0x1] [error] Failed to sync surface 0x2: 1 (operation failed).",
        ),
        (
            "input",
            "[in#0/matroska,webm @ 0x1] [error] Error during demuxing: Input/output error",
        ),
    ] {
        let (revision, _) = pipelines
            .start(session, &source, &options, "transcode", 0.0)
            .await
            .unwrap();
        pipelines.end_gpu_run(session, error).await;
        handles.push(RunHandle {
            session: session.into(),
            revision,
        });
    }
    *pipelines.hardware.status.lock().unwrap() = HardwareStatus::Ready(gpu());
    pipelines.use_hardware(true);
    let stopped = pipelines
        .maintain(&handles, &["gpu".to_owned(), "input".to_owned()])
        .await
        .unwrap();
    let reason = |session: &str| {
        stopped
            .iter()
            .find(|run| run.handle.session == session)
            .map(|run| run.reason)
    };
    assert!(matches!(reason("gpu"), Some(CleanupReason::HardwareFailed)));
    assert!(matches!(
        reason("input"),
        Some(CleanupReason::ProcessFailed)
    ));
    // Only the session the GPU failed loses it.
    assert!(pipelines.hardware.accelerator("gpu").is_none());
    assert!(pipelines.hardware.accelerator("input").is_some());
    // Its next start, a seek, converts in software.
    let (revision, _) = pipelines
        .start("gpu", &source, &options, "transcode", 6.0)
        .await
        .unwrap();
    assert_eq!(pipelines.uses_gpu("gpu").await, Some(false));
    assert!(
        pipelines
            .file("gpu", &revision, "index.m3u8")
            .await
            .unwrap()
            .is_some_and(|path| path.is_file())
    );
    // An expired session is forgotten.
    let stopped = pipelines
        .maintain(
            &[RunHandle {
                session: "gpu".into(),
                revision,
            }],
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(
        stopped[..],
        [StoppedRun {
            reason: CleanupReason::Inactive,
            ..
        }]
    ));
    assert!(pipelines.hardware.accelerator("gpu").is_some());
}

#[tokio::test]
async fn only_failures_ffmpeg_puts_on_the_gpu_or_a_run_without_frames_count_against_it() {
    let ended = |log: &[&str]| Watch::of(log, &["frame=48"]);
    // FFmpeg blames the device, a decoder that fell back to software, or a VAAPI filter.
    for log in [
        &["[VAAPI @ 0x1] [error] Failed to open /dev/dri/renderD128 as DRM device node."][..],
        &[
            "[error] Failed to set value 'vaapi=va:/dev/dri/renderD128,driver=iHD,connection_type=drm' for option 'init_hw_device': Invalid argument",
        ],
        &[
            "[hevc @ 0x1] [warning] Failed setup for format vaapi: hwaccel initialisation returned error.",
            "[vf#0:0 @ 0x2] [error] Impossible to convert between the formats supported by the filter 'graph 0 input from stream 0:0' and the filter 'auto_scale_0'",
        ],
        &["[Parsed_tonemap_vaapi_1 @ 0x1] [error] VAAPI driver doesn't support HDR"],
    ] {
        assert!(ended(log).gpu_failed(true).await, "{log:?}");
    }
    // A failing input takes the GPU encoder down with it, which also warns on healthy runs.
    for log in [
        &[
            "[in#0/matroska,webm @ 0x1] [error] Error during demuxing: Input/output error",
            "[vost#0:0/h264_vaapi @ 0x2] [error] Error submitting a packet to the muxer",
        ][..],
        &[
            "[h264_vaapi @ 0x1] [warning] Driver does not support some wanted packed headers (wanted 0xd, found 0x1).",
            "[https @ 0x2] [error] HTTP error 403 Forbidden",
        ],
    ] {
        let watch = ended(log);
        assert!(!watch.gpu_failed(true).await, "{log:?}");
        let first = log
            .iter()
            .find_map(|line| line.split_once("[error] "))
            .unwrap()
            .1;
        assert_eq!(watch.cause().await, first);
    }
    // A run out of time hung on the GPU only if it made no frame: a slow start is no fault.
    assert!(
        Watch::of(&[], &["frame=0", "progress=continue"])
            .gpu_failed(false)
            .await
    );
    assert!(
        !Watch::of(&[], &["frame=0", "frame=12", "progress=continue"])
            .gpu_failed(false)
            .await
    );
}

/// A public video served as HLS over the fixture CDN, with its keyframe index.
#[cfg(unix)]
async fn public_video(root: &Path) -> (tokio::process::Child, crate::RemoteSource, Source) {
    let media = root.join("source.mp4");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:d=18",
            "-c:v",
            "libx264",
            "-g",
            "48",
            media.to_str().unwrap(),
        ],
    );
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-i",
            media.to_str().unwrap(),
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
    );
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/helpers/online-cdn.py");
    let mut cdn = tokio::process::Command::new("python3")
        .arg(helper)
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut line = String::new();
    tokio::io::AsyncBufReadExt::read_line(
        &mut tokio::io::BufReader::new(cdn.stdout.take().unwrap()),
        &mut line,
    )
    .await
    .unwrap();
    let proxy: Value = serde_json::from_str(&line).unwrap();
    let remote = crate::RemoteSource {
        video: "https://fixture.googlevideo.com/live.m3u8".into(),
        audio: None,
        live: false,
        proxy: Some(proxy["proxy"].as_str().unwrap().into()),
        segments: (0..9).map(|index| f64::from(index) * 2.0).collect(),
    };
    let tools = thelxinoe_tools::Runtime::new(root.join("probe-tools"));
    tool_runtime::install(&tools, &root.join("probe-tools"));
    let mut probe = remote.probe(&tools.media().unwrap()).await.unwrap();
    probe["format"]["duration"] = json!("18");
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
    (cdn, remote, source)
}

#[cfg(unix)]
#[tokio::test]
async fn a_public_video_window_the_gpu_fails_resumes_in_software() {
    let temp = tempfile::tempdir().unwrap();
    let (_cdn, remote, source) = public_video(temp.path()).await;
    let (pipelines, _, _) = clip(temp.path()).await;
    // A browser without H.264 that takes fragmented MP4.
    let options = Options {
        capabilities: Capabilities {
            containers: vec!["mp4".into()],
            hls: true,
            hls_container: Some("mp4".into()),
            ..Capabilities::default()
        },
        ..quality("auto")
    };
    let planned = crate::decision(&source, &options).unwrap();
    assert!(planned.encoding.video);
    *pipelines.hardware.status.lock().unwrap() = HardwareStatus::Ready(Accelerator {
        device: "/dev/dri/renderD200".into(),
        ..gpu()
    });
    pipelines.use_hardware(true);
    let (revision, _) = pipelines
        .start_remote_vod("stream", &remote, &source, &options, &planned)
        .await
        .unwrap();
    // The GPU window fails; software converts from the requested segment, with its own
    // initialization.
    let init = pipelines
        .file("stream", &revision, "init-000002.mp4")
        .await
        .unwrap()
        .unwrap();
    let segment = pipelines
        .file("stream", &revision, "segment-000002.m4s")
        .await
        .unwrap()
        .unwrap();
    let joined = temp.path().join("joined.mp4");
    std::fs::write(
        &joined,
        [
            std::fs::read(init).unwrap(),
            std::fs::read(segment).unwrap(),
        ]
        .concat(),
    )
    .unwrap();
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-xerror",
            "-i",
            joined.to_str().unwrap(),
            "-f",
            "null",
            "-",
        ],
    );
    assert_eq!(probe(&joined)["streams"][0]["codec_name"], "h264");
    assert!(pipelines.hardware.accelerator("stream").is_none());
    pipelines.stop("stream").await;
}

#[tokio::test]
async fn probe_samples_are_the_formats_the_gpu_is_asked_to_decode() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = thelxinoe_tools::Runtime::new(temp.path().join("tools"));
    tool_runtime::install(&runtime, &temp.path().join("tools"));
    let samples = samples(&runtime.media().unwrap(), &temp.path().join("work")).await;
    for (sample, codec, format) in [
        (&samples.h264, "h264", "yuv420p"),
        (&samples.hevc, "hevc", "yuv420p"),
        (&samples.hevc_10bit, "hevc", "yuv420p10le"),
        (&samples.pq, "hevc", "yuv420p10le"),
    ] {
        let probe = probe(Path::new(sample.as_ref().unwrap()));
        let stream = &probe["streams"][0];
        assert_eq!(
            (stream["codec_name"].as_str(), stream["pix_fmt"].as_str()),
            (Some(codec), Some(format))
        );
        let tone_map = sample == &samples.pq;
        assert_eq!(crate::needs_tone_mapping(stream).unwrap(), tone_map);
        // tonemap_vaapi reads the mastering display metadata of each frame.
        assert_eq!(
            probe["frames"][0]["side_data_list"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s["side_data_type"] == "Mastering display metadata"),
            tone_map
        );
    }
}
