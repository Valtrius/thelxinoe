use crate::{Options, Source, bitrate, resolution};
use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Encoding {
    pub video: bool,
    pub audio: bool,
}
impl Encoding {
    pub(crate) fn for_mode(source: &Source, options: &Options, mode: &str) -> Result<Self> {
        if mode == "remux" {
            return Ok(Self::default());
        }
        // Low-level callers may explicitly request conversion without a client
        // profile. API callers have already validated their playback decision.
        if !options.capabilities.hls {
            return Ok(Self {
                video: source.video_codec().is_some(),
                audio: true,
            });
        }
        let planned = decision(source, options)?;
        Ok(if planned.mode == mode {
            planned.encoding
        } else {
            Self {
                video: source.video_codec().is_some(),
                audio: true,
            }
        })
    }
    pub(crate) fn args(self, options: &Options, probe: &Value) -> Result<Vec<String>> {
        if !self.video && !self.audio {
            return Ok(vec!["-c".into(), "copy".into()]);
        }
        let args = crate::conversion_args(options, probe)?;
        let mut result = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let key = &args[index];
            let audio = ["-c:a", "-b:a", "-ac", "-ar"].contains(&key.as_str());
            if (audio && self.audio) || (!audio && self.video) {
                result.extend_from_slice(&args[index..index + 2]);
            }
            index += 2;
        }
        if !self.video {
            result.extend(["-c:v".into(), "copy".into()]);
        }
        if !self.audio {
            result.extend(["-c:a".into(), "copy".into()]);
        }
        Ok(result)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub mode: &'static str,
    pub encoding: Encoding,
    pub conversion_reason: Option<String>,
    #[serde(skip)]
    pub audio_codec: Option<String>,
}

pub fn codec_string(stream: &Value) -> String {
    let name = stream["codec_name"].as_str().unwrap_or("");
    match name {
        "h264" => {
            let (profile, flags) = match stream["profile"].as_str().unwrap_or("") {
                "Constrained Baseline" => (66, 224),
                "Baseline" => (66, 0),
                "Main" => (77, 0),
                "High 10" => (110, 0),
                "High 4:2:2" => (122, 0),
                "High 4:4:4 Predictive" => (244, 0),
                _ => (100, 0),
            };
            format!(
                "avc1.{profile:02X}{flags:02X}{:02X}",
                stream["level"].as_u64().unwrap_or(40)
            )
        }
        "hevc" => format!(
            "hvc1.{}.6.L{}.B0",
            if stream["profile"] == "Main 10" { 2 } else { 1 },
            stream["level"].as_u64().unwrap_or(120)
        ),
        "vp9" => format!(
            "vp09.{:02}.10.{:02}",
            stream["profile"]
                .as_str()
                .and_then(|p| p.strip_prefix("Profile "))
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(0),
            if stream["pix_fmt"].as_str().is_some_and(|p| p.contains("10")) {
                10
            } else {
                8
            }
        ),
        "av1" => format!(
            "av01.0.08M.{:02}",
            if stream["pix_fmt"].as_str().is_some_and(|p| p.contains("10")) {
                10
            } else {
                8
            }
        ),
        "aac" => format!(
            "mp4a.40.{}",
            match stream["profile"].as_str() {
                Some("HE-AAC") => 5,
                Some("HE-AACv2") => 29,
                Some("LTP") => 4,
                _ => 2,
            }
        ),
        "mp3" => "mp4a.6B".into(),
        "ac3" => "ac-3".into(),
        "eac3" => "ec-3".into(),
        _ => name.into(),
    }
}
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}
fn supported(options: &Options, stream: Option<&Value>, container: &str, hls: bool) -> bool {
    let Some(stream) = stream else { return true };
    let Some(media) = (if hls {
        &options.capabilities.hls_media
    } else {
        &options.capabilities.supported_media
    }) else {
        return true;
    };
    if container == "wav" {
        return media.contains(&"audio/wav".into());
    }
    if ["flac", "mpeg"].contains(&container) && media.contains(&format!("audio/{container}")) {
        return true;
    }
    let kind = if stream["codec_type"] == "video" {
        "video"
    } else {
        "audio"
    };
    media.contains(&format!(
        "{kind}/{container}; codecs=\"{}\"",
        codec_string(stream)
    ))
}
pub fn decision(source: &Source, options: &Options) -> Result<Decision> {
    let rate = bitrate(&options.quality)?;
    let streams = source.probe["streams"].as_array();
    let video = streams
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let audio = streams.into_iter().flatten().find(|s| {
        s["codec_type"] == "audio" && options.audio.is_none_or(|n| s["index"].as_i64() == Some(n))
    });
    if options.audio.is_some() && audio.is_none() {
        bail!("Audio track no longer exists");
    }
    let caps = &options.capabilities;
    let video_codec = video.and_then(|s| s["codec_name"].as_str());
    let audio_codec = audio.and_then(|s| s["codec_name"].as_str());
    let video_ok = video_codec.is_none_or(|c| caps.video.iter().any(|v| v == c));
    let audio_ok = audio_codec.is_none_or(|c| caps.audio.iter().any(|v| v == c));
    let extension = source
        .path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let container = match extension.as_str() {
        "m4v" | "m4a" => "mp4",
        "mp3" => "mpeg",
        "opus" => "ogg",
        _ => &extension,
    };
    let resolution_limit = resolution(&options.quality);
    // A resolution is a ceiling, rather than a request to re-encode the same pixels.
    let resize = video.is_some_and(|v| {
        resolution_limit.is_some_and(|h| number(&v["height"]).is_none_or(|n| n > f64::from(h)))
    });
    let source_rate = number(&source.probe["format"]["bit_rate"]).or_else(|| {
        (source.size > 0 && source.duration() > 0.0)
            .then(|| source.size as f64 * 8.0 / source.duration())
    });
    let reduce_rate = resolution_limit.is_none()
        && rate.is_some_and(|r| source_rate.is_none_or(|n| n > f64::from(r)));
    let first_audio = streams
        .into_iter()
        .flatten()
        .find(|s| s["codec_type"] == "audio")
        .and_then(|s| s["index"].as_i64());
    if !resize
        && !reduce_rate
        && video_ok
        && audio_ok
        && caps.containers.contains(&extension)
        && supported(options, video, container, false)
        && supported(options, audio, container, false)
        && (caps.native_tracks || options.audio.is_none_or(|a| Some(a) == first_audio))
    {
        return Ok(Decision {
            mode: "direct",
            encoding: Encoding::default(),
            conversion_reason: None,
            audio_codec: audio_codec.map(str::to_owned),
        });
    }
    if !caps.hls {
        bail!("This client cannot play the selected media and quality");
    }
    let timestamps = extension != "avi" || video.is_none_or(|v| v["has_b_frames"] == 0);
    let hls_video = caps.hls_video.as_ref();
    let hls_audio = caps.hls_audio.as_ref();
    let copy_video = video_codec.is_none_or(|c| {
        video_ok && timestamps && hls_video.map_or(c == "h264", |list| list.iter().any(|v| v == c))
    }) && supported(options, video, "mp4", true)
        && !resize
        && !reduce_rate;
    let copy_audio = audio_codec.is_none_or(|c| {
        audio_ok
            && hls_audio.map_or(["aac", "mp3"].contains(&c), |list| {
                list.iter().any(|v| v == c)
            })
    }) && supported(options, audio, "mp4", true)
        && !(video.is_none() && reduce_rate);
    let encoding = Encoding {
        video: video.is_some() && !copy_video,
        audio: audio.is_some() && !copy_audio,
    };
    if (encoding.video || encoding.audio) && caps.allow_conversion == Some(false) {
        bail!("The client disabled the required conversion");
    }
    if options.quality == "original" && (encoding.video || encoding.audio) {
        bail!("Original quality is unsupported by this client; select Auto");
    }
    let mut reasons = Vec::new();
    if resize {
        reasons.push("Requested resolution limit");
    }
    if reduce_rate {
        reasons.push("Requested bitrate limit");
    }
    if encoding.video && !resize && !reduce_rate {
        reasons.push(if !timestamps {
            "Source timestamps cannot be copied reliably"
        } else {
            "Video format is unsupported by this client"
        });
    }
    if encoding.audio {
        reasons.push("Audio format is unsupported by this client");
    }
    Ok(Decision {
        mode: if encoding.video || encoding.audio {
            "transcode"
        } else {
            "remux"
        },
        encoding,
        conversion_reason: (!reasons.is_empty()).then(|| reasons.join("; ")),
        audio_codec: audio_codec.map(str::to_owned),
    })
}
