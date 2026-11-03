use super::*;
use crate::test_support::fixture;

/// A movie: its title, its lavfi picture and its length in seconds, then any other inputs and
/// the H.264 options.
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
/// 35 minutes, three sheets, its brightness rising with time.
const LONG: Video = (
    "Long",
    "nullsrc=s=160x90:r=1,geq=lum='40+mod(T,150)':cb=128:cr=128,format=yuv420p",
    2100,
    &["-preset", "ultrafast", "-g", "2"],
);
/// 1000 s of video in a file 30 s longer: its second window has no video.
const ENDING: Video = (
    "Ending",
    "testsrc2=s=160x90:r=1",
    1000,
    &[
        "-f",
        "lavfi",
        "-t",
        "1030",
        "-i",
        "sine=r=8000",
        "-preset",
        "ultrafast",
        "-g",
        "2",
        "-c:a",
        "aac",
    ],
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
            .args(["-t", &seconds.to_string(), "-i", picture])
            .args(*options)
            .args(["-c:v", "libx264"])
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
/// Alice plays `source`.
async fn watch(state: &AppState, source: &Source) {
    let (media, file, generation) = (
        source.media_id.clone(),
        source.id.clone(),
        source.generation.clone(),
    );
    state
        .db
        .write("test.fixture", move |db| {
            db.execute("INSERT INTO sessions(id,user_id,token_hash,transport,name,created_at,expires_at,last_seen) VALUES ('watching','alice','watching','web','Browser',1,unixepoch()+3600,1)", [])?;
            db.execute(
                "INSERT INTO playback_sessions(id,user_id,auth_session_id,media_id,file_id,generation,edition,state,mode,options,duration,created_at,updated_at) VALUES ('watching','alice','watching',?1,?2,?3,'','playing','direct','{}',2100,unixepoch(),unixepoch())",
                rusqlite::params![media, file, generation],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}
fn modified(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}
/// Sets back a file's time by two days.
fn age(path: &std::path::Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - 2 * DAY)
        .unwrap();
}
/// The frame of `source` at `seconds`, at the thumbnails' size.
fn frame(source: &Source, seconds: u32) -> image::RgbImage {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
        .arg(&source.path)
        .args(["-frames:v", "1", "-vf", "scale=320:180", "-f", "image2pipe"])
        .args(["-c:v", "png", "-"])
        .output()
        .unwrap();
    image::load_from_memory(&output.stdout).unwrap().to_rgb8()
}
fn difference(a: &image::RgbImage, b: &image::RgbImage) -> u64 {
    a.as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum()
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
async fn an_interrupted_video_keeps_its_sheets_and_continues() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state, &[ENDING, LONG]).await;
    let (ending, long) = (&sources[0], &sources[1]);
    let dir = entry(&state, long);
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100]);
    let first = sheets(&dir).join("0.jpg");
    let taken = modified(&first);
    // Library work waits for the sheet that has started, and holds back the next one.
    sql(&state, DUE_JOB).await;
    assert!(step(&state, long).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100, 100]);
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100, 100]);
    sql(&state, "DELETE FROM jobs").await;
    // Playback stops a sheet: it is dropped, the others kept.
    watch(&state, long).await;
    assert!(!step(&state, long).await.unwrap());
    assert_eq!(resume(&dir).await.sheets, [100, 100]);
    sql(&state, "DELETE FROM playback_sessions").await;
    assert_eq!(info(&state, &long.id, &long.generation).await, None);
    assert!(next(&state).await.unwrap());
    let three = info(&state, &long.id, &long.generation).await.unwrap();
    assert_eq!((three.thumbnail_count, three.sheets()), (210, 3));
    assert_eq!(modified(&first), taken);
    // The second sheet starts with the frame at 1000 s.
    let second = image::open(sheets(&dir).join("1.jpg")).unwrap().to_rgb8();
    let tile = image::imageops::crop_imm(&second, 0, 0, 320, 180).to_image();
    let nearest = [0, 990, 1000, 1010]
        .into_iter()
        .min_by_key(|&seconds| difference(&tile, &frame(long, seconds)))
        .unwrap();
    assert_eq!(nearest, 1000);
    // The window after the end of a video that ends before its file finds none, which
    // completes the version.
    assert!(next(&state).await.unwrap());
    assert_eq!(resume(&entry(&state, ending)).await.sheets, [100]);
    assert!(next(&state).await.unwrap());
    let one = info(&state, &ending.id, &ending.generation).await.unwrap();
    assert_eq!((one.thumbnail_count, one.sheets()), (100, 1));
    assert!(!next(&state).await.unwrap());
}

#[tokio::test]
async fn unusable_videos_are_skipped_for_good_and_unreadable_ones_for_a_day() {
    let (_temp, state, _) = fixture().await;
    let sources = library(&state, &[small("Moved"), small("Vision"), small("Zeroed")]).await;
    let (moved, vision, zeroed) = (&sources[0], &sources[1], &sources[2]);
    // Data FFmpeg can't read, with the size and time the scan recorded.
    let scanned = modified(&zeroed.path);
    std::fs::write(&zeroed.path, vec![0; zeroed.size as usize]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&zeroed.path)
        .unwrap()
        .set_modified(scanned)
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
    let marker = |source: &Source, name: &str| entry(&state, source).join(name);
    let attempt = || async {
        for _ in 0..3 {
            assert!(next(&state).await.unwrap());
        }
        assert!(!next(&state).await.unwrap());
    };
    // FFmpeg's first failure, like the moved file, is retried a day later; Dolby Vision isn't.
    attempt().await;
    assert!(marker(zeroed, FAILED).exists() && marker(moved, FAILED).exists());
    let reason = std::fs::read_to_string(marker(vision, UNUSABLE)).unwrap();
    assert!(reason.contains("Dolby Vision"), "{reason}");
    age(&marker(zeroed, FAILED));
    age(&marker(moved, FAILED));
    assert!(next(&state).await.unwrap());
    // Repeated a day later, FFmpeg's failure marks the version for good, with its own report.
    let reason = std::fs::read_to_string(marker(zeroed, UNUSABLE)).unwrap();
    assert!(
        reason.starts_with("FFmpeg could not take the thumbnails\n") && reason.lines().count() > 1,
        "{reason}"
    );
    assert!(next(&state).await.unwrap());
    assert!(!next(&state).await.unwrap());
    for path in [
        marker(zeroed, UNUSABLE),
        marker(vision, UNUSABLE),
        marker(moved, FAILED),
    ] {
        age(&path);
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
