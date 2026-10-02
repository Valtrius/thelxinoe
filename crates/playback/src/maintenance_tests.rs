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
