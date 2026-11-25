use super::*;
use thelxinoe_playback::Capabilities;

fn track(id: &str, index: Option<i64>, kind: &str, codec: &str, language: &str) -> Track {
    Track {
        id: id.into(),
        index,
        kind: kind.into(),
        codec: codec.into(),
        language: language.into(),
        title: String::new(),
        default: false,
        forced: false,
        supported: kind == "audio" || codec != "hdmv_pgs_subtitle",
        path: None,
    }
}
fn tracks() -> Vec<Track> {
    let mut forced = track("embedded-5", Some(5), "subtitle", "subrip", "fra");
    forced.forced = true;
    vec![
        track("embedded-1", Some(1), "audio", "aac", "eng"),
        track("embedded-2", Some(2), "audio", "aac", "fra"),
        track("embedded-3", Some(3), "subtitle", "subrip", "eng"),
        track(
            "embedded-4",
            Some(4),
            "subtitle",
            "hdmv_pgs_subtitle",
            "fra",
        ),
        forced,
    ]
}
fn options(audio: Option<i64>, subtitle: Option<&str>) -> Options {
    Options {
        quality: "auto".into(),
        audio,
        subtitle: subtitle.map(str::to_owned),
        capabilities: Capabilities::default(),
    }
}
fn preferences(audio: &str, subtitles: Option<&str>) -> Preferences {
    Preferences {
        audio_language: audio.into(),
        subtitles: subtitles.is_some(),
        subtitle_language: subtitles.unwrap_or_default().into(),
        ..Preferences::default()
    }
}
fn selected(prefs: &Preferences, mut options: Options) -> (Option<i64>, Option<String>) {
    select_tracks(&tracks(), prefs, &mut options).unwrap();
    (options.audio, options.subtitle)
}

#[test]
fn unset_tracks_follow_the_viewer_preferences() {
    assert_eq!(
        selected(&preferences("fra", Some("eng")), options(None, None)),
        (Some(2), Some("embedded-3".into()))
    );
    // No preferred language leaves the file's own default audio, and the forced subtitle of
    // the chosen audio's language shows.
    assert_eq!(
        selected(&preferences("", None), options(Some(2), None)),
        (Some(2), Some("embedded-5".into()))
    );
    assert_eq!(
        selected(&preferences("", None), options(None, None)),
        (None, None)
    );
    // A picture subtitle needs a transcode, so a preference passes it for a text one.
    assert_eq!(
        selected(&preferences("", Some("fra")), options(Some(1), None)),
        (Some(1), Some("embedded-5".into()))
    );
}
#[test]
fn chosen_tracks_are_kept() {
    assert_eq!(
        selected(
            &preferences("fra", Some("eng")),
            options(Some(1), Some("off"))
        ),
        (Some(1), Some("off".into()))
    );
}
#[test]
fn a_client_selecting_embedded_tracks_also_selects_their_forced_subtitles() {
    let mut options = options(Some(2), None);
    options.capabilities.native_tracks = true;
    assert_eq!(selected(&preferences("", None), options), (Some(2), None));
}
#[test]
fn a_picture_subtitle_plays_only_when_the_client_burns_it_in() {
    let mut picture = options(None, Some("embedded-4"));
    assert_eq!(
        select_tracks(&tracks(), &Preferences::default(), &mut picture)
            .unwrap_err()
            .2,
        "Unsupported subtitle track"
    );
    let mut unknown = options(None, Some("embedded-9"));
    assert!(select_tracks(&tracks(), &Preferences::default(), &mut unknown).is_err());
    picture.capabilities.burn_subtitles = true;
    select_tracks(&tracks(), &Preferences::default(), &mut picture).unwrap();
}
#[test]
fn subtitles_list_what_the_client_can_show_with_their_files() {
    let urls = |burn| {
        subtitles(&tracks(), burn, "session", "grant")
            .into_iter()
            .map(|s| (s.id, s.url))
            .collect::<Vec<_>>()
    };
    let text = |id: &str| {
        (
            id.to_owned(),
            Some(format!(
                "/api/v1/playback/session/subtitles/{id}?grant=grant"
            )),
        )
    };
    assert_eq!(urls(false), [text("embedded-3"), text("embedded-5")]);
    assert_eq!(
        urls(true),
        [
            text("embedded-3"),
            ("embedded-4".into(), None),
            text("embedded-5")
        ]
    );
}

/// A scanned movie with an English and a French audio track (streams 1 and 2) and an English
/// SubRip track (stream 3), and the principal of its viewer.
async fn movie() -> (tempfile::TempDir, AppState, Principal, String, String) {
    let (temp, state, cookie) = crate::test_support::fixture().await;
    crate::test_support::tool_runtime::install(
        &state.tools.runtime,
        &state.config.state.join("tools"),
    );
    let folder = state.config.media.join("movies");
    std::fs::create_dir_all(&folder).unwrap();
    let captions = state.config.state.join("captions.srt");
    std::fs::write(&captions, "1\n00:00:00,500 --> 00:00:01,500\nHello\n").unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=s=64x36:r=10"])
        .args(["-f", "lavfi", "-i", "sine=sample_rate=48000"])
        .args(["-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000"])
        .arg("-i")
        .arg(&captions)
        .args([
            "-t", "4", "-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3",
        ])
        .args(["-c:v", "libx264", "-c:a", "aac", "-c:s", "srt"])
        .args([
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=fre",
        ])
        .args(["-metadata:s:s:0", "language=eng"])
        .arg(folder.join("Tracks (2021).mkv"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = folder.to_string_lossy().into_owned();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies',?1)",
                [path],
            )?;
            db.execute(
                r#"INSERT INTO playback_preferences VALUES ('alice','{"quality":"auto","audio_language":"fre","subtitle_language":"eng","subtitles":true,"replay_gain":"album"}')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    for root in thelxinoe_catalog::roots(&state.db).await.unwrap() {
        thelxinoe_catalog::scan(&state.db, root, state.tools.runtime.media().unwrap())
            .await
            .unwrap();
    }
    let media = state
        .db
        .read("test.media", |db| {
            Ok(db.query_row("SELECT id FROM media", [], |r| r.get::<_, String>(0))?)
        })
        .await
        .unwrap();
    let p = crate::security::principal(
        &state,
        &[(axum::http::header::COOKIE, cookie.parse().unwrap())]
            .into_iter()
            .collect(),
    )
    .await
    .unwrap();
    (temp, state, p, cookie, media)
}
fn browser(video: &[&str]) -> Options {
    let mut options = options(None, None);
    options.capabilities = Capabilities {
        containers: vec!["mkv".into()],
        video: video.iter().map(|&codec| codec.into()).collect(),
        audio: vec!["aac".into()],
        hls: true,
        ..Capabilities::default()
    };
    options
}
fn create(media: &str, options: Options) -> Create {
    Create {
        media_id: media.into(),
        file_id: None,
        position: None,
        options,
        orientation: None,
    }
}

#[tokio::test]
async fn a_started_session_plays_the_viewers_preferred_tracks() {
    let (_temp, state, p, cookie, media) = movie().await;
    let started = start(&state, &p, create(&media, browser(&["h264"])), false)
        .await
        .unwrap();
    assert_eq!(started.options.audio, Some(2));
    assert_eq!(started.selected_subtitle.as_deref(), Some("embedded-3"));
    // Only the first audio track plays directly; HLS selects the French one.
    assert_eq!(started.mode, "remux");
    assert_eq!(
        (started.encoding.video, started.encoding.audio),
        (false, false)
    );
    let id = &started.id;
    assert!(
        started
            .url
            .starts_with(&format!("/api/v1/playback/{id}/hls/"))
    );
    assert!(started.url.ends_with(&format!("?grant={}", started.grant)));
    assert_eq!(
        started
            .subtitles
            .iter()
            .map(|s| s.url.clone())
            .collect::<Vec<_>>(),
        [Some(format!(
            "/api/v1/playback/{id}/subtitles/embedded-3?grant={}",
            started.grant
        ))]
    );
    assert_eq!(started.replay_gain, "album");
    let path = format!("/api/v1/playback/{id}");
    let stopped =
        crate::test_support::call(&state, &path, "DELETE", serde_json::json!({}), &cookie);
    assert_eq!(stopped.await.0, axum::http::StatusCode::OK);
}
#[tokio::test]
async fn a_start_the_transcoding_policy_forbids_is_refused_before_a_session_exists() {
    let (_temp, state, p, _cookie, media) = movie().await;
    state.transcoding.write().await.video_transcoding = false;
    let refused = start(&state, &p, create(&media, browser(&[])), false)
        .await
        .unwrap_err();
    assert_eq!(
        (refused.0, refused.1),
        (axum::http::StatusCode::FORBIDDEN, "transcoding_disabled")
    );
    let sessions = state
        .db
        .read("test.sessions", |db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM playback_sessions", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    // Audio is copied, so only a video transcode is refused.
    let mut copied = browser(&["h264"]);
    copied.audio = Some(1);
    let started = start(&state, &p, create(&media, copied), false).await;
    assert_eq!(started.unwrap().mode, "direct");
}
