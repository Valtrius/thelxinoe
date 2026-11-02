use super::*;
use crate::test_support::fixture;

/// A movie: its title, its lavfi picture, its length in seconds and its H.264 options.
type Video = (&'static str, &'static str, u32, &'static [&'static str]);
const PATTERN: Video = (
    "Pattern",
    "testsrc2=s=640x360:r=24,format=yuv420p",
    25,
    &["-profile:v", "high", "-g", "48"],
);
/// A 2.35:1 red picture in HDR10.
const RED: Video = (
    "Red",
    "color=c=red:s=640x272:r=24,format=yuv444p,setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,zscale=t=linear:npl=203,format=gbrpf32le,zscale=p=bt2020:t=smpte2084:m=bt2020nc:r=tv:npl=203,format=yuv420p10le",
    25,
    &[
        "-profile:v",
        "high10",
        "-g",
        "48",
        "-color_primaries",
        "bt2020",
        "-color_trc",
        "smpte2084",
        "-colorspace",
        "bt2020nc",
    ],
);
/// 35 minutes, three sheets.
const LONG: Video = (
    "Long",
    "testsrc2=s=160x90:r=1",
    2100,
    &["-preset", "ultrafast", "-g", "2"],
);
const fn small(title: &'static str) -> Video {
    (title, "testsrc2=s=160x90:r=24", 25, &["-g", "48"])
}

/// The scanned movies, in the order they were added.
async fn library(state: &AppState, videos: &[Video]) -> Vec<Source> {
    crate::test_support::tool_runtime::install(
        &state.tools.runtime,
        &state.config.state.join("tools"),
    );
    let folder = state.config.media.join("movies");
    std::fs::create_dir_all(&folder).unwrap();
    for (title, picture, seconds, options) in videos {
        let output = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", picture, "-t", &seconds.to_string(), "-c:v", "libx264"])
            .args(*options)
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
    for (added, (title, _, _, _)) in videos.iter().enumerate() {
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
async fn sql(state: &AppState, sql: &'static str) {
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(sql, [])?;
            Ok(())
        })
        .await
        .unwrap();
}
/// A scan, the library work previews wait for.
const DUE_JOB: &str = "INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('scan','library.scan','movies','{}','queued',1,1)";

#[tokio::test]
async fn the_worker_takes_the_newest_videos_first_and_tone_maps_hdr() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state, &[PATTERN, RED]).await;
    let (pattern, red) = (&sources[0], &sources[1]);
    sql(&state, DUE_JOB).await;
    assert!(next(&state).await.unwrap());
    assert_eq!(info(&state, &red.id, &red.generation).await, None);
    sql(&state, "DELETE FROM jobs").await;
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
        assert_eq!((info.interval, info.thumbnail_count), (10_000, 3));
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
async fn a_video_interrupted_mid_run_keeps_its_sheets_and_continues() {
    let (_temp, state, _) = fixture().await;
    let long = &library(&state, &[LONG]).await[0];
    let dir = entry(&state, long);
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100]);
    let first = sheets(&dir).join("0.jpg");
    let taken = std::fs::metadata(&first).unwrap().modified().unwrap();
    // A job due while a sheet is taken stops FFmpeg: that sheet is dropped, the first kept.
    sql(&state, DUE_JOB).await;
    assert!(!step(&state, long).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100]);
    // Later turns wait for the job, then continue from the second sheet.
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100]);
    sql(&state, "DELETE FROM jobs").await;
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100, 100]);
    assert_eq!(info(&state, &long.id, &long.generation).await, None);
    assert!(next(&state).await.unwrap());
    let info = info(&state, &long.id, &long.generation).await.unwrap();
    assert_eq!((info.thumbnail_count, info.sheets()), (210, 3));
    assert_eq!(
        std::fs::metadata(&first).unwrap().modified().unwrap(),
        taken
    );
    assert!(!next(&state).await.unwrap());
}

#[tokio::test]
async fn unusable_videos_are_skipped_for_good_and_unreadable_ones_for_a_day() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state, &[small("Moved"), small("Vision"), small("Zeroed")]).await;
    let (moved, vision, zeroed) = (&sources[0], &sources[1], &sources[2]);
    // Data FFmpeg can't read, with the size and time the scan recorded.
    let modified = std::fs::metadata(&zeroed.path).unwrap().modified().unwrap();
    std::fs::write(&zeroed.path, vec![0; zeroed.size as usize]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&zeroed.path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    // Dolby Vision profile 5 has no base layer to show instead.
    let file = vision.id.clone();
    state
        .db
        .write("test.fixture", move |db| {
            db.execute(
                "UPDATE media_files SET probe=json_set(probe,'$.streams[0].side_data_list',json('[{\"side_data_type\":\"DOVI configuration record\",\"dv_profile\":5,\"dv_bl_signal_compatibility_id\":0,\"rpu_present_flag\":1,\"bl_present_flag\":1}]')) WHERE id=?1",
                [file],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Gone before it is read.
    std::fs::remove_file(&moved.path).unwrap();
    for _ in 0..3 {
        assert!(next(&state).await.unwrap());
    }
    assert!(!next(&state).await.unwrap());
    let marker = |source: &Source, name: &str| entry(&state, source).join(name);
    let ffmpeg = std::fs::read_to_string(marker(zeroed, UNUSABLE)).unwrap();
    // FFmpeg's own report follows.
    assert!(
        ffmpeg.starts_with("FFmpeg could not take the thumbnails\n") && ffmpeg.lines().count() > 1,
        "{ffmpeg}"
    );
    assert!(
        std::fs::read_to_string(marker(vision, UNUSABLE))
            .unwrap()
            .contains("Dolby Vision")
    );
    // A day later, only the version that could not be read is tried again.
    let earlier = std::time::SystemTime::now() - 2 * DAY;
    for path in [
        marker(zeroed, UNUSABLE),
        marker(vision, UNUSABLE),
        marker(moved, FAILED),
    ] {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
    }
    assert_eq!(
        (
            settled(&entry(&state, moved)).await,
            settled(&entry(&state, vision)).await,
            settled(&entry(&state, zeroed)).await
        ),
        (None, Some(false), Some(false))
    );
}
