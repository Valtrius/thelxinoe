use serde_json::json;
use thelxinoe_playback::Source;

#[tokio::test]
async fn forced_subtitle_flags_preserve_language_and_stable_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Movie.eng.forced.default.srt");
    tokio::fs::write(&path, "1\n00:00:01,000 --> 00:00:02,000\nHello\n")
        .await
        .unwrap();
    let source = Source {
        id: "file".into(),
        media_id: "movie".into(),
        generation: "1".into(),
        edition: String::new(),
        path: temp.path().join("Movie.mkv"),
        root: temp.path().into(),
        size: 0,
        modified: "1".into(),
        probe: json!({"streams":[{"index":2,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"fra"},"disposition":{"forced":1,"default":0}}]}),
    };
    let before = source.tracks().await.unwrap();
    assert!(before[0].forced);
    assert!(!before[0].default);
    assert_eq!(before[0].language, "fra");
    let sidecar = before
        .iter()
        .find(|t| t.path.as_ref() == Some(&path))
        .unwrap();
    assert!(sidecar.forced && sidecar.default);
    assert_eq!(sidecar.language, "eng");
    tokio::fs::write(temp.path().join("Movie.ara.srt"), "")
        .await
        .unwrap();
    let after = source.tracks().await.unwrap();
    assert_eq!(
        sidecar.id,
        after
            .iter()
            .find(|t| t.path.as_ref() == Some(&path))
            .unwrap()
            .id
    );
}
