use std::{
    path::Path,
    process::{Command, Output},
    time::UNIX_EPOCH,
};
use thelxinoe_playback::{Capabilities, Options, Pipelines, Source};

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
    for media in [media, shifted] {
        check_timeline(temp.path(), &media).await;
    }
}
async fn check_timeline(root: &Path, media: &Path) {
    let expected = frames(media);
    assert_eq!(expected.len(), 180);
    let source = source(root, media);
    let pipelines = Pipelines::open(&root.join("cache")).await.unwrap();
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
    let pipelines = Pipelines::open(&temp.path().join("cache")).await.unwrap();
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
