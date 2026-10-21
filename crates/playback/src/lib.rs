mod cache;
#[cfg(test)]
mod maintenance_tests;
mod pipeline;
mod remote;
pub use remote::{MediaAccessDenied, RemoteSource};
mod planning;
mod vod;
pub use pipeline::{CleanupReason, Pipelines, RunHandle, StoppedRun};
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
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Conversion {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub profile: String,
    pub level: u32,
    #[serde(default)]
    pub tone_map: bool,
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
pub(crate) fn conversion_args(options: &Options, probe: &Value) -> Result<Vec<String>> {
    let mut picture = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .and_then(|s| Some((s["width"].as_u64()?, s["height"].as_u64()?)));
    let mut ceiling = u32::MAX;
    let mut filter = if let Some(height) = resolution(&options.quality) {
        format!("scale=w=-2:h='min({height},ih)'")
    } else {
        // The picture keeps its size; H.264 only needs it even.
        "scale=w='trunc(iw/2)*2':h='trunc(ih/2)*2'".to_owned()
    };
    let mut extra = Vec::new();
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
        picture = Some((c.width.into(), c.height.into()));
        ceiling = level_rate(c.level);
        filter = format!(
            "{}{}scale={}:{},setsar=1,fps={:.6}",
            if c.deinterlace {
                "yadif=mode=send_frame:parity=auto,"
            } else {
                ""
            },
            if c.tone_map {
                "zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=p=bt709:t=bt709:m=bt709:r=limited,format=yuv420p,"
            } else {
                ""
            },
            c.width,
            c.height,
            c.frame_rate
        );
        if c.fit {
            filter = format!(
                "scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={:.6}",
                c.width, c.height, c.width, c.height, c.frame_rate
            );
        }
        extra = vec![
            "-profile:v".into(),
            c.profile.clone(),
            "-level:v".into(),
            format!("{}.{}", c.level / 10, c.level % 10),
            "-refs".into(),
            "1".into(),
        ];
        if c.tone_map {
            extra.extend(
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
    }
    let rate = bitrate(&options.quality)?
        .unwrap_or_else(|| picture.map_or(8_000_000, |(w, h)| picture_rate(w, h)))
        .min(ceiling);
    let mut args = [
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-threads",
        "2",
        "-pix_fmt",
        "yuv420p",
        "-vf",
        &filter,
        "-b:v",
        &rate.to_string(),
        "-maxrate",
        &rate.to_string(),
        "-bufsize",
        &(rate * 2).to_string(),
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-ac",
        "2",
        "-ar",
        "48000",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    args.extend(extra);
    Ok(args)
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
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            quality: auto(),
            audio_language: String::new(),
            subtitle_language: String::new(),
            subtitles: false,
            replay_gain: "track".into(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        bitrate(&self.quality)?;
        if self.audio_language.len() > 16
            || self.subtitle_language.len() > 16
            || !["off", "track", "album"].contains(&self.replay_gain.as_str())
        {
            bail!("Invalid playback preferences");
        }
        Ok(())
    }
}
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
                supported: kind == "audio"
                    || ["subrip", "webvtt", "ass", "ssa", "mov_text", "text"]
                        .contains(&codec.as_str()),
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
