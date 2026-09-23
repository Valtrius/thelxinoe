use crate::error::{ApiError, Result};
use serde_json::{Value, json};
use thelxinoe_playback::{Capabilities, RemoteSource};

#[derive(Clone)]
pub(super) struct Format {
    pub source: RemoteSource,
    pub native: bool,
    pub height: u64,
    pub fps: u64,
    pub video: String,
    pub audio: String,
}
impl Format {
    pub fn quality(&self) -> String {
        format!(
            "{}p{}",
            self.height,
            if self.fps > 30 {
                self.fps.to_string()
            } else {
                String::new()
            }
        )
    }
    pub fn supported(&self, caps: &Capabilities) -> bool {
        caps.video.contains(&self.video)
            && (caps.audio.contains(&self.audio)
                || (self.source.live
                    && self.audio.is_empty()
                    && caps.audio.iter().any(|c| c == "aac")))
    }
}
fn codec(value: &Value) -> String {
    let value = value.as_str().unwrap_or_default();
    if value.starts_with("avc1") || value == "h264" {
        "h264"
    } else if value.starts_with("vp09") || value == "vp9" {
        "vp9"
    } else if value.starts_with("av01") || value == "av1" {
        "av1"
    } else if value.starts_with("mp4a") || value == "aac" {
        "aac"
    } else {
        value
    }
    .into()
}
fn transport(format: &Value) -> bool {
    format["url"].is_string()
        && matches!(
            format["protocol"].as_str(),
            Some("https" | "m3u8_native" | "m3u8")
        )
}
pub(super) fn extract(metadata: &Value) -> Result<Vec<Format>> {
    let formats = metadata["formats"]
        .as_array()
        .ok_or_else(|| ApiError::conflict("No public media formats are available"))?;
    let live = metadata["is_live"] == true;
    let audio = formats
        .iter()
        .filter(|f| f["vcodec"] == "none" && f["acodec"] != "none" && transport(f))
        .max_by_key(|f| {
            (
                codec(&f["acodec"]) == "aac",
                f["protocol"] == "https",
                f["abr"].as_f64().unwrap_or(0.0) as u64,
            )
        });
    let mut output = Vec::new();
    for video in formats.iter().filter(|f| {
        transport(f)
            && f["height"].as_u64().is_some_and(|h| h > 0)
            && matches!(codec(&f["vcodec"]).as_str(), "h264" | "vp9" | "av1")
    }) {
        // Prefer SDR formats until the player negotiates HDR transfer functions.
        if video["dynamic_range"].as_str().is_some_and(|v| v != "SDR") {
            continue;
        }
        let audio = if video["acodec"].as_str().is_some_and(|v| v != "none") {
            None
        } else {
            Some(audio.ok_or_else(|| {
                ApiError::conflict("No supported public audio stream is available")
            })?)
        };
        let source = RemoteSource {
            video: video["url"].as_str().unwrap().into(),
            audio: audio.map(|f| f["url"].as_str().unwrap().into()),
            live,
        };
        source.validate().map_err(|_| {
            ApiError::conflict("The extractor returned an unsupported media address")
        })?;
        output.push(Format {
            native: !live
                && video["protocol"] == "https"
                && matches!(video["ext"].as_str(), Some("mp4" | "webm"))
                && audio.is_none_or(|f| f["protocol"] == "https"),
            source,
            height: video["height"].as_u64().unwrap(),
            fps: video["fps"].as_f64().unwrap_or(30.0).round() as u64,
            video: codec(&video["vcodec"]),
            audio: codec(&audio.unwrap_or(video)["acodec"]),
        });
    }
    output.sort_by_key(|f| (f.height, f.fps, f.video == "h264", f.native));
    if output.is_empty() {
        return Err(ApiError::conflict(
            "No supported public video stream is available",
        ));
    }
    Ok(output)
}
pub(super) fn choices(formats: &[Format], caps: &Capabilities) -> Vec<Value> {
    let mut choices = Vec::new();
    for f in formats.iter().rev().filter(|f| f.supported(caps)) {
        let quality = f.quality();
        if !choices.iter().any(|v: &Value| v["value"] == quality) {
            choices.push(json!({"value":quality,"label":quality}));
        }
    }
    choices
}
pub(super) fn choose<'a>(
    formats: &'a [Format],
    quality: &str,
    caps: &Capabilities,
) -> Result<&'a Format> {
    let eligible = |f: &&Format| f.supported(caps);
    let selected = if quality == "auto" || quality.ends_with("mbps") {
        formats
            .iter()
            .rev()
            .filter(eligible)
            .find(|f| f.height <= 1080)
            .or_else(|| formats.iter().find(eligible))
    } else if quality == "original" {
        formats.iter().rev().find(eligible)
    } else {
        formats
            .iter()
            .rev()
            .filter(eligible)
            .find(|f| f.quality() == quality)
            .or_else(|| {
                thelxinoe_playback::resolution(quality).and_then(|height| {
                    formats
                        .iter()
                        .rev()
                        .filter(eligible)
                        .find(|f| f.height <= u64::from(height))
                        .or_else(|| formats.iter().find(eligible))
                })
            })
    };
    selected.ok_or_else(|| ApiError::conflict("This resolution is not available for this player"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn available_resolutions_respect_codecs_and_select_original_source_formats() {
        let mut formats = vec![
            json!({"vcodec":"none","acodec":"mp4a.40.2","protocol":"https","url":"https://r1.googlevideo.com/audio","abr":128}),
        ];
        for (height, fps, codec) in [
            (360, 30, "avc1.4d401e"),
            (720, 60, "avc1.640028"),
            (1080, 30, "avc1.640028"),
            (1080, 30, "vp9"),
            (2160, 60, "vp9"),
        ] {
            formats.push(json!({"vcodec":codec,"acodec":"none","protocol":"https","url":"https://r1.googlevideo.com/video?private=1","ext":"mp4","height":height,"fps":fps}));
        }
        let formats = extract(&json!({"formats":formats})).unwrap();
        let mut caps = Capabilities {
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            ..Default::default()
        };
        let menu = choices(&formats, &caps);
        assert_eq!(
            menu.iter()
                .map(|v| v["value"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["1080p", "720p60", "360p"]
        );
        assert!(!serde_json::to_string(&menu).unwrap().contains("private"));
        assert_eq!(choose(&formats, "auto", &caps).unwrap().video, "h264");
        assert_eq!(choose(&formats, "720p60", &caps).unwrap().height, 720);
        assert_eq!(choose(&formats, "480p", &caps).unwrap().height, 360);
        assert!(choose(&formats, "https://elsewhere/video", &caps).is_err());
        caps.video.push("vp9".into());
        assert_eq!(choices(&formats, &caps)[0]["value"], "2160p60");
        assert_eq!(choose(&formats, "2160p60", &caps).unwrap().height, 2160);
        assert_eq!(choose(&formats, "auto", &caps).unwrap().height, 1080);
    }
}
