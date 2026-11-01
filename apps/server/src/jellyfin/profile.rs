//! Translate the tested Jellyfin device profile into supported delivery paths.
use crate::error::ApiError;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use thelxinoe_playback::{
    Capabilities, Conversion, Options, Source, conversion_rate, level_rate, picture_rate,
};

type Properties = BTreeMap<String, String>;
pub fn matches(value: &Value, wanted: &str) -> bool {
    value
        .as_str()
        .is_none_or(|v| v.is_empty() || v.split(',').any(|s| s.trim().eq_ignore_ascii_case(wanted)))
}
fn text(value: &Value) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string()),
        )
    }
}
fn stream<'a>(source: &'a Source, kind: &str, index: Option<i64>) -> Option<&'a Value> {
    source.probe["streams"].as_array()?.iter().find(|s| {
        s["codec_type"] == kind
            && s["disposition"]["attached_pic"] != 1
            && index.is_none_or(|i| s["index"].as_i64() == Some(i))
    })
}
/// Jellyfin's `VideoRange` and `VideoRangeType` of a video stream. A Dolby Vision stream
/// is named by the base layer that players without Dolby Vision show instead.
pub fn video_range(video: &Value) -> (&'static str, &'static str) {
    let transfer = video["color_transfer"].as_str().unwrap_or("");
    let pq = transfer == "smpte2084";
    let hlg = transfer == "arib-std-b67";
    let base = if pq || hlg { "HDR" } else { "SDR" };
    let dovi = video["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| {
            s["side_data_type"]
                .as_str()
                .is_some_and(|v| v.contains("DOVI"))
        })
        .filter(|s| {
            s["rpu_present_flag"] == 1
                && s["bl_present_flag"] == 1
                && s["dv_profile"]
                    .as_i64()
                    .is_some_and(|p| [5, 7, 8, 10].contains(&p))
                && s["dv_bl_signal_compatibility_id"]
                    .as_i64()
                    .is_some_and(|id| [0, 1, 2, 4, 6].contains(&id))
        });
    let Some(dovi) = dovi else {
        return if pq {
            ("HDR", "HDR10")
        } else if hlg {
            ("HDR", "HLG")
        } else {
            ("SDR", "SDR")
        };
    };
    let range = match (
        dovi["dv_profile"].as_i64(),
        dovi["dv_bl_signal_compatibility_id"].as_i64(),
    ) {
        (Some(5), _) | (Some(10), Some(0)) => ("HDR", "DOVI"),
        (Some(7), _) => ("HDR", "DOVIWithEL"),
        (Some(8 | 10), Some(1)) => ("HDR", "DOVIWithHDR10"),
        (Some(8 | 10), Some(2)) => ("SDR", "DOVIWithSDR"),
        (Some(8 | 10), Some(4)) => ("HDR", "DOVIWithHLG"),
        _ => (base, "DOVIInvalid"),
    };
    let tagged = |transfer: &str| {
        video["color_space"] == "bt2020nc"
            && video["color_transfer"] == transfer
            && video["color_primaries"] == "bt2020"
    };
    match range.1 {
        "DOVIWithHDR10" | "DOVIWithEL" if !tagged("smpte2084") => (base, "DOVIInvalid"),
        "DOVIWithHLG" if !tagged("arib-std-b67") => (base, "DOVIInvalid"),
        _ => range,
    }
}
fn properties(source: &Source, audio: Option<i64>) -> Properties {
    let mut result = Properties::new();
    let video = stream(source, "video", None).unwrap_or(&Value::Null);
    let audio = stream(source, "audio", audio).unwrap_or(&Value::Null);
    for (name, probe, key) in [
        ("Width", video, "width"),
        ("Height", video, "height"),
        ("VideoLevel", video, "level"),
        ("VideoProfile", video, "profile"),
        ("VideoBitrate", video, "bit_rate"),
        ("RefFrames", video, "refs"),
        ("VideoCodecTag", video, "codec_tag_string"),
        ("IsAvc", video, "is_avc"),
        ("AudioChannels", audio, "channels"),
        ("AudioProfile", audio, "profile"),
        ("AudioBitrate", audio, "bit_rate"),
        ("AudioSampleRate", audio, "sample_rate"),
        ("AudioBitDepth", audio, "bits_per_raw_sample"),
    ] {
        if let Some(value) = text(&probe[key]) {
            result.insert(name.into(), value);
        }
    }
    if !video.is_null() {
        let depth = video["bits_per_raw_sample"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .unwrap_or_else(|| {
                let pixel = video["pix_fmt"].as_str().unwrap_or("");
                if pixel.contains("12") {
                    12
                } else if pixel.contains("10") {
                    10
                } else {
                    8
                }
            });
        result.insert("VideoBitDepth".into(), depth.to_string());
        result.insert("VideoRangeType".into(), video_range(video).1.into());
        result.insert(
            "IsInterlaced".into(),
            (!["progressive", "unknown", ""]
                .contains(&video["field_order"].as_str().unwrap_or("")))
            .to_string(),
        );
        result.insert(
            "IsAnamorphic".into(),
            (!["1:1", "0:1", ""].contains(&video["sample_aspect_ratio"].as_str().unwrap_or("")))
                .to_string(),
        );
        if let Some(rate) = video["avg_frame_rate"]
            .as_str()
            .and_then(|s| s.split_once('/'))
            .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
            .filter(|n| n.is_finite() && *n > 0.0)
        {
            result.insert("VideoFramerate".into(), rate.to_string());
        }
    }
    for (name, kind) in [("NumAudioStreams", "audio"), ("NumVideoStreams", "video")] {
        result.insert(
            name.into(),
            source.probe["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["codec_type"] == kind && s["disposition"]["attached_pic"] != 1)
                .count()
                .to_string(),
        );
    }
    result.insert("IsSecondaryAudio".into(), "false".into());
    result
}
fn condition(condition: &Value, properties: &Properties) -> bool {
    let name = condition["Property"].as_str().unwrap_or("");
    let Some(actual) = properties.get(name) else {
        return condition["IsRequired"] == false;
    };
    let expected = text(&condition["Value"]).unwrap_or_default();
    let numeric = actual
        .parse::<f64>()
        .ok()
        .zip(expected.parse::<f64>().ok())
        .filter(|(a, b)| a.is_finite() && b.is_finite());
    match condition["Condition"].as_str().unwrap_or("") {
        "Equals" => numeric.map_or_else(|| actual.eq_ignore_ascii_case(&expected), |(a, b)| a == b),
        "NotEquals" => !expected.split('|').any(|v| actual.eq_ignore_ascii_case(v)),
        "EqualsAny" => expected.split('|').any(|v| actual.eq_ignore_ascii_case(v)),
        "LessThanEqual" => numeric.is_some_and(|(a, b)| a <= b),
        "GreaterThanEqual" => numeric.is_some_and(|(a, b)| a >= b),
        _ => false,
    }
}
fn constraints(
    profile: &Value,
    container: &str,
    video: &str,
    audio: &str,
    values: &Properties,
) -> bool {
    for rule in profile["CodecProfiles"].as_array().into_iter().flatten() {
        let codec = match rule["Type"].as_str() {
            Some("Video") if !video.is_empty() => video,
            Some("Audio") if video.is_empty() => audio,
            Some("VideoAudio") if !video.is_empty() => audio,
            _ => continue,
        };
        if codec.is_empty()
            || !matches(&rule["Codec"], codec)
            || !matches(&rule["Container"], container)
        {
            continue;
        }
        if !rule["ApplyConditions"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|c| condition(c, values))
        {
            continue;
        }
        if !rule["Conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|c| condition(c, values))
        {
            return false;
        }
    }
    for rule in profile["ContainerProfiles"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if matches(&rule["Container"], container)
            && !rule["Conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|c| condition(c, values))
        {
            return false;
        }
    }
    true
}
fn hls(
    profile: &Value,
    kind: &str,
    video: &str,
    audio: &str,
    values: &Properties,
    container: &str,
) -> bool {
    if container == "ts"
        && !video.is_empty()
        && !["h264", "hevc", "mpeg2video", "mpeg4", "vc1"].contains(&video)
    {
        return false;
    }
    profile["TranscodingProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| {
            p["Type"] == kind
                && p["Protocol"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case("hls"))
                && matches(&p["Container"], container)
                && matches(&p["VideoCodec"], video)
                && matches(&p["AudioCodec"], audio)
                && p["Conditions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|c| condition(c, values))
                && p["MaxAudioChannels"]
                    .as_str()
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_none_or(|max| {
                        values
                            .get("AudioChannels")
                            .and_then(|v| v.parse::<u32>().ok())
                            .is_some_and(|channels| channels <= max)
                    })
        })
}
/// Jellyfin's delivery choice for a subtitle stored in a direct-played file: the client
/// reads the track itself when one of its Embed profiles names the track's format, or
/// any format a text track converts to.
pub fn embeds_subtitle(profile: &Value, codec: &str, language: &str, container: &str) -> bool {
    // Jellyfin's names for FFprobe's bitmap subtitle codecs.
    let codec = match codec {
        "dvb_subtitle" => "dvbsub",
        "dvb_teletext" => "dvbtxt",
        "dvd_subtitle" => "dvdsub",
        "hdmv_pgs_subtitle" => "pgssub",
        codec => codec,
    };
    let text_format = |format: &str| {
        let format = format.to_ascii_lowercase();
        format.contains("microdvd")
            || !(["pgs", "dvdsub", "vobsub", "dvbsub"]
                .iter()
                .any(|bitmap| format.contains(bitmap))
                || ["sup", "sub"].contains(&format.as_str()))
    };
    let styled = |format: &str| {
        ["ass", "ssa"]
            .iter()
            .any(|s| format.eq_ignore_ascii_case(s))
    };
    let text = !codec.is_empty() && text_format(codec);
    profile["SubtitleProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["Method"] == "Embed"
                && matches(&p["Container"], container)
                && matches(&p["Language"], language)
        })
        .filter_map(|p| p["Format"].as_str())
        .any(|format| {
            (format.eq_ignore_ascii_case(codec) && text == text_format(format))
                || (text && !styled(codec) && !styled(format))
        })
}
/// The highest conversion quality whose video and 192 kbps audio fit a client's bitrate limit.
pub fn quality_within(max: i64) -> Option<&'static str> {
    [
        (20_192_000, "20mbps"),
        (8_192_000, "8mbps"),
        (4_192_000, "4mbps"),
        (2_192_000, "2mbps"),
    ]
    .into_iter()
    .find(|(least, _)| max >= *least)
    .map(|(_, quality)| quality)
}
/// The quality that keeps a planned video conversion and its audio within the client's limit,
/// if the requested one doesn't. Auto converts at the rate of its picture, which a source
/// within the limit can still exceed.
pub fn bounded_quality(
    source: &Source,
    options: &Options,
    max: Option<i64>,
) -> crate::error::Result<Option<&'static str>> {
    let Some(max) = max else {
        return Ok(None);
    };
    let converts =
        thelxinoe_playback::decision(source, options).is_ok_and(|planned| planned.encoding.video);
    let rate = conversion_rate(options, &source.probe).map_err(|e| ApiError::bad(e.to_string()))?;
    if !converts || i64::from(rate) + 192_000 <= max {
        return Ok(None);
    }
    quality_within(max).map(Some).ok_or_else(|| {
        ApiError::bad("This client bitrate limit is below the supported conversion quality")
    })
}
/// The lowest H.264 level that holds a picture, by its macroblocks per frame and per second.
fn level(width: u32, height: u32, frame_rate: f64) -> Option<u32> {
    let blocks = u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16));
    [
        (30, 1_620, 40_500),
        (31, 3_600, 108_000),
        (32, 5_120, 216_000),
        (41, 8_192, 245_760),
        (42, 8_704, 522_240),
        (50, 22_080, 589_824),
        (51, 36_864, 983_040),
        (52, 36_864, 2_073_600),
        (60, 139_264, 4_177_920),
        (61, 139_264, 8_355_840),
        (62, 139_264, 16_711_680),
    ]
    .into_iter()
    .find(|(_, frame, second)| blocks <= *frame && blocks as f64 * frame_rate <= *second as f64)
    .map(|(level, ..)| level)
}
pub fn negotiate(
    source: &Source,
    audio_index: Option<i64>,
    profile: &Value,
    direct: bool,
    remux: bool,
    video_bitrate: Option<u32>,
) -> Capabilities {
    negotiate_for_container(
        source,
        audio_index,
        profile,
        direct,
        remux,
        video_bitrate,
        "ts",
    )
}
pub fn negotiate_online(
    source: &Source,
    audio_index: Option<i64>,
    profile: &Value,
    direct: bool,
    remux: bool,
    video_bitrate: Option<u32>,
) -> Capabilities {
    let negotiate = |profile: &Value, container| {
        negotiate_for_container(
            source,
            audio_index,
            profile,
            direct,
            remux,
            video_bitrate,
            container,
        )
    };
    let score = |caps: &Capabilities| {
        thelxinoe_playback::decision(
            source,
            &thelxinoe_playback::Options {
                quality: "auto".into(),
                audio: audio_index,
                subtitle: None,
                capabilities: caps.clone(),
            },
        )
        .map(|d| u8::from(d.encoding.video) * 2 + u8::from(d.encoding.audio))
        .unwrap_or(u8::MAX)
    };
    let declared = ["mp4", "ts"]
        .map(|container| negotiate(profile, container))
        .into_iter()
        .min_by_key(score)
        .unwrap();
    let declared_score = score(&declared);
    // Recorded originals are MP4 files, whose own fragments reach a fragmented MP4 HLS client
    // without FFmpeg.
    let fragments = source.path.extension().is_some_and(|e| e == "mp4");
    if declared_score < 2 && (!fragments || declared.hls_container.as_deref() == Some("mp4")) {
        return declared;
    }
    // A client that plays a codec from MP4 files also plays it as fragmented MP4 HLS, which
    // keeps an original its streaming profiles omit, such as AV1 for MPEG-TS-only clients.
    let mut files = profile.clone();
    files["TranscodingProfiles"] = profile["DirectPlayProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| matches(&p["Container"], "mp4"))
        .map(|p| json!({"Type":p["Type"],"Protocol":"hls","Container":"mp4","VideoCodec":p["VideoCodec"],"AudioCodec":p["AudioCodec"]}))
        .collect();
    let fragmented = negotiate(&files, "mp4");
    let fragmented_score = score(&fragmented);
    if fragmented_score < 2 && fragmented_score <= declared_score {
        fragmented
    } else {
        declared
    }
}
fn negotiate_for_container(
    source: &Source,
    audio_index: Option<i64>,
    profile: &Value,
    direct: bool,
    remux: bool,
    video_bitrate: Option<u32>,
    output_container: &str,
) -> Capabilities {
    let video = source.video_codec().unwrap_or("");
    let audio = stream(source, "audio", audio_index)
        .and_then(|s| s["codec_name"].as_str())
        .unwrap_or("");
    let container = source
        .path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = if video.is_empty() { "Audio" } else { "Video" };
    let values = properties(source, audio_index);
    let direct = direct
        && constraints(profile, &container, video, audio, &values)
        && profile["DirectPlayProfiles"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|p| {
                p["Type"] == kind
                    && matches(&p["Container"], &container)
                    && matches(&p["VideoCodec"], video)
                    && matches(&p["AudioCodec"], audio)
            });
    let allow_remux = remux;
    let mut conversion = None;
    let mut converted = values.clone();
    for (key, value) in [
        ("AudioChannels", "2"),
        ("AudioProfile", "LC"),
        ("AudioBitrate", "192000"),
        ("AudioSampleRate", "48000"),
        ("AudioBitDepth", "16"),
        ("NumAudioStreams", "1"),
        ("VideoBitDepth", "8"),
        ("VideoRangeType", "SDR"),
        ("IsInterlaced", "false"),
        ("IsAnamorphic", "false"),
        ("RefFrames", "1"),
        ("IsAvc", "false"),
        ("NumVideoStreams", if video.is_empty() { "0" } else { "1" }),
    ] {
        converted.insert(key.into(), value.into());
    }
    let range = values.get("VideoRangeType").map_or("", String::as_str);
    // Without a standard base layer, FFmpeg can't reproduce the picture.
    if !video.is_empty() && !["DOVI", "DOVIInvalid"].contains(&range) {
        let upper = |name: &str, fallback: f64| {
            profile["CodecProfiles"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| {
                    p["Type"] == "Video"
                        && matches(&p["Codec"], "h264")
                        && matches(&p["Container"], "ts")
                })
                .flat_map(|p| p["Conditions"].as_array().into_iter().flatten())
                .filter(|c| c["Property"] == name && c["Condition"] == "LessThanEqual")
                .filter_map(|c| text(&c["Value"]).and_then(|s| s.parse::<f64>().ok()))
                .filter(|n| n.is_finite() && *n > 0.0)
                .fold(fallback, f64::min)
        };
        let aspect = stream(source, "video", None)
            .and_then(|v| v["sample_aspect_ratio"].as_str())
            .and_then(|s| s.split_once(':'))
            .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(1.0);
        let width = values
            .get("Width")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1920.0)
            * aspect;
        let height = values
            .get("Height")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1080.0);
        let source_rate = values
            .get("VideoFramerate")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(30.0)
            .clamp(1.0, 60.0);
        // The source picture when the client takes it, else the largest smaller one it does.
        'candidate: for ((max_width, max_height), most) in [
            (7680.0, 4320.0),
            (3840.0, 2160.0),
            (2560.0, 1440.0),
            (1920.0, 1080.0),
            (1280.0, 720.0),
            (720.0, 480.0),
        ]
        .into_iter()
        .flat_map(|size| [(size, 60.0), (size, 30.0)])
        {
            let frame_rate = source_rate.min(most).min(upper("VideoFramerate", 60.0));
            let scale = (upper("Width", max_width) / width)
                .min(upper("Height", max_height) / height)
                .min(1.0);
            let width = ((width * scale / 2.0).floor().max(1.0) as u32) * 2;
            let height = ((height * scale / 2.0).floor().max(1.0) as u32) * 2;
            let Some(level) = level(width, height, frame_rate) else {
                continue;
            };
            converted.insert("Width".into(), width.to_string());
            converted.insert("Height".into(), height.to_string());
            converted.insert("VideoLevel".into(), level.to_string());
            converted.insert(
                "VideoBitrate".into(),
                video_bitrate
                    .unwrap_or_else(|| picture_rate(width.into(), height.into()))
                    .min(level_rate(level))
                    .to_string(),
            );
            converted.insert("VideoFramerate".into(), frame_rate.to_string());
            for (name, encoded) in [
                ("High", "high"),
                ("Main", "main"),
                ("Constrained Baseline", "baseline"),
            ] {
                converted.insert("VideoProfile".into(), name.into());
                if hls(profile, kind, "h264", "aac", &converted, output_container)
                    && constraints(profile, output_container, "h264", "aac", &converted)
                {
                    conversion = Some(Conversion {
                        width,
                        height,
                        frame_rate,
                        profile: encoded.into(),
                        level,
                        tone_map: ["HDR10", "HLG", "DOVIWithHDR10", "DOVIWithHLG", "DOVIWithEL"]
                            .contains(&range),
                        deinterlace: values.get("IsInterlaced").is_some_and(|v| v == "true"),
                        fit: false,
                    });
                    break 'candidate;
                }
            }
        }
    }
    let can_convert = if video.is_empty() {
        hls(profile, kind, "", "aac", &converted, output_container)
            && constraints(profile, output_container, "", "aac", &converted)
    } else {
        conversion.is_some()
    };
    // Whether the client takes the video copied, with the original audio or converted audio.
    let copies = |values: &Properties| {
        let mut audio_converted = values.clone();
        for key in [
            "AudioChannels",
            "AudioProfile",
            "AudioBitrate",
            "AudioSampleRate",
            "AudioBitDepth",
            "NumAudioStreams",
        ] {
            if let Some(value) = converted.get(key) {
                audio_converted.insert(key.into(), value.clone());
            }
        }
        (
            allow_remux
                && hls(profile, kind, video, audio, values, output_container)
                && constraints(profile, output_container, video, audio, values),
            allow_remux
                && !video.is_empty()
                && hls(
                    profile,
                    kind,
                    video,
                    "aac",
                    &audio_converted,
                    output_container,
                )
                && constraints(profile, output_container, video, "aac", &audio_converted),
        )
    };
    let (mut remux, mut copy_video) = copies(&values);
    // A client that refuses Dolby Vision can still take the base layer beneath it.
    let base = match range {
        "DOVIWithHDR10" | "DOVIWithEL" => Some("HDR10"),
        "DOVIWithHLG" => Some("HLG"),
        "DOVIWithSDR" => Some("SDR"),
        _ => None,
    }
    .filter(|_| ["hevc", "av1"].contains(&video));
    let mut strip_dolby_vision = false;
    if !remux
        && !copy_video
        && let Some(base) = base
    {
        let mut stripped = values.clone();
        stripped.insert("VideoRangeType".into(), base.into());
        (remux, copy_video) = copies(&stripped);
        strip_dolby_vision = remux || copy_video;
    }
    let mut video_converted = converted.clone();
    for key in [
        "AudioChannels",
        "AudioProfile",
        "AudioBitrate",
        "AudioSampleRate",
        "AudioBitDepth",
        "NumAudioStreams",
    ] {
        if let Some(value) = values.get(key) {
            video_converted.insert(key.into(), value.clone());
        } else {
            video_converted.remove(key);
        }
    }
    let copy_audio = allow_remux
        && can_convert
        && hls(
            profile,
            kind,
            if video.is_empty() { "" } else { "h264" },
            audio,
            &video_converted,
            output_container,
        )
        && constraints(
            profile,
            output_container,
            if video.is_empty() { "" } else { "h264" },
            audio,
            &video_converted,
        );
    Capabilities {
        hls_container: Some(output_container.into()),
        containers: if direct { vec![container] } else { vec![] },
        video: if direct || remux || copy_video {
            vec![video.into()]
        } else {
            vec![]
        },
        audio: if direct || remux || copy_audio {
            vec![audio.into()]
        } else {
            vec![]
        },
        hls: remux || can_convert,
        native_tracks: true,
        conversion,
        native_remote: false,
        hls_video: Some(if remux || copy_video {
            vec![video.into()]
        } else {
            vec![]
        }),
        hls_audio: Some(if remux || copy_audio {
            vec![audio.into()]
        } else {
            vec![]
        }),
        supported_media: None,
        hls_media: None,
        allow_conversion: None,
        strip_dolby_vision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use thelxinoe_playback::{Options, plan};
    fn fixture() -> (Source, Value) {
        let source = Source {
            id: "f".into(),
            media_id: "m".into(),
            generation: "1".into(),
            edition: String::new(),
            path: "movie.mkv".into(),
            root: "/".into(),
            size: 1,
            modified: "1".into(),
            probe: json!({"streams":[{"index":0,"codec_type":"video","codec_name":"h264","profile":"High 10","level":51,"width":3840,"height":2160,"avg_frame_rate":"60/1","bits_per_raw_sample":"10","refs":4},{"index":1,"codec_type":"audio","codec_name":"aac","profile":"LC","channels":6,"sample_rate":"48000"}]}),
        };
        let profile = json!({"DirectPlayProfiles":[{"Type":"Video","Container":"mkv","VideoCodec":"h264","AudioCodec":"aac"}],"TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}],"CodecProfiles":[{"Type":"Video","Codec":"h264","Conditions":[{"Property":"VideoBitDepth","Condition":"LessThanEqual","Value":"8","IsRequired":true},{"Property":"VideoLevel","Condition":"LessThanEqual","Value":"31","IsRequired":true},{"Property":"Width","Condition":"LessThanEqual","Value":"1280","IsRequired":true},{"Property":"Height","Condition":"LessThanEqual","Value":"720","IsRequired":true}]},{"Type":"VideoAudio","Codec":"aac","Conditions":[{"Property":"AudioChannels","Condition":"LessThanEqual","Value":"2","IsRequired":true}]}]});
        (source, profile)
    }
    #[test]
    fn unsupported_depth_resolution_level_and_channels_require_a_supported_conversion() {
        let (source, profile) = fixture();
        let caps = negotiate(&source, Some(1), &profile, true, true, Some(8_000_000));
        let conversion = caps.conversion.as_ref().unwrap();
        assert_eq!(
            (conversion.width, conversion.height, conversion.level),
            (1280, 720, 31)
        );
        assert_eq!(conversion.frame_rate, 30.0);
        let options = Options {
            quality: "auto".into(),
            audio: Some(1),
            subtitle: None,
            capabilities: caps,
        };
        assert_eq!(plan(&source, &options).unwrap(), "transcode");
        let mut denied = profile;
        denied["TranscodingProfiles"][0]["Type"] = json!("Audio");
        let options = Options {
            capabilities: negotiate(&source, Some(1), &denied, true, true, Some(8_000_000)),
            ..options
        };
        assert!(plan(&source, &options).is_err());
    }
    #[test]
    fn conditional_constraints_and_small_resolution_limits_are_respected() {
        let (mut source, mut profile) = fixture();
        source.probe["streams"][0]["bits_per_raw_sample"] = json!("8");
        source.probe["streams"][0]["profile"] = json!("High");
        source.probe["streams"][0]["level"] = json!(31);
        source.probe["streams"][0]["width"] = json!(320);
        source.probe["streams"][0]["height"] = json!(180);
        source.probe["streams"][1]["channels"] = json!(2);
        profile["CodecProfiles"].as_array_mut().unwrap().push(json!({"Type":"Video","Codec":"h264","ApplyConditions":[{"Property":"VideoProfile","Condition":"Equals","Value":"High 10"}],"Conditions":[{"Property":"Width","Condition":"LessThanEqual","Value":"160","IsRequired":true}]}));
        let caps = negotiate(&source, Some(1), &profile, true, true, Some(8_000_000));
        assert_eq!(caps.containers, vec!["mkv"]);
        profile["CodecProfiles"][2]["ApplyConditions"] = json!([]);
        let caps = negotiate(&source, Some(1), &profile, true, true, Some(8_000_000));
        assert!(caps.containers.is_empty());
        let conversion = caps.conversion.unwrap();
        assert_eq!((conversion.width, conversion.height), (160, 90));
    }
    #[test]
    fn mp4_originals_reach_clients_that_play_mp4_files_as_fragments() {
        let (mut source, _) = fixture();
        source.path = "public.mp4".into();
        source.probe["streams"][0] = json!({"index":0,"codec_type":"video","codec_name":"h264","profile":"High","level":40,"width":1920,"height":1080,"avg_frame_rate":"30/1","bits_per_raw_sample":"8"});
        source.probe["streams"][1]["channels"] = json!(2);
        // Wholphin streams MPEG-TS HLS and plays MP4 files.
        let mut profile = json!({"TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}],"DirectPlayProfiles":[{"Type":"Video","Container":"mkv,mp4","VideoCodec":"h264","AudioCodec":"aac"}]});
        let container = |source: &Source, profile: &Value| {
            negotiate_online(source, None, profile, false, true, None).hls_container
        };
        assert_eq!(container(&source, &profile).as_deref(), Some("mp4"));
        let mut live = source.clone();
        live.path = "public.ts".into();
        assert_eq!(container(&live, &profile).as_deref(), Some("ts"));
        profile["DirectPlayProfiles"][0]["Container"] = json!("mkv");
        assert_eq!(container(&source, &profile).as_deref(), Some("ts"));
    }
    #[test]
    fn conversions_stay_within_the_client_bitrate_limit_even_from_a_smaller_source() {
        let (mut source, _) = fixture();
        // A 6 Mbps 4K HEVC source that an H.264 client receives converted.
        source.probe["streams"][0] = json!({"index":0,"codec_type":"video","codec_name":"hevc","profile":"Main","width":3840,"height":2160,"avg_frame_rate":"30/1","bits_per_raw_sample":"8"});
        source.probe["format"] = json!({"bit_rate":"6000000","duration":"60"});
        let profile = json!({"DirectPlayProfiles":[{"Type":"Video","Container":"mkv","VideoCodec":"h264","AudioCodec":"aac"}],"TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}]});
        let options = |source: &Source, quality: &str| Options {
            quality: quality.into(),
            audio: Some(1),
            subtitle: None,
            capabilities: negotiate(
                source,
                Some(1),
                &profile,
                true,
                true,
                thelxinoe_playback::bitrate(quality).unwrap(),
            ),
        };
        let auto = options(&source, "auto");
        assert_eq!(conversion_rate(&auto, &source.probe).unwrap(), 25_000_000);
        assert_eq!(bounded_quality(&source, &auto, None).unwrap(), None);
        assert_eq!(
            bounded_quality(&source, &auto, Some(30_000_000)).unwrap(),
            None
        );
        assert_eq!(
            bounded_quality(&source, &auto, Some(10_000_000)).unwrap(),
            Some("8mbps")
        );
        let bounded = options(&source, "8mbps");
        assert_eq!(plan(&source, &bounded).unwrap(), "transcode");
        assert_eq!(conversion_rate(&bounded, &source.probe).unwrap(), 8_000_000);
        assert!(bounded_quality(&source, &auto, Some(1_000_000)).is_err());
        // An original the client plays directly needs no bound.
        source.probe["streams"][0]["codec_name"] = json!("h264");
        let direct = options(&source, "auto");
        assert_eq!(plan(&source, &direct).unwrap(), "direct");
        assert_eq!(
            bounded_quality(&source, &direct, Some(1_000_000)).unwrap(),
            None
        );
    }
    fn dolby_vision(profile: i64, compatibility: i64, transfer: &str) -> Value {
        json!({"index":0,"codec_type":"video","codec_name":"hevc","profile":"Main 10","level":150,"width":3840,"height":2160,"avg_frame_rate":"24000/1001","pix_fmt":"yuv420p10le","color_space":"bt2020nc","color_transfer":transfer,"color_primaries":"bt2020","side_data_list":[{"side_data_type":"DOVI configuration record","dv_profile":profile,"dv_bl_signal_compatibility_id":compatibility,"rpu_present_flag":1,"bl_present_flag":1,"el_present_flag":0}]})
    }
    #[test]
    fn dolby_vision_is_named_by_its_base_layer() {
        let pq = "smpte2084";
        let hlg = "arib-std-b67";
        for (stream, expected) in [
            (dolby_vision(8, 1, pq), ("HDR", "DOVIWithHDR10")),
            (dolby_vision(10, 1, pq), ("HDR", "DOVIWithHDR10")),
            (dolby_vision(8, 4, hlg), ("HDR", "DOVIWithHLG")),
            (dolby_vision(8, 2, "bt709"), ("SDR", "DOVIWithSDR")),
            (dolby_vision(7, 6, pq), ("HDR", "DOVIWithEL")),
            (dolby_vision(5, 0, "bt709"), ("HDR", "DOVI")),
            (dolby_vision(10, 0, pq), ("HDR", "DOVI")),
            // A base layer whose tags contradict its record, or a compatibility outside the spec.
            (dolby_vision(8, 1, "bt709"), ("SDR", "DOVIInvalid")),
            (dolby_vision(8, 6, pq), ("HDR", "DOVIInvalid")),
            // A record without a playable layer leaves the base picture.
            (
                {
                    let mut s = dolby_vision(8, 1, pq);
                    s["side_data_list"][0]["rpu_present_flag"] = json!(0);
                    s
                },
                ("HDR", "HDR10"),
            ),
            (json!({"color_transfer":hlg}), ("HDR", "HLG")),
            (json!({"color_transfer":"bt709"}), ("SDR", "SDR")),
        ] {
            assert_eq!(video_range(&stream), expected, "{stream}");
        }
    }
    #[test]
    fn hdr10_clients_play_dolby_vision_with_an_hdr10_base_layer() {
        let (mut source, _) = fixture();
        source.probe["streams"][0] = dolby_vision(8, 1, "smpte2084");
        source.probe["streams"][1] =
            json!({"index":1,"codec_type":"audio","codec_name":"eac3","channels":6});
        // Wholphin on an HDR10 display without Dolby Vision.
        let profile = json!({"DirectPlayProfiles":[{"Type":"Video","Container":"mkv","VideoCodec":"hevc,h264","AudioCodec":"eac3,aac"}],"TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac"}],"CodecProfiles":[{"Type":"Video","Codec":"hevc","Conditions":[{"Property":"VideoRangeType","Condition":"NotEquals","Value":"DOVI|DOVIInvalid|DOVIWithEL","IsRequired":false}],"ApplyConditions":[{"Property":"VideoRangeType","Condition":"EqualsAny","Value":"DOVI|DOVIInvalid|DOVIWithEL","IsRequired":false}]}]});
        let caps = negotiate(&source, Some(1), &profile, true, true, None);
        assert_eq!(caps.containers, vec!["mkv"]);
        assert!(caps.conversion.unwrap().tone_map);
        // Without a base layer it shows, the client gets neither the original nor a conversion.
        source.probe["streams"][0] = dolby_vision(5, 0, "bt709");
        let caps = negotiate(&source, Some(1), &profile, true, true, None);
        assert!(caps.containers.is_empty());
        assert!(caps.conversion.is_none());
    }
    #[test]
    fn hdr10_clients_take_dolby_vision_with_an_enhancement_layer_stripped() {
        let (mut source, _) = fixture();
        source.probe["streams"][0] = dolby_vision(7, 6, "smpte2084");
        source.probe["streams"][0]["side_data_list"][0]["el_present_flag"] = json!(1);
        source.probe["streams"][1] =
            json!({"index":1,"codec_type":"audio","codec_name":"truehd","channels":8});
        // Wholphin on an HDR10 display that decodes neither Dolby Vision nor its enhancement layer.
        let mut profile = json!({"DirectPlayProfiles":[{"Type":"Video","Container":"mkv","VideoCodec":"hevc,h264","AudioCodec":"truehd,aac"}],"TranscodingProfiles":[{"Type":"Video","Container":"ts","Protocol":"hls","VideoCodec":"hevc,h264","AudioCodec":"truehd,aac"}],"CodecProfiles":[{"Type":"Video","Codec":"hevc","Conditions":[{"Property":"VideoRangeType","Condition":"NotEquals","Value":"DOVI|DOVIInvalid|DOVIWithEL","IsRequired":false}],"ApplyConditions":[{"Property":"VideoRangeType","Condition":"EqualsAny","Value":"DOVI|DOVIInvalid|DOVIWithEL","IsRequired":false}]}]});
        let options = |caps| Options {
            quality: "auto".into(),
            audio: Some(1),
            subtitle: None,
            capabilities: caps,
        };
        let caps = negotiate(&source, Some(1), &profile, true, true, None);
        assert!(caps.containers.is_empty());
        assert!(caps.strip_dolby_vision);
        assert_eq!(caps.hls_video.as_deref(), Some(&["hevc".to_owned()][..]));
        assert_eq!(plan(&source, &options(caps)).unwrap(), "remux");
        // A client that takes the enhancement layer keeps it.
        profile["CodecProfiles"] = json!([]);
        let caps = negotiate(&source, Some(1), &profile, true, true, None);
        assert_eq!(caps.containers, vec!["mkv"]);
        assert!(!caps.strip_dolby_vision);
        // Profile 5 has no base layer to leave.
        source.probe["streams"][0] = dolby_vision(5, 0, "bt709");
        profile["CodecProfiles"] = json!([{"Type":"Video","Codec":"hevc","Conditions":[{"Property":"VideoRangeType","Condition":"NotEquals","Value":"DOVI","IsRequired":false}],"ApplyConditions":[{"Property":"VideoRangeType","Condition":"EqualsAny","Value":"DOVI","IsRequired":false}]}]);
        let caps = negotiate(&source, Some(1), &profile, true, true, None);
        assert!(!caps.strip_dolby_vision);
        assert!(plan(&source, &options(caps)).is_err());
    }
    #[test]
    fn direct_play_embeds_the_subtitles_a_client_reads_from_the_file() {
        // Wholphin's ExoPlayer profile, PGS and ASS options off.
        let mut profile = json!({"SubtitleProfiles":[]});
        for (format, methods) in [
            ("srt", &["Embed", "External"][..]),
            ("subrip", &["Embed", "External"]),
            ("pgssub", &["Encode"]),
            ("ass", &["Encode"]),
        ] {
            for method in methods {
                profile["SubtitleProfiles"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"Format":format,"Method":method}));
            }
        }
        assert!(embeds_subtitle(&profile, "subrip", "fre", "mkv"));
        // Another text format converts to SRT; a styled or bitmap one does not.
        assert!(embeds_subtitle(&profile, "mov_text", "eng", "mp4"));
        for codec in ["ass", "hdmv_pgs_subtitle", "dvd_subtitle"] {
            assert!(!embeds_subtitle(&profile, codec, "eng", "mkv"), "{codec}");
        }
        profile["SubtitleProfiles"]
            .as_array_mut()
            .unwrap()
            .push(json!({"Format":"pgssub","Method":"Embed"}));
        assert!(embeds_subtitle(&profile, "hdmv_pgs_subtitle", "eng", "mkv"));
        // A profile can limit embedding to some containers and languages.
        let limited = json!({"SubtitleProfiles":[{"Format":"srt","Method":"Embed","Container":"mkv","Language":"eng,fre"}]});
        assert!(embeds_subtitle(&limited, "subrip", "fre", "mkv"));
        assert!(!embeds_subtitle(&limited, "subrip", "fre", "mp4"));
        assert!(!embeds_subtitle(&limited, "subrip", "ger", "mkv"));
        let external = json!({"SubtitleProfiles":[{"Format":"srt","Method":"External"}]});
        assert!(!embeds_subtitle(&external, "subrip", "fre", "mkv"));
        assert!(!embeds_subtitle(&Value::Null, "subrip", "fre", "mkv"));
    }
}
