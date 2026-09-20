//! Translate the tested Jellyfin device profile into supported delivery paths.
use serde_json::Value;
use std::collections::BTreeMap;
use thelxinoe_playback::{Capabilities, Conversion, Source};

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
pub fn video_range(video: &Value) -> &'static str {
    if video["side_data_list"].as_array().is_some_and(|a| {
        a.iter().any(|s| {
            s["side_data_type"]
                .as_str()
                .is_some_and(|v| v.contains("DOVI"))
        })
    }) {
        "DOVI"
    } else if video["color_transfer"] == "smpte2084" {
        "HDR10"
    } else if video["color_transfer"] == "arib-std-b67" {
        "HLG"
    } else {
        "SDR"
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
        result.insert("VideoRangeType".into(), video_range(video).into());
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
fn hls(profile: &Value, kind: &str, video: &str, audio: &str, values: &Properties) -> bool {
    profile["TranscodingProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| {
            p["Type"] == kind
                && p["Protocol"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case("hls"))
                && matches(&p["Container"], "ts")
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
pub fn negotiate(
    source: &Source,
    audio_index: Option<i64>,
    profile: &Value,
    direct: bool,
    remux: bool,
    video_bitrate: u32,
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
    let remux = remux
        && hls(profile, kind, video, audio, &values)
        && constraints(profile, "ts", video, audio, &values);
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
    if !video.is_empty() && values.get("VideoRangeType").is_none_or(|v| v != "DOVI") {
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
        let frame_rate = values
            .get("VideoFramerate")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(30.0)
            .clamp(1.0, 30.0)
            .min(upper("VideoFramerate", 30.0));
        'candidate: for (max_width, max_height, level) in [
            (1920.0, 1080.0, 41),
            (1280.0, 720.0, 31),
            (720.0, 480.0, 30),
        ] {
            let scale = (upper("Width", max_width) / width)
                .min(upper("Height", max_height) / height)
                .min(1.0);
            let width = ((width * scale / 2.0).floor().max(1.0) as u32) * 2;
            let height = ((height * scale / 2.0).floor().max(1.0) as u32) * 2;
            converted.insert("Width".into(), width.to_string());
            converted.insert("Height".into(), height.to_string());
            converted.insert("VideoLevel".into(), level.to_string());
            converted.insert(
                "VideoBitrate".into(),
                video_bitrate
                    .min(match level {
                        30 => 10_000_000,
                        31 => 14_000_000,
                        _ => 20_000_000,
                    })
                    .to_string(),
            );
            converted.insert("VideoFramerate".into(), frame_rate.to_string());
            for (name, encoded) in [
                ("High", "high"),
                ("Main", "main"),
                ("Constrained Baseline", "baseline"),
            ] {
                converted.insert("VideoProfile".into(), name.into());
                if hls(profile, kind, "h264", "aac", &converted)
                    && constraints(profile, "ts", "h264", "aac", &converted)
                {
                    conversion = Some(Conversion {
                        width,
                        height,
                        frame_rate,
                        profile: encoded.into(),
                        level,
                        tone_map: values
                            .get("VideoRangeType")
                            .is_some_and(|v| ["HDR10", "HLG"].contains(&v.as_str())),
                        deinterlace: values.get("IsInterlaced").is_some_and(|v| v == "true"),
                        fit: false,
                    });
                    break 'candidate;
                }
            }
        }
    }
    let can_convert = if video.is_empty() {
        hls(profile, kind, "", "aac", &converted)
            && constraints(profile, "ts", "", "aac", &converted)
    } else {
        conversion.is_some()
    };
    Capabilities {
        containers: if direct { vec![container] } else { vec![] },
        video: if direct || remux {
            vec![video.into()]
        } else {
            vec![]
        },
        audio: if direct || remux {
            vec![audio.into()]
        } else {
            vec![]
        },
        hls: remux || can_convert,
        native_tracks: true,
        conversion,
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
        let caps = negotiate(&source, Some(1), &profile, true, true, 8_000_000);
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
            capabilities: negotiate(&source, Some(1), &denied, true, true, 8_000_000),
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
        let caps = negotiate(&source, Some(1), &profile, true, true, 8_000_000);
        assert_eq!(caps.containers, vec!["mkv"]);
        profile["CodecProfiles"][2]["ApplyConditions"] = json!([]);
        let caps = negotiate(&source, Some(1), &profile, true, true, 8_000_000);
        assert!(caps.containers.is_empty());
        let conversion = caps.conversion.unwrap();
        assert_eq!((conversion.width, conversion.height), (160, 90));
    }
}
