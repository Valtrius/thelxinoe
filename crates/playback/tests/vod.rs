#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;
fn runtime(root: &std::path::Path) -> thelxinoe_tools::Runtime {
    let tools = thelxinoe_tools::Runtime::new(root.to_path_buf());
    tool_runtime::install(&tools, root);
    tools
}
use std::{
    path::Path,
    process::{Command, Output},
    time::UNIX_EPOCH,
};
use thelxinoe_playback::{Capabilities, Conversion, Options, Pipelines, Source, decision};

fn run(program: &str, args: &[&str]) -> Output {
    let mut command = Command::new(program);
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
fn frames(path: &Path) -> Vec<f64> {
    let output = run(
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
            "-pix_fmt",
            "yuv420p",
            "-f",
            "rawvideo",
            "-",
        ],
    );
    output
        .stdout
        .as_chunks::<{ 64 * 64 * 3 / 2 }>()
        .0
        .iter()
        .map(|frame| frame[..4096].iter().map(|v| *v as f64).sum::<f64>() / 4096.0)
        .collect()
}
#[tokio::test]
async fn vod_can_seek_to_an_unprepared_segment_and_regenerate_evicted_segments() {
    let temp = tempfile::tempdir().unwrap();
    let media = temp.path().join("clock.mkv");
    run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x64:r=10,geq=lum='16+8*T':cb=128:cr=128",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "18",
            "-c:v",
            "libx264",
            "-g",
            "60",
            "-keyint_min",
            "60",
            "-sc_threshold",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            media.to_str().unwrap(),
        ],
    );
    let shifted = temp.path().join("shifted.ts");
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
            "-output_ts_offset",
            "7",
            "-muxdelay",
            "0",
            shifted.to_str().unwrap(),
        ],
    );
    let avi = temp.path().join("clock.avi");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-i",
            media.to_str().unwrap(),
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-g",
            "60",
            "-keyint_min",
            "60",
            "-sc_threshold",
            "0",
            "-c:a",
            "aac",
            avi.to_str().unwrap(),
        ],
    );
    for media in [media, shifted, avi] {
        check_timeline(temp.path(), &media).await;
    }
}
async fn check_timeline(root: &Path, media: &Path) {
    let expected = frames(media);
    assert_eq!(expected.len(), 180);
    let source = source(root, media);
    let pipelines = Pipelines::open(&root.join("cache"), runtime(&root.join("tools")))
        .await
        .unwrap();
    let options = Options {
        quality: "2mbps".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities::default(),
    };
    check_modes(&pipelines, &source, &options, &expected).await;
}
fn source(root: &Path, media: &Path) -> Source {
    let probe = run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
            media.to_str().unwrap(),
        ],
    );
    let meta = std::fs::metadata(media).unwrap();
    Source {
        id: "source".into(),
        media_id: "movie".into(),
        generation: "one".into(),
        edition: String::new(),
        path: media.to_owned(),
        root: root.to_owned(),
        size: meta.len(),
        modified: meta
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string(),
        probe: serde_json::from_slice(&probe.stdout).unwrap(),
    }
}
async fn check_modes(pipelines: &Pipelines, source: &Source, options: &Options, expected: &[f64]) {
    for mode in ["remux", "transcode"] {
        let (revision, offset) = pipelines
            .start_vod(mode, source, options, mode)
            .await
            .unwrap();
        assert_eq!(offset, 0.0);
        let manifest = pipelines
            .file(mode, &revision, "index.m3u8")
            .await
            .unwrap()
            .unwrap();
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("#EXT-X-PLAYLIST-TYPE:VOD"));
        assert!(text.contains("#EXT-X-ENDLIST"));
        let duration: f64 = text
            .lines()
            .filter_map(|line| line.strip_prefix("#EXTINF:"))
            .map(|line| line.trim_end_matches(',').parse::<f64>().unwrap())
            .sum();
        assert!((duration - source.duration()).abs() < 0.01);
        // A seek to the last six seconds must not require generating earlier video.
        let late = pipelines
            .file(mode, &revision, "segment-000002.ts")
            .await
            .unwrap()
            .unwrap();
        assert!(
            !manifest
                .parent()
                .unwrap()
                .join("segment-000000.ts")
                .exists()
        );
        let late_luma = frames(&late)[0];
        assert!(
            (late_luma - 112.0).abs() < 3.0,
            "{mode} seek luma was {late_luma}; start {:?}; manifest {text}",
            source.probe["format"]["start_time"]
        );
        let first = pipelines
            .file(mode, &revision, "segment-000000.ts")
            .await
            .unwrap()
            .unwrap();
        assert!((frames(&first)[0] - 16.0).abs() < 3.0);
        let mut decoded = Vec::new();
        for name in text.lines().filter(|line| line.starts_with("segment-")) {
            let segment = pipelines
                .file(mode, &revision, name)
                .await
                .unwrap()
                .unwrap();
            decoded.extend(frames(&segment));
        }
        assert_eq!(
            decoded.len(),
            expected.len(),
            "{mode} has missing or duplicate frames in {}",
            source.path.display()
        );
        for (index, (actual, expected)) in decoded.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 3.0,
                "{mode} frame {index}: {actual} vs {expected}"
            );
        }
        std::fs::remove_file(&late).unwrap();
        let regenerated = pipelines
            .file(mode, &revision, "segment-000002.ts")
            .await
            .unwrap()
            .unwrap();
        assert!((frames(&regenerated)[0] - late_luma).abs() < 1.0);
        assert!(
            pipelines
                .file(mode, &revision, "../../clock.mkv")
                .await
                .unwrap()
                .is_none()
        );
        pipelines.stop(mode).await;
        assert!(
            pipelines
                .file(mode, &revision, "index.m3u8")
                .await
                .unwrap()
                .is_none()
        );
        assert!(!manifest.exists());
    }
}

#[tokio::test]
async fn audio_only_vod_remux_and_conversion_keep_the_requested_interval() {
    let temp = tempfile::tempdir().unwrap();
    let media = temp.path().join("levels.m4a");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.1*(1+floor(t/6))*sin(2*PI*440*t):s=48000:d=18",
            "-c:a",
            "aac",
            media.to_str().unwrap(),
        ],
    );
    let source = source(temp.path(), &media);
    let pipelines = Pipelines::open(
        &temp.path().join("cache"),
        runtime(&temp.path().join("tools")),
    )
    .await
    .unwrap();
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities::default(),
    };
    for mode in ["remux", "transcode"] {
        let (revision, _) = pipelines
            .start_vod(mode, &source, &options, mode)
            .await
            .unwrap();
        let mut levels = Vec::new();
        for index in [2, 0] {
            let file = pipelines
                .file(mode, &revision, &format!("segment-{index:06}.ts"))
                .await
                .unwrap()
                .unwrap();
            let output = run(
                "ffmpeg",
                &[
                    "-v",
                    "error",
                    "-i",
                    file.to_str().unwrap(),
                    "-ac",
                    "1",
                    "-ar",
                    "48000",
                    "-f",
                    "f32le",
                    "-",
                ],
            );
            let samples = output
                .stdout
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b) as f64)
                .collect::<Vec<_>>();
            assert!(
                (samples.len() as i64 - 288000).abs() < 4096,
                "{mode} audio duration"
            );
            levels.push((samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt());
        }
        assert!(
            levels[0] > levels[1] * 2.4,
            "{mode} audio seek starts too early"
        );
        pipelines.stop(mode).await;
    }
}

/// Adds a placeholder Dolby Vision enhancement layer (63) and RPU (62) unit after every
/// picture of an Annex B HEVC stream.
fn with_dolby_vision_units(stream: &[u8]) -> Vec<u8> {
    let starts = (0..stream.len().saturating_sub(3))
        .filter(|&i| stream[i..i + 3] == [0, 0, 1])
        .map(|i| i + 3)
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let unit = &stream[start..starts.get(n + 1).map_or(stream.len(), |next| next - 3)];
        result.extend([0, 0, 0, 1]);
        result.extend(unit);
        if (unit[0] >> 1) & 0x3f < 32 {
            for header in [0x7e, 0x7c] {
                result.extend([0, 0, 0, 1, header, 0x01, 0x11, 0x22, 0x33, 0x44]);
            }
        }
    }
    result
}
fn dolby_vision_units(stream: &[u8]) -> usize {
    stream
        .windows(5)
        .filter(|w| w[..3] == [0, 0, 1] && [0x7c, 0x7e].contains(&w[3]) && w[4] == 1)
        .count()
}
#[tokio::test]
async fn vod_remux_strips_dolby_vision_units_when_the_client_takes_the_base_layer() {
    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("base.hevc");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x64:r=10,geq=lum='16+8*T':cb=128:cr=128",
            "-t",
            "12",
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=none:bframes=0:keyint=60:min-keyint=60:scenecut=0",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "hevc",
            raw.to_str().unwrap(),
        ],
    );
    let layered = temp.path().join("layered.hevc");
    std::fs::write(
        &layered,
        with_dolby_vision_units(&std::fs::read(&raw).unwrap()),
    )
    .unwrap();
    let media = temp.path().join("layered.mkv");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-fflags",
            "+genpts",
            "-f",
            "hevc",
            "-framerate",
            "10",
            "-i",
            layered.to_str().unwrap(),
            "-c",
            "copy",
            media.to_str().unwrap(),
        ],
    );
    let source = source(temp.path(), &media);
    let pipelines = Pipelines::open(
        &temp.path().join("cache"),
        runtime(&temp.path().join("tools")),
    )
    .await
    .unwrap();
    for strip in [false, true] {
        let id = format!("strip-{strip}");
        let options = Options {
            quality: "auto".into(),
            audio: None,
            subtitle: None,
            capabilities: Capabilities {
                strip_dolby_vision: strip,
                ..Capabilities::default()
            },
        };
        let (revision, _) = pipelines
            .start_vod(&id, &source, &options, "remux")
            .await
            .unwrap();
        let manifest = pipelines
            .file(&id, &revision, "index.m3u8")
            .await
            .unwrap()
            .unwrap();
        let (mut units, mut pictures) = (0, 0);
        for name in std::fs::read_to_string(&manifest)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("segment-"))
        {
            let segment = pipelines.file(&id, &revision, name).await.unwrap().unwrap();
            let video = run(
                "ffmpeg",
                &[
                    "-v",
                    "error",
                    "-i",
                    segment.to_str().unwrap(),
                    "-map",
                    "0:v:0",
                    "-c",
                    "copy",
                    "-f",
                    "hevc",
                    "-",
                ],
            );
            units += dolby_vision_units(&video.stdout);
            if strip {
                pictures += frames(&segment).len();
            }
        }
        if strip {
            assert_eq!(units, 0);
            assert_eq!(pictures, 120);
        } else {
            assert_eq!(units, 240);
        }
        pipelines.stop(&id).await;
    }
}

/// The average RGB of a picture's columns `x0..x1`, as a browser shows it.
fn colour(path: &Path, x0: usize, x1: usize) -> [f64; 3] {
    let output = run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-i",
            path.to_str().unwrap(),
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ],
    );
    let mut sum = [0.0; 3];
    for row in output.stdout.chunks(64 * 3).take(64) {
        for pixel in row[x0 * 3..x1 * 3].chunks(3) {
            for (total, value) in sum.iter_mut().zip(pixel) {
                *total += f64::from(*value);
            }
        }
    }
    sum.map(|total| total / (64 * (x1 - x0)) as f64)
}
#[tokio::test]
async fn vod_conversion_tone_maps_pq_and_hlg_and_refuses_dolby_vision_without_a_base_layer() {
    let temp = tempfile::tempdir().unwrap();
    let pipelines = Pipelines::open(
        &temp.path().join("cache"),
        runtime(&temp.path().join("tools")),
    )
    .await
    .unwrap();
    let mut sources = Vec::new();
    for (transfer, hdr10) in [
        (
            "smpte2084",
            ":hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400",
        ),
        ("arib-std-b67", ""),
    ] {
        let media = temp.path().join(format!("{transfer}.mkv"));
        // Red and white bars graded into BT.2020, white at the 203 nits of HDR reference white.
        run(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x64:r=10:d=3[red];color=c=white:s=32x64:r=10:d=3[white];[red][white]hstack[out0]",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:d=3",
                "-vf",
                &format!(
                    "zscale=tin=bt709:pin=bt709:min=bt709:rin=limited:t={transfer}:p=bt2020:m=bt2020nc:r=limited:npl=203,format=yuv420p10le"
                ),
                "-c:v",
                "libx265",
                "-x265-params",
                &format!("log-level=none:repeat-headers=1{hdr10}"),
                "-color_primaries",
                "bt2020",
                "-color_trc",
                transfer,
                "-colorspace",
                "bt2020nc",
                "-c:a",
                "ac3",
                media.to_str().unwrap(),
            ],
        );
        sources.push(source(temp.path(), &media));
    }
    // A PQ stream that tags its transfer only, which zscale can't convert as tagged.
    let untagged = temp.path().join("untagged.ts");
    run(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-y",
            "-i",
            sources[0].path.to_str().unwrap(),
            "-map",
            "0",
            "-c",
            "copy",
            "-bsf:v",
            "hevc_metadata=colour_primaries=2:matrix_coefficients=2",
            untagged.to_str().unwrap(),
        ],
    );
    let untagged = source(temp.path(), &untagged);
    for key in ["color_primaries", "color_space"] {
        assert!(
            untagged.probe["streams"][0][key]
                .as_str()
                .is_none_or(|tag| tag == "unknown"),
            "{key}"
        );
    }
    // A browser without HEVC or AC-3.
    let browser = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities {
            containers: vec!["mp4".into()],
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            hls: true,
            ..Capabilities::default()
        },
    };
    // A Jellyfin client conversion, at half the frame rate of the source.
    let jellyfin = Options {
        capabilities: Capabilities {
            hls: true,
            hls_container: Some("ts".into()),
            hls_video: Some(vec![]),
            hls_audio: Some(vec![]),
            conversion: Some(Conversion {
                width: 64,
                height: 64,
                frame_rate: 5.0,
                profile: "high".into(),
                level: 30,
                deinterlace: false,
                fit: false,
            }),
            ..Capabilities::default()
        },
        ..browser.clone()
    };
    for (id, source, options, pictures) in [
        ("pq", &sources[0], &browser, 30),
        ("hlg", &sources[1], &browser, 30),
        ("untagged", &untagged, &browser, 30),
        ("jellyfin", &sources[0], &jellyfin, 15),
    ] {
        let (revision, _) = pipelines
            .start_vod(id, source, options, "transcode")
            .await
            .unwrap();
        let segment = pipelines
            .file(id, &revision, "segment-000000.ts")
            .await
            .unwrap()
            .unwrap();
        let probe = run(
            "ffprobe",
            &[
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_streams",
                "-show_frames",
                "-read_intervals",
                "%+#1",
                "-of",
                "json",
                segment.to_str().unwrap(),
            ],
        );
        let probe: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let stream = &probe["streams"][0];
        for key in ["color_primaries", "color_transfer", "color_space"] {
            assert_eq!(stream[key], "bt709", "{id} {key}");
        }
        // The SDR picture carries none of the HDR metadata of the source.
        assert!(
            probe["frames"][0]["side_data_list"]
                .as_array()
                .into_iter()
                .flatten()
                .all(
                    |data| !["Mastering display metadata", "Content light level metadata"]
                        .contains(&data["side_data_type"].as_str().unwrap_or(""))
                ),
            "{id} {probe}"
        );
        // Without tone mapping, the red shows as brown.
        let red = colour(&segment, 4, 28);
        assert!(
            red[0] > 150.0 && red[1] < red[0] / 4.0 && red[2] < red[0] / 4.0,
            "{id} red {red:?}"
        );
        let white = colour(&segment, 36, 60);
        assert!(
            white
                .iter()
                .all(|v| *v > 160.0 && (v - white[1]).abs() < 12.0),
            "{id} white {white:?}"
        );
        assert_eq!(frames(&segment).len(), pictures, "{id}");
        pipelines.stop(id).await;
    }
    // Dolby Vision profile 5 has no base layer FFmpeg reproduces: its video is refused, while
    // its audio still converts.
    let mut source = sources.remove(0);
    source.probe["streams"][0]["side_data_list"] = serde_json::json!([{"side_data_type":"DOVI configuration record","dv_profile":5,"dv_bl_signal_compatibility_id":0,"rpu_present_flag":1,"bl_present_flag":1,"el_present_flag":0}]);
    assert_eq!(
        decision(&source, &browser).unwrap_err().to_string(),
        "This Dolby Vision video cannot be converted"
    );
    let mut dolby_vision = browser.clone();
    dolby_vision.capabilities.video.push("hevc".into());
    dolby_vision.capabilities.hls_video = Some(vec!["hevc".into()]);
    let planned = decision(&source, &dolby_vision).unwrap();
    assert!(!planned.encoding.video && planned.encoding.audio);
    let (revision, _) = pipelines
        .start_vod("dovi", &source, &dolby_vision, "transcode")
        .await
        .unwrap();
    assert!(
        pipelines
            .file("dovi", &revision, "segment-000000.ts")
            .await
            .unwrap()
            .is_some()
    );
    pipelines.stop("dovi").await;
}
