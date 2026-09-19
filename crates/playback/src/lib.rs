mod pipeline;
pub use pipeline::Pipelines;

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
                default: false,
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
pub fn bitrate(quality: &str) -> Result<Option<u32>> {
    Ok(match quality {
        "auto" | "original" => None,
        "2mbps" => Some(2_000_000),
        "4mbps" => Some(4_000_000),
        "8mbps" => Some(8_000_000),
        "20mbps" => Some(20_000_000),
        _ => bail!("Unsupported quality"),
    })
}
pub fn plan(source: &Source, options: &Options) -> Result<&'static str> {
    let rate = bitrate(&options.quality)?;
    let video = source.video_codec();
    let audio = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| {
            s["codec_type"] == "audio"
                && options
                    .audio
                    .is_none_or(|index| s["index"].as_i64() == Some(index))
        });
    if options.audio.is_some() && audio.is_none() {
        bail!("Audio track no longer exists");
    }
    let audio_codec = audio.and_then(|s| s["codec_name"].as_str());
    let caps = &options.capabilities;
    let video_ok = video.is_none_or(|v| caps.video.iter().any(|c| c == v));
    let audio_ok = audio_codec.is_none_or(|v| caps.audio.iter().any(|c| c == v));
    let extension = source
        .path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let container = caps.containers.contains(&extension);
    let first_audio = source.probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "audio")
        .and_then(|s| s["index"].as_i64());
    if rate.is_none()
        && video_ok
        && audio_ok
        && container
        && options.audio.is_none_or(|a| Some(a) == first_audio)
    {
        return Ok("direct");
    }
    if !caps.hls {
        bail!("This client cannot play the selected media and quality");
    }
    if options.quality == "original"
        && (!video_ok
            || !audio_ok
            || !video.is_none_or(|v| v == "h264")
            || !audio_codec.is_none_or(|v| ["aac", "mp3"].contains(&v)))
    {
        bail!("Original quality is unsupported by this client; select Auto");
    }
    Ok(
        if rate.is_none()
            && video_ok
            && audio_ok
            && video.is_none_or(|v| v == "h264")
            && audio_codec.is_none_or(|v| ["aac", "mp3"].contains(&v))
        {
            "remux"
        } else {
            "transcode"
        },
    )
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
