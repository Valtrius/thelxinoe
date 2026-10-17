use crate::error::{ApiError, Result};
use serde_json::{Value, json};
use thelxinoe_playback::{Capabilities, RemoteSource};

#[derive(Clone)]
pub(super) struct Format {
    pub source: RemoteSource,
    /// Progressive files that a client can read through the relay.
    pub native: bool,
    /// Fragmented MP4 video and audio files that play as byte-range HLS.
    pub indexed: bool,
    pub height: u64,
    pub width: Option<u64>,
    pub fps: u64,
    pub frame_rate: Option<f64>,
    pub bitrate: Option<u64>,
    pub video: String,
    pub audio: String,
    /// RFC 6381 codec strings as the provider declares them.
    pub codecs: [String; 2],
    audio_bitrate: Option<u64>,
    sample_rate: Option<u64>,
    channels: Option<u64>,
    /// The provider refuses its media before this time, e.g. during a pre-roll ad.
    pub available_at: i64,
}
impl Format {
    pub fn descriptor(&self) -> Value {
        json!({"quality":self.quality(),"video":self.video,"audio":self.audio,"width":self.width,"height":self.height,"fps":self.frame_rate,"bitrate":self.bitrate})
    }
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
    /// Whether a client can play this rendition as fragmented MP4 HLS.
    pub fn indexed_for(&self, caps: &Capabilities) -> bool {
        let video = caps.hls_video.as_ref().unwrap_or(&caps.video);
        let audio = caps.hls_audio.as_ref().unwrap_or(&caps.audio);
        self.indexed
            && caps.hls
            && caps.hls_container.as_deref().is_none_or(|c| c == "mp4")
            && video.contains(&self.video)
            && audio.contains(&self.audio)
            // Browsers may decode AV1 only up to a level, which their exact
            // codec support reports.
            && (self.video != "av1"
                || caps.hls_media.as_ref().is_none_or(|media| {
                    let wanted = format!("video/mp4; codecs=\"{}\"", self.codecs[0]);
                    media.iter().any(|m| m.eq_ignore_ascii_case(&wanted))
                }))
    }
    /// The media description FFprobe would report, from the provider's metadata.
    pub fn probe(&self, duration: f64) -> Value {
        let (profile, level, depth) = video_profile(&self.codecs[0]);
        let rate = self.frame_rate.map(|rate| {
            if rate.fract() == 0.0 {
                format!("{rate}/1")
            } else {
                format!("{:.0}/1000", rate * 1000.0)
            }
        });
        let start = "0.000000";
        json!({
            "format": {"duration": duration.to_string(), "start_time": start, "bit_rate": self.bitrate.map(|b| b.to_string())},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": self.video, "codec_tag_string": self.codecs[0].split('.').next(), "profile": profile, "level": level, "pix_fmt": if depth > 8 { "yuv420p10le" } else { "yuv420p" }, "width": self.width, "height": self.height, "avg_frame_rate": rate, "r_frame_rate": rate, "field_order": "progressive", "sample_aspect_ratio": "1:1", "is_avc": (self.video == "h264").then_some("true"), "start_time": start},
                {"index": 1, "codec_type": "audio", "codec_name": self.audio, "profile": audio_profile(&self.codecs[1]), "sample_rate": self.sample_rate.map(|r| r.to_string()), "channels": self.channels, "bit_rate": self.audio_bitrate.map(|b| b.to_string()), "start_time": start}
            ]
        })
    }
}
/// Profile name, level and bit depth from an RFC 6381 video codec string.
fn video_profile(codec: &str) -> (Option<String>, Option<u64>, u64) {
    let parts: Vec<&str> = codec.split('.').collect();
    let hex = |value: &str| u64::from_str_radix(value, 16).ok();
    match parts.as_slice() {
        ["avc1" | "avc3", value] if value.len() == 6 => {
            let profile = hex(&value[..2]).unwrap_or(0);
            let constrained = hex(&value[2..4]).is_some_and(|flags| flags & 0x40 != 0);
            let name = match profile {
                66 if constrained => "Constrained Baseline",
                66 => "Baseline",
                77 => "Main",
                88 => "Extended",
                110 => "High 10",
                122 => "High 4:2:2",
                244 => "High 4:4:4 Predictive",
                _ => "High",
            };
            (
                Some(name.into()),
                hex(&value[4..]),
                if profile == 110 { 10 } else { 8 },
            )
        }
        ["av01", profile, level, depth, ..] => (
            Some(
                match *profile {
                    "1" => "High",
                    "2" => "Professional",
                    _ => "Main",
                }
                .into(),
            ),
            level.get(..2).and_then(|l| l.parse().ok()),
            depth.parse().unwrap_or(8),
        ),
        ["vp09", profile, level, depth, ..] => (
            Some(format!("Profile {}", profile.parse::<u8>().unwrap_or(0))),
            level.parse().ok(),
            depth.parse().unwrap_or(8),
        ),
        _ => (None, None, 8),
    }
}
fn audio_profile(codec: &str) -> Option<&'static str> {
    match codec {
        "mp4a.40.2" => Some("LC"),
        "mp4a.40.5" => Some("HE-AAC"),
        "mp4a.40.29" => Some("HE-AACv2"),
        _ => None,
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
    } else if value.starts_with("hvc1") || value.starts_with("hev1") || value == "hevc" {
        "hevc"
    } else if value.starts_with("mp4a") || value == "aac" {
        "aac"
    } else {
        value
    }
    .into()
}
pub(super) fn extract(metadata: &Value, proxy: Option<String>) -> Result<Vec<Format>> {
    let formats = metadata["formats"]
        .as_array()
        .ok_or_else(|| ApiError::conflict("No public media formats are available"))?;
    let live = metadata["is_live"] == true;
    // Live streams arrive as playlists; recorded videos as separate files.
    let transport = |format: &Value| {
        format["url"].is_string()
            && match format["protocol"].as_str() {
                Some("https") => true,
                Some("m3u8_native" | "m3u8") => live,
                _ => false,
            }
    };
    let audio = formats
        .iter()
        .filter(|f| f["vcodec"] == "none" && f["acodec"] != "none" && transport(f))
        .max_by_key(|f| {
            (
                codec(&f["acodec"]) == "aac",
                f["protocol"] == "https",
                // The original language over dubbed tracks, then original loudness.
                f["language_preference"].as_i64().unwrap_or(0),
                !f["format_id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with("-drc")),
                f["abr"].as_f64().unwrap_or(0.0) as u64,
            )
        });
    let mut output = Vec::new();
    for video in formats.iter().filter(|f| {
        transport(f)
            && f["height"].as_u64().is_some_and(|h| h > 0)
            && matches!(
                codec(&f["vcodec"]).as_str(),
                "h264" | "hevc" | "vp9" | "av1"
            )
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
            proxy: proxy.clone(),
            segments: Vec::new(),
        };
        source.validate().map_err(|_| {
            ApiError::conflict("The extractor returned an unsupported media address")
        })?;
        let frame_rate = video["fps"].as_f64().filter(|f| f.is_finite() && *f > 0.0);
        let bitrate = video["tbr"]
            .as_f64()
            .map(|rate| (rate + audio.and_then(|f| f["abr"].as_f64()).unwrap_or(0.0)) * 1000.0)
            .filter(|r| r.is_finite() && *r > 0.0)
            .map(|r| r as u64);
        let sound = audio.unwrap_or(video);
        output.push(Format {
            native: !live
                && video["protocol"] == "https"
                && matches!(video["ext"].as_str(), Some("mp4" | "webm"))
                && audio.is_none_or(|f| f["protocol"] == "https"),
            indexed: !live
                && video["protocol"] == "https"
                && video["ext"] == "mp4"
                && audio.is_some_and(|f| {
                    f["protocol"] == "https" && f["ext"] == "m4a" && codec(&f["acodec"]) == "aac"
                }),
            source,
            height: video["height"].as_u64().unwrap(),
            width: video["width"].as_u64(),
            frame_rate,
            bitrate,
            fps: frame_rate.unwrap_or(30.0).round() as u64,
            video: codec(&video["vcodec"]),
            audio: codec(&sound["acodec"]),
            codecs: [&video["vcodec"], &sound["acodec"]]
                .map(|c| c.as_str().unwrap_or_default().to_owned()),
            audio_bitrate: sound["abr"]
                .as_f64()
                .filter(|r| r.is_finite() && *r > 0.0)
                .map(|r| (r * 1000.0) as u64),
            sample_rate: sound["asr"].as_u64(),
            channels: sound["audio_channels"].as_u64(),
            available_at: [video, sound]
                .iter()
                .filter_map(|f| f["available_at"].as_i64())
                .max()
                .unwrap_or(0),
        });
    }
    // At one resolution and frame rate, the most efficient encoding of the same
    // source starts and seeks fastest: AV1 is about half the size of H.264.
    let efficiency = |f: &Format| match f.video.as_str() {
        "av1" => 3,
        "vp9" => 2,
        "h264" => 1,
        _ => 0,
    };
    output.sort_by_key(|f| (f.height, f.fps, efficiency(f), f.indexed, f.native));
    if output.is_empty() {
        return Err(ApiError::conflict(
            "No supported public video stream is available",
        ));
    }
    Ok(output)
}
pub(super) fn choices<'a>(formats: impl IntoIterator<Item = &'a Format>) -> Vec<Value> {
    let mut choices: Vec<Value> = Vec::new();
    for f in formats {
        let quality = f.quality();
        if !choices.iter().any(|v| v["value"] == quality) {
            choices.insert(0, json!({"value":quality,"label":quality}));
        }
    }
    choices
}
/// The original rendition for a requested quality among ascending `formats`:
/// the best for Auto, the best within a bitrate or resolution, else the lowest.
pub(super) fn pick<'a>(formats: &[&'a Format], quality: &str) -> Result<&'a Format> {
    let last = formats
        .last()
        .copied()
        .ok_or_else(|| ApiError::conflict("This resolution is not available for this player"))?;
    if quality == "auto" || quality == "original" {
        return Ok(last);
    }
    let within = |fits: &dyn Fn(&Format) -> bool| {
        formats
            .iter()
            .rev()
            .find(|f| fits(f))
            .or(formats.first())
            .copied()
    };
    if let Some(rate) = thelxinoe_playback::bitrate(quality)
        .ok()
        .flatten()
        .filter(|_| quality.ends_with("mbps"))
    {
        return Ok(within(&|f| f.bitrate.is_some_and(|b| b <= u64::from(rate))).unwrap());
    }
    if let Some(format) = formats.iter().rev().find(|f| f.quality() == quality) {
        return Ok(format);
    }
    let height = thelxinoe_playback::resolution(quality)
        .ok_or_else(|| ApiError::conflict("This resolution is not available for this player"))?;
    Ok(within(&|f| f.height <= u64::from(height)).unwrap())
}
pub(super) fn choose<'a>(
    formats: &'a [Format],
    quality: &str,
    caps: &Capabilities,
) -> Result<&'a Format> {
    let supported: Vec<_> = formats.iter().filter(|f| f.supported(caps)).collect();
    if !supported.is_empty() {
        return pick(&supported, quality);
    }
    // If no original rendition is compatible, retain the requested provider
    // quality and let the shared planner convert only incompatible tracks.
    if !caps.hls {
        return Err(ApiError::conflict(
            "This resolution is not available for this player",
        ));
    }
    pick(&formats.iter().collect::<Vec<_>>(), quality)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> Value {
        let mut formats = vec![
            json!({"format_id":"140-drc","vcodec":"none","acodec":"mp4a.40.2","protocol":"https","ext":"m4a","url":"https://r1.googlevideo.com/drc","abr":129}),
            json!({"format_id":"140","vcodec":"none","acodec":"mp4a.40.2","protocol":"https","ext":"m4a","url":"https://r1.googlevideo.com/audio","abr":129,"asr":44100,"audio_channels":2,"available_at":1000}),
            json!({"format_id":"140-1","vcodec":"none","acodec":"mp4a.40.2","protocol":"https","ext":"m4a","url":"https://r1.googlevideo.com/dubbed","abr":130,"language_preference":-1}),
            json!({"format_id":"251","vcodec":"none","acodec":"opus","protocol":"https","ext":"webm","url":"https://r1.googlevideo.com/opus","abr":160}),
        ];
        formats[1]["language_preference"] = json!(10);
        for (height, fps, codec, ext, rate) in [
            (360, 30, "avc1.4D401E", "mp4", 300),
            (720, 60, "avc1.64002A", "mp4", 2000),
            (1080, 30, "avc1.640028", "mp4", 3000),
            (1080, 30, "av01.0.08M.08", "mp4", 1400),
            (1080, 30, "vp9", "webm", 2500),
            (2160, 60, "vp9", "webm", 17000),
            (2160, 60, "av01.0.13M.08", "mp4", 9000),
        ] {
            formats.push(json!({"vcodec":codec,"acodec":"none","protocol":"https","url":"https://r1.googlevideo.com/video?private=1","ext":ext,"height":height,"width":height*16/9,"fps":fps,"tbr":rate,"available_at":900}));
        }
        formats.push(json!({"vcodec":"avc1.640028","acodec":"none","protocol":"m3u8_native","url":"https://r1.googlevideo.com/playlist.m3u8","ext":"mp4","height":1080,"fps":30}));
        json!({"duration":60,"formats":formats})
    }
    #[test]
    fn original_renditions_respect_codecs_and_select_original_audio() {
        let formats = extract(&metadata(), Some("http://127.0.0.1:41000".into())).unwrap();
        assert!(
            formats
                .iter()
                .all(|f| f.source.proxy.as_deref() == Some("http://127.0.0.1:41000"))
        );
        // Recorded videos are separate files; their segmented playlists are unused.
        assert!(formats.iter().all(|f| !f.source.video.contains("m3u8")));
        assert!(formats.iter().all(|f| f.source.audio.as_deref()
            == Some("https://r1.googlevideo.com/audio")
            && f.available_at == 1000));
        let mut caps = Capabilities {
            video: vec!["h264".into()],
            audio: vec!["aac".into()],
            hls: true,
            ..Default::default()
        };
        let supported: Vec<_> = formats.iter().filter(|f| f.supported(&caps)).collect();
        let menu = choices(supported.iter().copied());
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
        // Audio counts toward a bitrate ceiling.
        assert_eq!(choose(&formats, "2mbps", &caps).unwrap().height, 360);
        assert_eq!(choose(&formats, "4mbps", &caps).unwrap().height, 1080);
        assert_eq!(choose(&formats, "240p", &caps).unwrap().height, 360);
        assert!(choose(&formats, "https://elsewhere/video", &caps).is_err());
        caps.video.push("vp9".into());
        assert_eq!(
            choose(&formats, "2160p60", &caps).unwrap().source.video,
            "https://r1.googlevideo.com/video?private=1"
        );
        assert_eq!(choose(&formats, "1080p", &caps).unwrap().video, "vp9");
        caps.video.push("av1".into());
        let best = choose(&formats, "auto", &caps).unwrap();
        assert_eq!((best.height, best.video.as_str()), (2160, "av1"));
        // The smallest encoding of a resolution is preferred.
        assert_eq!(choose(&formats, "1080p", &caps).unwrap().video, "av1");
        // Only MP4 renditions with AAC audio play as byte-range HLS.
        assert!(best.indexed && best.indexed_for(&caps));
        assert!(
            formats
                .iter()
                .filter(|f| f.indexed)
                .all(|f| f.codecs[0] != "vp9")
        );
        caps.hls_media = Some(vec!["video/mp4; codecs=\"av01.0.08M.08\"".into()]);
        assert!(!best.indexed_for(&caps));
        caps.hls_media = Some(vec!["video/mp4; codecs=\"AV01.0.13M.08\"".into()]);
        assert!(best.indexed_for(&caps));
        caps.hls_container = Some("ts".into());
        assert!(!best.indexed_for(&caps));
    }
    #[test]
    fn probes_describe_provider_codecs() {
        let formats = extract(&metadata(), None).unwrap();
        let h264 = formats.iter().find(|f| f.height == 720).unwrap();
        let probe = h264.probe(60.0);
        assert_eq!(probe["format"]["duration"], "60");
        assert_eq!(probe["format"]["bit_rate"], "2129000");
        assert_eq!(probe["streams"][0]["profile"], "High");
        assert_eq!(probe["streams"][0]["level"], 42);
        assert_eq!(probe["streams"][0]["avg_frame_rate"], "60/1");
        assert_eq!(probe["streams"][0]["width"], 1280);
        assert_eq!(probe["streams"][1]["profile"], "LC");
        assert_eq!(probe["streams"][1]["sample_rate"], "44100");
        assert_eq!(probe["streams"][1]["channels"], 2);
        assert_eq!(
            thelxinoe_playback::codec_string(&probe["streams"][0]),
            "avc1.64002A"
        );
        let main = formats
            .iter()
            .find(|f| f.height == 360)
            .unwrap()
            .probe(60.0);
        assert_eq!(main["streams"][0]["profile"], "Main");
        assert_eq!(main["streams"][0]["level"], 30);
        let av1 = formats
            .iter()
            .find(|f| f.video == "av1" && f.height == 2160)
            .unwrap()
            .probe(60.0);
        assert_eq!(av1["streams"][0]["profile"], "Main");
        assert_eq!(av1["streams"][0]["level"], 13);
        assert_eq!(av1["streams"][0]["pix_fmt"], "yuv420p");
    }
}
