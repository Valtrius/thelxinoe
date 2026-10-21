use super::*;
use std::{path::Path, sync::Arc};
use tokio::sync::oneshot;

#[path = "../../../tests/helpers/tool-runtime.rs"]
mod tool_runtime;

async fn fixture(root: &Path) -> (Arc<Pipelines>, Source, Options) {
    let tools = thelxinoe_tools::Runtime::new(root.to_path_buf());
    tool_runtime::install(&tools, root);
    let media = root.join("source.wav");
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "18",
            "-y",
        ])
        .arg(&media);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    assert!(command.status().await.unwrap().success());
    let meta = std::fs::metadata(&media).unwrap();
    let source = Source {
        id: "source".into(),
        media_id: "media".into(),
        generation: "generation".into(),
        edition: "default".into(),
        path: media,
        root: root.into(),
        size: meta.len(),
        modified: meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string(),
        probe: serde_json::json!({"format":{"duration":"18"},"streams":[{"index":0,"codec_type":"audio","codec_name":"pcm_s16le"}]}),
    };
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities::default(),
    };
    (
        Arc::new(Pipelines::open(&root.join("cache"), tools).await.unwrap()),
        source,
        options,
    )
}

#[tokio::test]
async fn cache_turnover_during_streaming_and_vod_sweeps_is_benign() {
    for vod in [false, true] {
        for directory in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (pipelines, source, options) = fixture(temp.path()).await;
            let revision = if vod {
                pipelines
                    .start_vod("session", &source, &options, "transcode")
                    .await
                    .unwrap()
                    .0
            } else {
                pipelines
                    .start("session", &source, &options, "transcode", 0.0)
                    .await
                    .unwrap()
                    .0
            };
            let unrelated = pipelines
                .start_vod("other", &source, &options, "transcode")
                .await
                .unwrap()
                .0;
            let (entered, observed) = oneshot::channel();
            let (release, resume) = oneshot::channel();
            let directory_path = pipelines
                .file("session", &revision, "index.m3u8")
                .await
                .unwrap()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            *pipelines.scanner.before_stat.lock().await = Some((directory_path, entered, resume));
            let task = {
                let pipelines = pipelines.clone();
                tokio::spawn(async move {
                    pipelines
                        .maintain(&pipelines.runs().await, &["session".into(), "other".into()])
                        .await
                })
            };
            let entry = observed.await.unwrap();
            if directory {
                tokio::fs::remove_dir_all(entry.parent().unwrap())
                    .await
                    .unwrap();
            } else {
                tokio::fs::remove_file(entry).await.unwrap();
            }
            release.send(()).unwrap();
            task.await.unwrap().unwrap();
            pipelines
                .maintain(&pipelines.runs().await, &["session".into(), "other".into()])
                .await
                .unwrap();
            assert!(
                pipelines
                    .file("other", &unrelated, "index.m3u8")
                    .await
                    .unwrap()
                    .unwrap()
                    .is_file()
            );
            pipelines.stop("session").await;
            pipelines.stop("other").await;
        }
    }
}

#[tokio::test]
async fn cleanup_cannot_remove_a_run_published_after_its_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let (pipelines, source, options) = fixture(temp.path()).await;
    let candidates = pipelines.runs().await;
    let revision = pipelines
        .start_vod("new", &source, &options, "transcode")
        .await
        .unwrap()
        .0;
    assert!(
        pipelines
            .maintain(&candidates, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        pipelines
            .file("new", &revision, "index.m3u8")
            .await
            .unwrap()
            .unwrap()
            .is_file()
    );
    let candidates = pipelines.runs().await;
    let directory = pipelines
        .file("new", &revision, "index.m3u8")
        .await
        .unwrap()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let (entered, observed) = oneshot::channel();
    let (release, resume) = oneshot::channel();
    *pipelines.scanner.before_stat.lock().await = Some((directory, entered, resume));
    let sweeping = {
        let pipelines = pipelines.clone();
        tokio::spawn(async move { pipelines.maintain(&candidates, &[]).await })
    };
    observed.await.unwrap();
    let replacement = pipelines
        .start_vod("new", &source, &options, "transcode")
        .await
        .unwrap()
        .0;
    release.send(()).unwrap();
    assert!(sweeping.await.unwrap().unwrap().is_empty());
    assert!(
        pipelines
            .file("new", &replacement, "index.m3u8")
            .await
            .unwrap()
            .unwrap()
            .is_file()
    );
    pipelines.stop("new").await;
}

// A seek drops the request preparing a segment mid-conversion. Its partial output must go
// with it, or it counts against the run's work and cache limits until playback stops.
#[cfg(unix)]
#[tokio::test]
async fn an_abandoned_segment_leaves_no_partial_output() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let tools = thelxinoe_tools::Runtime::new(root.to_path_buf());
    tool_runtime::install(&tools, root);
    // A pipe holding the first nine seconds of an 18-second µ-law WAV, small enough for its
    // buffer: converting the second segment starts writing output, then waits for input
    // that never comes.
    let media = root.join("source.wav");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&media)
            .status()
            .unwrap()
            .success()
    );
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&media)
        .unwrap();
    let rate = 4000u32;
    let mut wav = Vec::new();
    for chunk in [
        &b"RIFF"[..],
        &(36 + rate * 18).to_le_bytes(),
        b"WAVEfmt ",
        &16u32.to_le_bytes(),
        &7u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &rate.to_le_bytes(),
        &rate.to_le_bytes(),
        &1u16.to_le_bytes(),
        &8u16.to_le_bytes(),
        b"data",
        &(rate * 18).to_le_bytes(),
    ] {
        wav.extend_from_slice(chunk);
    }
    wav.resize(wav.len() + rate as usize * 9, 0xff);
    pipe.write_all(&wav).unwrap();
    let meta = std::fs::metadata(&media).unwrap();
    let source = Source {
        id: "source".into(),
        media_id: "media".into(),
        generation: "generation".into(),
        edition: "default".into(),
        path: media,
        root: root.into(),
        size: meta.len(),
        modified: meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string(),
        probe: serde_json::json!({"format":{"duration":"18"},"streams":[{"index":0,"codec_type":"audio","codec_name":"pcm_s16le"}]}),
    };
    let options = Options {
        quality: "auto".into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities::default(),
    };
    let pipelines = Arc::new(Pipelines::open(&root.join("cache"), tools).await.unwrap());
    let (revision, _) = pipelines
        .start_vod("session", &source, &options, "transcode")
        .await
        .unwrap();
    let directory = pipelines
        .file("session", &revision, "index.m3u8")
        .await
        .unwrap()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let request = {
        let pipelines = pipelines.clone();
        let revision = revision.clone();
        tokio::spawn(async move {
            pipelines
                .file("session", &revision, "segment-000001.ts")
                .await
        })
    };
    let partial = directory.join("segment-000001.tmp");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while !partial.exists() {
            assert!(!request.is_finished(), "The conversion ended early");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("The conversion never wrote its output");
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let left: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "index.m3u8")
        .collect();
    assert!(left.is_empty(), "{left:?}");
    pipelines.stop("session").await;
    drop(pipe);
}

// A remux reads the whole file once to find its keyframes; later starts of the same file
// generation reuse the index instead of reading it again.
#[tokio::test]
async fn a_keyframe_index_is_built_once_per_file_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let tools = thelxinoe_tools::Runtime::new(root.to_path_buf());
    tool_runtime::install(&tools, root);
    let media = root.join("clip.mkv");
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=64x64:r=10",
            "-t",
            "6",
            "-c:v",
            "libx264",
            "-g",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&media);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    assert!(command.status().await.unwrap().success());
    let meta = std::fs::metadata(&media).unwrap();
    let source = Source {
        id: "clip".into(),
        media_id: "media".into(),
        generation: "generation".into(),
        edition: "default".into(),
        path: media.clone(),
        root: root.into(),
        size: meta.len(),
        modified: "1".into(),
        probe: serde_json::json!({"format":{"duration":"6","start_time":"0"}}),
    };
    let vod = crate::vod::VodCache::open(
        &root.join("cache"),
        Arc::new(tokio::sync::Semaphore::new(4)),
        tools.clone(),
        Arc::new(crate::cache::Scanner::default()),
    )
    .await
    .unwrap();
    let media_tools = tools.media().unwrap();
    let (boundaries, _) = vod.index(&source, &media_tools).await.unwrap();
    assert_eq!(boundaries, [0.0, 2.0, 4.0, 6.0]);
    // Served from the index: the file is no longer there to read.
    std::fs::remove_file(&media).unwrap();
    assert_eq!(
        vod.index(&source, &media_tools).await.unwrap().0,
        boundaries
    );
    // Another generation of the file is indexed anew.
    let changed = Source {
        modified: "2".into(),
        ..source
    };
    assert!(vod.index(&changed, &media_tools).await.is_err());
}
