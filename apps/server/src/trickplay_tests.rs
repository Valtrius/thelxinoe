use super::*;
use crate::test_support::fixture;

/// Scanned 25-second movies in the order they were added: a 16:9 SDR test pattern, then a
/// 2.35:1 red picture in HDR10.
async fn library(state: &AppState) -> Vec<Source> {
    crate::test_support::tool_runtime::install(
        &state.tools.runtime,
        &state.config.state.join("tools"),
    );
    let folder = state.config.media.join("movies");
    std::fs::create_dir_all(&folder).unwrap();
    let videos: [(&str, &str, &[&str]); 2] = [
        (
            "Pattern",
            "testsrc2=s=640x360:r=24,format=yuv420p",
            &["-profile:v", "high"],
        ),
        (
            "Red",
            "color=c=red:s=640x272:r=24,format=yuv444p,setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,zscale=t=linear:npl=203,format=gbrpf32le,zscale=p=bt2020:t=smpte2084:m=bt2020nc:r=tv:npl=203,format=yuv420p10le",
            &[
                "-profile:v",
                "high10",
                "-color_primaries",
                "bt2020",
                "-color_trc",
                "smpte2084",
                "-colorspace",
                "bt2020nc",
            ],
        ),
    ];
    for (title, picture, codec) in videos {
        let output = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", picture, "-t", "25", "-c:v", "libx264", "-g", "48"])
            .args(codec)
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
    for (added, (title, _, _)) in videos.iter().enumerate() {
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
fn entry(state: &AppState, source: &Source) -> PathBuf {
    root(state).join(name(&source.id, &source.generation))
}

#[tokio::test]
async fn the_worker_takes_the_newest_videos_first_and_tone_maps_hdr() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state).await;
    let (pattern, red) = (&sources[0], &sources[1]);
    // A due job is library work the previews wait for.
    let job = |sql: &'static str| {
        state.db.write("test.fixture", move |db| {
            db.execute(sql, [])?;
            Ok(())
        })
    };
    job("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('scan','library.scan','movies','{}','queued',1,1)").await.unwrap();
    assert!(next(&state).await.unwrap());
    assert_eq!(info(&state, &red.id, &red.generation).await, None);
    job("DELETE FROM jobs").await.unwrap();
    assert!(next(&state).await.unwrap());
    assert_eq!(info(&state, &pattern.id, &pattern.generation).await, None);
    let hdr = info(&state, &red.id, &red.generation).await.unwrap();
    assert!(next(&state).await.unwrap());
    assert!(!next(&state).await.unwrap());
    let sdr = info(&state, &pattern.id, &pattern.generation)
        .await
        .unwrap();
    // Thumbnails keep the display proportions; one per 10 s from 0 s, in one padded sheet.
    for (info, height) in [(&sdr, 180), (&hdr, 136)] {
        assert_eq!(
            (info.width, info.height, info.tile_width, info.tile_height),
            (320, height, 10, 10)
        );
        assert_eq!((info.interval, info.thumbnail_count), (10_000, 2));
        assert!(info.bandwidth > 0);
    }
    assert!(sheet(&state, pattern, 320, 0).await.unwrap().exists());
    assert_eq!(sheet(&state, pattern, 320, 1).await, None);
    assert_eq!(sheet(&state, pattern, 160, 0).await, None);
    // HDR keeps its color: converted without tone mapping, this red is a dull brown.
    let picture = image::open(sheet(&state, red, 320, 0).await.unwrap())
        .unwrap()
        .to_rgb8();
    let [r, g, b] = picture.get_pixel(160, 68).0;
    assert!(r > 170 && g < 40 && b < 40, "{:?}", [r, g, b]);
    // Cleanup keeps the sheets of the files' current versions, whatever their age.
    let keep = storage::current(&state.db)
        .await
        .unwrap()
        .into_iter()
        .map(|(file, generation)| name(&file, &generation))
        .collect();
    crate::subtitles::sweep(&root(&state), &keep, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(
        (
            settled(&entry(&state, pattern)).await,
            settled(&entry(&state, red)).await
        ),
        (Some(true), Some(true))
    );
}

#[tokio::test]
async fn a_video_that_fails_is_not_read_again_for_a_day() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state).await;
    // Unreadable, with the size and time the scan recorded.
    for source in &sources {
        let modified = std::fs::metadata(&source.path).unwrap().modified().unwrap();
        std::fs::write(&source.path, vec![0; source.size as usize]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&source.path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        source.validate().await.unwrap();
    }
    assert!(next(&state).await.unwrap());
    assert!(next(&state).await.unwrap());
    for source in &sources {
        assert_eq!(settled(&entry(&state, source)).await, Some(false));
    }
    assert!(!next(&state).await.unwrap());
}
