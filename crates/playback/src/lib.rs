mod cache;
mod dynamic_range;
pub use dynamic_range::{needs_tone_mapping, video_range};
mod hardware;
pub use hardware::{Accelerator, HardwareStatus};
#[cfg(test)]
mod maintenance_tests;
mod pipeline;
mod remote;
pub use remote::{MediaAccessDenied, RemoteSource};
mod planning;
mod vod;
pub use pipeline::{CleanupReason, Pipelines, RunHandle, StoppedRun};
pub(crate) use planning::Args;
pub use planning::{Decision, Encoding, codec_string, decision};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Capabilities {
    pub containers: Vec<String>,
    pub video: Vec<String>,
    pub audio: Vec<String>,
    pub hls: bool,
    #[serde(default)]
    pub native_tracks: bool,
    /// Can load separate original online video/audio through scoped server URLs.
    #[serde(default)]
    pub native_remote: bool,
    #[serde(default)]
    pub hls_container: Option<String>,
    #[serde(default)]
    pub conversion: Option<Conversion>,
    #[serde(default)]
    pub supported_media: Option<Vec<String>>,
    #[serde(default)]
    pub hls_media: Option<Vec<String>>,
    #[serde(default)]
    pub hls_video: Option<Vec<String>>,
    #[serde(default)]
    pub hls_audio: Option<Vec<String>>,
    #[serde(default)]
    pub allow_conversion: Option<bool>,
    /// Copied video loses its Dolby Vision layers, leaving the base picture the client shows.
    #[serde(default)]
    pub strip_dolby_vision: bool,
    /// Shows an embedded picture subtitle by having a conversion burn it into the video.
    #[serde(default)]
    pub burn_subtitles: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Conversion {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub profile: String,
    pub level: u32,
    #[serde(default)]
    pub deinterlace: bool,
    #[serde(default)]
    pub fit: bool,
}
/// The rate of the smallest listed resolution that holds a picture's short side.
pub fn picture_rate(width: u64, height: u64) -> u32 {
    let side = width.min(height);
    let height = [360, 480, 720, 1080, 1440, 2160]
        .into_iter()
        .find(|h| side <= *h)
        .unwrap_or(4320);
    bitrate(&format!("{height}p"))
        .ok()
        .flatten()
        .unwrap_or(8_000_000)
}
/// The video rate of a conversion: the requested one, else that of its output picture, within
/// the client's H.264 level.
pub fn conversion_rate(options: &Options, probe: &Value) -> Result<u32> {
    let conversion = options.capabilities.conversion.as_ref();
    let picture = conversion
        .map(|c| (u64::from(c.width), u64::from(c.height)))
        .or_else(|| {
            probe["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
                .and_then(|s| Some((s["width"].as_u64()?, s["height"].as_u64()?)))
        });
    Ok(bitrate(&options.quality)?
        .unwrap_or_else(|| picture.map_or(8_000_000, |(w, h)| picture_rate(w, h)))
        .min(conversion.map_or(u32::MAX, |c| level_rate(c.level))))
}
/// The filters that follow the scaling of a PQ or HLG stream and tone map it to SDR BT.709.
pub fn tone_map_filter(video: &Value) -> String {
    format!("{}{TONE_MAP}", dynamic_range::missing_hdr_tags(video))
}

/// Maps PQ or HLG to SDR BT.709 in linear light, without the HDR metadata FFmpeg would
/// otherwise pass on to the SDR output.
const TONE_MAP: &str = ",zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=p=bt709:t=bt709:m=bt709:r=limited,format=yuv420p,sidedata=mode=delete:type=MASTERING_DISPLAY_METADATA,sidedata=mode=delete:type=CONTENT_LIGHT_LEVEL";
/// FFmpeg output options converting the audio to stereo AAC.
pub(crate) const AUDIO_CONVERSION_ARGS: [&str; 8] =
    ["-c:a", "aac", "-b:a", "192k", "-ac", "2", "-ar", "48000"];
/// The `fps` filter of a conversion's limits, before scaling or after it. Frames dropped
/// first are neither scaled nor tone mapped.
pub(crate) fn frame_rate_filters(c: &Conversion, video: Option<&Value>) -> (String, String) {
    let source_rate = video
        .and_then(|v| v["avg_frame_rate"].as_str()?.split_once('/'))
        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
        .filter(|rate| rate.is_finite());
    let fps = format!("fps={:.6}", c.frame_rate);
    if source_rate.is_some_and(|rate| c.frame_rate < rate) {
        (format!("{fps},"), String::new())
    } else {
        (String::new(), format!(",{fps}"))
    }
}
/// The `scale` size the quality asks for: at most its height, else the source's.
pub(crate) fn picture_size(options: &Options) -> String {
    match resolution(&options.quality) {
        Some(height) => format!("w=-2:h='min({height},ih)'"),
        // The picture keeps its size; H.264 only needs it even.
        None => "w='trunc(iw/2)*2':h='trunc(ih/2)*2'".into(),
    }
}
/// FFmpeg options converting the video to 8-bit SDR H.264, on the GPU when `hardware` can.
pub(crate) fn video_conversion_args(
    options: &Options,
    probe: &Value,
    hardware: Option<&Accelerator>,
) -> Result<Args> {
    let video = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let tone_map = video.map(needs_tone_mapping).transpose()?.unwrap_or(false);
    // Scaled first: tone mapping works on float RGB, the costliest pixels to process.
    let tone_filter = match video {
        Some(video) if tone_map => tone_map_filter(video),
        _ => String::new(),
    };
    let mut filter = format!("scale={}{tone_filter}", picture_size(options));
    if let Some(c) = &options.capabilities.conversion {
        if !(2..=7680).contains(&c.width)
            || c.width % 2 != 0
            || !(2..=4320).contains(&c.height)
            || c.height % 2 != 0
            || !c.frame_rate.is_finite()
            || !(1.0..=60.0).contains(&c.frame_rate)
            || !["baseline", "main", "high"].contains(&c.profile.as_str())
            || ![30, 31, 32, 41, 42, 50, 51, 52, 60, 61, 62].contains(&c.level)
        {
            bail!("Invalid conversion limits");
        }
        let (before, after) = frame_rate_filters(c, video);
        filter = format!(
            "{}{before}scale={}:{}{tone_filter},setsar=1{after}",
            if c.deinterlace {
                "yadif=mode=send_frame:parity=auto,"
            } else {
                ""
            },
            c.width,
            c.height,
        );
        if c.fit {
            filter = format!(
                "{before}scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2{tone_filter},pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1{after}",
                c.width, c.height, c.width, c.height
            );
        }
    }
    let plan = hardware.map(|h| h.plan(video, options, tone_map));
    let mut output = match &plan {
        Some(plan) => vec![
            "-c:v".into(),
            "h264_vaapi".into(),
            // Chosen by the driver otherwise, possibly constant.
            "-rc_mode".into(),
            "VBR".into(),
            "-vf".into(),
            plan.filter(options, video, tone_map, &filter),
        ],
        None => [
            "-c:v", "libx264", "-preset", "veryfast", "-threads", "2", "-pix_fmt", "yuv420p",
            "-vf", &filter,
        ]
        .map(str::to_owned)
        .to_vec(),
    };
    let rate = conversion_rate(options, probe)?;
    output.extend(
        [
            "-b:v",
            &rate.to_string(),
            "-maxrate",
            &rate.to_string(),
            "-bufsize",
            &(rate * 2).to_string(),
        ]
        .map(str::to_owned),
    );
    if let Some(c) = &options.capabilities.conversion {
        let (profile, level, references) = match &plan {
            None => (
                c.profile.as_str(),
                format!("{}.{}", c.level / 10, c.level % 10),
                ["-refs", "1"],
            ),
            // One reference frame: h264_vaapi ignores -refs and references one more frame per
            // B-frame level. VAAPI encodes Baseline's constrained subset.
            Some(_) => (
                if c.profile == "baseline" {
                    "constrained_baseline"
                } else {
                    c.profile.as_str()
                },
                c.level.to_string(),
                ["-bf", "0"],
            ),
        };
        output.extend(["-profile:v", profile, "-level:v", &level].map(str::to_owned));
        output.extend(references.map(str::to_owned));
    }
    if tone_map {
        output.extend(
            [
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ]
            .map(str::to_owned),
        );
    }
    Ok(Args {
        input: plan.map(|p| p.input_args()).unwrap_or_default(),
        output,
    })
}
/// The highest rate a conversion uses at an H.264 level, well within the level's own.
pub fn level_rate(level: u32) -> u32 {
    match level {
        30 => 10_000_000,
        31 => 14_000_000,
        32 | 41 | 42 => 20_000_000,
        _ => 60_000_000,
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Options {
    #[serde(default = "auto")]
    pub quality: String,
    pub audio: Option<i64>,
    pub subtitle: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}
fn auto() -> String {
    "auto".into()
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preferences {
    pub quality: String,
    pub audio_language: String,
    pub subtitle_language: String,
    pub subtitles: bool,
    pub replay_gain: String,
    /// The scale of subtitle text in the web player.
    #[serde(default = "unscaled")]
    pub subtitle_size: f64,
    /// Whether the next episode starts by itself when one finishes.
    #[serde(default = "enabled")]
    pub autoplay: bool,
}
fn unscaled() -> f64 {
    1.0
}
fn enabled() -> bool {
    true
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            quality: auto(),
            audio_language: String::new(),
            subtitle_language: String::new(),
            subtitles: false,
            replay_gain: "track".into(),
            subtitle_size: unscaled(),
            autoplay: enabled(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        bitrate(&self.quality)?;
        if self.audio_language.len() > 16
            || self.subtitle_language.len() > 16
            || !["off", "track", "album"].contains(&self.replay_gain.as_str())
            || !(0.5..=2.0).contains(&self.subtitle_size)
        {
            bail!("Invalid playback preferences");
        }
        Ok(())
    }
}
/// The FFprobe codecs of the text subtitles the server converts to WebVTT and SRT.
pub const TEXT_SUBTITLES: [&str; 6] = ["subrip", "webvtt", "ass", "ssa", "mov_text", "text"];
/// The FFprobe codecs of the picture subtitles a conversion can burn into the video.
pub const PICTURE_SUBTITLES: [&str; 3] = ["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle"];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub index: Option<i64>,
    pub kind: String,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
    pub supported: bool,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}
impl Track {
    /// An embedded picture subtitle, which a conversion can burn into the video.
    pub fn picture(&self) -> bool {
        self.kind == "subtitle"
            && self.index.is_some()
            && PICTURE_SUBTITLES.contains(&self.codec.as_str())
    }
}
/// The first forced text subtitle in the language of audio track `audio`, or of the first audio
/// track when `None`. A picture subtitle needs a conversion, so it never shows by default.
pub fn forced_subtitle(tracks: &[Track], audio: Option<i64>) -> Option<&Track> {
    let language = tracks
        .iter()
        .find(|t| t.kind == "audio" && audio.is_none_or(|index| t.index == Some(index)))
        .map_or("und", |t| t.language.as_str());
    tracks.iter().find(|t| {
        t.kind == "subtitle"
            && t.forced
            && t.supported
            && (t.language == language || t.language == "und")
    })
}
/// The attachment streams of a file that hold fonts, which styled subtitles use.
pub fn font_attachments(probe: &Value) -> Vec<i64> {
    probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            let mime = s["tags"]["mimetype"].as_str().unwrap_or("").to_lowercase();
            let name = s["tags"]["filename"].as_str().unwrap_or("").to_lowercase();
            s["codec_type"] == "attachment"
                && (["ttf", "otf"].contains(&s["codec_name"].as_str().unwrap_or(""))
                    || ["font", "truetype", "opentype"]
                        .iter()
                        .any(|kind| mime.contains(kind))
                    || [".ttf", ".ttc", ".otf", ".otc", ".woff", ".woff2"]
                        .iter()
                        .any(|extension| name.ends_with(extension)))
        })
        .filter_map(|s| s["index"].as_i64())
        .collect()
}
/// The filter graph that draws embedded picture subtitle `subtitle` of input 0 over its video
/// once `filter` has converted it, its canvas fitted to the picture. The input starts
/// `preroll` seconds early so that the demuxer reads a subtitle already showing at the start;
/// the video drops them before any filtering. The graph paces itself to real time: under
/// `-readrate`, the overlay would wait for each next subtitle.
pub(crate) fn burn_graph(filter: &str, subtitle: i64, preroll: f64) -> String {
    format!(
        "[0:v:0]trim=start={preroll},{filter},split[main][reference];[0:{subtitle}][reference]scale=rw:rh:force_original_aspect_ratio=decrease[subtitle];[main][subtitle]overlay=(W-w)/2:(H-h)/2,setpts=PTS-{preroll}/TB,realtime[video]"
    )
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    /// Seconds into the media.
    pub start: f64,
    pub title: Option<String>,
}
/// The chapter markers FFprobe found in a file.
pub fn chapters(probe: &Value) -> Vec<Chapter> {
    probe["chapters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|chapter| {
            Some(Chapter {
                start: chapter["start_time"]
                    .as_str()?
                    .parse()
                    .ok()
                    .filter(|start: &f64| start.is_finite())?,
                title: chapter["tags"]["title"]
                    .as_str()
                    .filter(|title| !title.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub media_id: String,
    pub generation: String,
    pub edition: String,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub root: PathBuf,
    pub size: u64,
    pub modified: String,
    pub probe: Value,
}
impl Source {
    pub fn duration(&self) -> f64 {
        self.probe["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    }
    pub async fn validate(&self) -> Result<()> {
        let path = tokio::fs::canonicalize(&self.path).await?;
        let root = tokio::fs::canonicalize(&self.root).await?;
        if !path.starts_with(root) {
            bail!("Media moved outside its library");
        }
        let meta = tokio::fs::metadata(path).await?;
        let modified = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        if meta.len() != self.size || modified != self.modified {
            bail!("Media was replaced; rescan the library before playing");
        }
        Ok(())
    }
    pub async fn tracks(&self) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        for stream in self.probe["streams"].as_array().into_iter().flatten() {
            let kind = stream["codec_type"].as_str().unwrap_or("");
            if !["audio", "subtitle"].contains(&kind) {
                continue;
            }
            let index = stream["index"].as_i64().unwrap_or(0);
            let codec = stream["codec_name"].as_str().unwrap_or("").to_owned();
            tracks.push(Track {
                id: format!("embedded-{index}"),
                index: Some(index),
                kind: kind.into(),
                supported: kind == "audio" || TEXT_SUBTITLES.contains(&codec.as_str()),
                codec,
                language: stream["tags"]["language"].as_str().unwrap_or("und").into(),
                title: stream["tags"]["title"].as_str().unwrap_or("").into(),
                default: stream["disposition"]["default"].as_i64() == Some(1),
                forced: stream["disposition"]["forced"].as_i64() == Some(1),
                path: None,
            });
        }
        let Some(parent) = self.path.parent() else {
            return Ok(tracks);
        };
        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut entries = tokio::fs::read_dir(parent).await?;
        let root = tokio::fs::canonicalize(&self.root).await?;
        let mut sidecars = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_symlink() || !entry.file_type().await?.is_file() {
                continue;
            }
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if (name == stem || name.starts_with(&format!("{stem}.")))
                && ["srt", "vtt", "ass", "ssa"].contains(&ext.as_str())
                && tokio::fs::canonicalize(&path).await?.starts_with(&root)
            {
                sidecars.push((
                    path.clone(),
                    name.strip_prefix(stem)
                        .unwrap_or("")
                        .trim_start_matches('.')
                        .to_owned(),
                    ext,
                ));
            }
        }
        sidecars.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, language, codec) in sidecars {
            let flags = language.split('.').collect::<Vec<_>>();
            let forced = flags.iter().any(|s| s.eq_ignore_ascii_case("forced"));
            let default = flags.iter().any(|s| s.eq_ignore_ascii_case("default"));
            let language = flags
                .iter()
                .find(|s| {
                    !["forced", "default", "sdh", "cc", "hi"]
                        .iter()
                        .any(|flag| s.eq_ignore_ascii_case(flag))
                })
                .copied()
                .unwrap_or("und")
                .to_owned();
            let identity = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            tracks.push(Track {
                id: format!("sidecar-{identity}"),
                index: None,
                kind: "subtitle".into(),
                codec,
                language,
                title: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                default,
                forced,
                supported: true,
                path: Some(path),
            });
        }
        Ok(tracks)
    }
    pub fn video_codec(&self) -> Option<&str> {
        self.probe["streams"].as_array()?.iter().find(|s|s["codec_type"]=="video" && s["disposition"]["attached_pic"]!=1)?["codec_name"].as_str()
    }
}
pub fn resolution(quality: &str) -> Option<u32> {
    quality
        .strip_suffix('p')
        .and_then(|s| s.parse().ok())
        .filter(|h| [144, 240, 360, 480, 720, 1080, 1440, 2160, 4320].contains(h))
}
pub fn bitrate(quality: &str) -> Result<Option<u32>> {
    Ok(match quality {
        "auto" | "original" => None,
        "2mbps" => Some(2_000_000),
        "4mbps" => Some(4_000_000),
        "8mbps" => Some(8_000_000),
        "20mbps" => Some(20_000_000),
        _ => Some(match resolution(quality) {
            Some(144 | 240 | 360) => 1_000_000,
            Some(480) => 2_000_000,
            Some(720) => 4_000_000,
            Some(1080) => 8_000_000,
            Some(1440) => 16_000_000,
            Some(2160) => 25_000_000,
            Some(4320) => 60_000_000,
            _ => bail!("Unsupported quality"),
        }),
    })
}
pub fn plan(source: &Source, options: &Options) -> Result<&'static str> {
    Ok(decision(source, options)?.mode)
}
pub fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "m4a" => "audio/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "ts" => "video/mp2t",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
}
