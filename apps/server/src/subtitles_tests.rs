use super::*;
use crate::test_support::fixture;

/// Scanned movies, each with a French SubRip and an English ASS track as streams 2 and 3,
/// in the order they were added.
async fn library(state: &AppState, titles: &[&str]) -> Vec<Source> {
    crate::test_support::tool_runtime::install(
        &state.tools.runtime,
        &state.config.state.join("tools"),
    );
    let folder = state.config.media.join("movies");
    std::fs::create_dir_all(&folder).unwrap();
    let captions = state.config.state.join("captions");
    std::fs::create_dir_all(&captions).unwrap();
    for (language, text) in [("fre", "Bonjour"), ("eng", "Hello")] {
        std::fs::write(
            captions.join(format!("{language}.srt")),
            format!("1\n00:00:00,500 --> 00:00:01,500\n{text}\n"),
        )
        .unwrap();
    }
    for title in titles {
        let output = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "color=s=64x36:r=10"])
            .args(["-f", "lavfi", "-i", "sine=sample_rate=48000"])
            .arg("-i")
            .arg(captions.join("fre.srt"))
            .arg("-i")
            .arg(captions.join("eng.srt"))
            .args([
                "-t", "2", "-map", "0:v", "-map", "1:a", "-map", "2", "-map", "3",
            ])
            .args([
                "-c:v", "libx264", "-c:a", "aac", "-c:s:0", "srt", "-c:s:1", "ass",
            ])
            .args([
                "-metadata:s:s:0",
                "language=fre",
                "-metadata:s:s:1",
                "language=eng",
            ])
            .arg(folder.join(format!("{title} (2021).mkv")))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = folder.to_string_lossy().into_owned();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "INSERT INTO library_roots(id,name,kind,path) VALUES ('movies','Movies','movies',?1)",
                [path],
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
    let mut sources = Vec::new();
    for (added, title) in titles.iter().enumerate() {
        let path = folder
            .join(format!("{title} (2021).mkv"))
            .to_string_lossy()
            .into_owned();
        let media = state
            .db
            .write("test.fixture", move |db| {
                db.execute(
                    "UPDATE media_files SET scanned_at=?1 WHERE path=?2",
                    rusqlite::params![added as i64 + 1, path],
                )?;
                Ok(db.query_row(
                    "SELECT s.media_id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE f.path=?1",
                    [path],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap();
        sources.push(crate::playback::source(state, &media, None).await.unwrap());
    }
    sources
}
async fn subtitle(source: &Source, index: i64) -> Track {
    source
        .tracks()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.index == Some(index))
        .unwrap()
}

#[tokio::test]
async fn one_pass_extracts_every_text_track_for_later_requests() {
    let (_temp, state, _) = fixture().await;
    let source = &library(&state, &["Feature"]).await[0];
    let french = track(&state, source, &subtitle(source, 2).await)
        .await
        .unwrap();
    assert!(french.ends_with("2.srt"));
    assert!(std::fs::read_to_string(french).unwrap().contains("Bonjour"));
    // The English track was extracted in the same pass, so its request no longer needs the file.
    std::fs::remove_file(&source.path).unwrap();
    let english = track(&state, source, &subtitle(source, 3).await)
        .await
        .unwrap();
    assert!(english.ends_with("3.ass"));
    assert!(std::fs::read_to_string(english).unwrap().contains("Hello"));
}

#[tokio::test]
async fn the_worker_extracts_the_newest_files_first_when_nothing_else_runs() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state, &["Older", "Newer"]).await;
    let (older, newer) = (entry(&state, &sources[0]), entry(&state, &sources[1]));
    // A due job is library work the extraction waits for.
    let job = |sql: &'static str| {
        state.db.write("test.fixture", move |db| {
            db.execute(sql, [])?;
            Ok(())
        })
    };
    job("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('scan','library.scan','movies','{}','queued',1,1)").await.unwrap();
    assert!(next(&state).await.unwrap());
    assert_eq!((settled(&older).await, settled(&newer).await), (None, None));
    job("DELETE FROM jobs").await.unwrap();
    assert!(next(&state).await.unwrap());
    assert_eq!(
        (settled(&older).await, settled(&newer).await),
        (None, Some(true))
    );
    assert!(next(&state).await.unwrap());
    assert_eq!(settled(&older).await, Some(true));
    assert!(!next(&state).await.unwrap());
    // Cleanup keeps the extractions of the files' current versions, whatever their age.
    let keep = storage::current(&state.db)
        .await
        .unwrap()
        .into_iter()
        .map(|(source, generation)| name(&source, &generation))
        .collect();
    sweep(&root(&state), &keep, Duration::ZERO).await.unwrap();
    assert_eq!(
        (settled(&older).await, settled(&newer).await),
        (Some(true), Some(true))
    );
}

#[tokio::test]
async fn a_file_that_fails_extraction_is_not_read_again_for_a_day() {
    let (_temp, state, _) = fixture().await;
    let source = &library(&state, &["Feature"]).await[0];
    // Unreadable, with the size and time the scan recorded.
    let modified = std::fs::metadata(&source.path).unwrap().modified().unwrap();
    std::fs::write(&source.path, vec![0; source.size as usize]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&source.path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    source.validate().await.unwrap();
    let french = subtitle(source, 2).await;
    assert!(track(&state, source, &french).await.is_err());
    assert_eq!(settled(&entry(&state, source)).await, Some(false));
    let error = track(&state, source, &french).await.unwrap_err();
    assert!(error.to_string().contains("less than a day ago"), "{error}");
    assert!(!next(&state).await.unwrap());
}

#[tokio::test]
async fn cleanup_removes_gone_versions_and_partial_extractions_after_a_day() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["current", "replaced", ".partial-crashed"] {
        std::fs::create_dir_all(temp.path().join(name)).unwrap();
        std::fs::write(temp.path().join(name).join("2.srt"), "").unwrap();
    }
    let keep = HashSet::from(["current".to_owned()]);
    let names = || {
        let mut names = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    sweep(temp.path(), &keep, DAY).await.unwrap();
    assert_eq!(names(), [".partial-crashed", "current", "replaced"]);
    sweep(temp.path(), &keep, Duration::ZERO).await.unwrap();
    assert_eq!(names(), ["current"]);
}
