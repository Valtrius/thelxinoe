use anyhow::{Result, bail};
use serde_json::Value;

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

/// Whether converting a video stream, always to SDR BT.709, tone maps its PQ or HLG picture.
/// Fails for Dolby Vision without a standard base layer: FFmpeg can't reproduce its picture.
pub fn needs_tone_mapping(video: &Value) -> Result<bool> {
    match video_range(video) {
        (_, "DOVI" | "DOVIInvalid") => bail!("This Dolby Vision video cannot be converted"),
        (range, _) => Ok(range == "HDR"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn dolby_vision(profile: i64, compatibility: i64, transfer: &str) -> Value {
        json!({"codec_type":"video","codec_name":"hevc","color_space":"bt2020nc","color_transfer":transfer,"color_primaries":"bt2020","side_data_list":[{"side_data_type":"DOVI configuration record","dv_profile":profile,"dv_bl_signal_compatibility_id":compatibility,"rpu_present_flag":1,"bl_present_flag":1,"el_present_flag":0}]})
    }
    #[test]
    fn dolby_vision_is_named_and_converted_by_its_base_layer() {
        let pq = "smpte2084";
        let hlg = "arib-std-b67";
        // A conversion tone maps PQ and HLG base layers, and refuses a stream without one.
        for (stream, expected, tone_map) in [
            (dolby_vision(8, 1, pq), ("HDR", "DOVIWithHDR10"), Some(true)),
            (
                dolby_vision(10, 1, pq),
                ("HDR", "DOVIWithHDR10"),
                Some(true),
            ),
            (dolby_vision(8, 4, hlg), ("HDR", "DOVIWithHLG"), Some(true)),
            (
                dolby_vision(8, 2, "bt709"),
                ("SDR", "DOVIWithSDR"),
                Some(false),
            ),
            (dolby_vision(7, 6, pq), ("HDR", "DOVIWithEL"), Some(true)),
            (dolby_vision(5, 0, "bt709"), ("HDR", "DOVI"), None),
            (dolby_vision(10, 0, pq), ("HDR", "DOVI"), None),
            // A base layer whose tags contradict its record, or a compatibility outside the spec.
            (dolby_vision(8, 1, "bt709"), ("SDR", "DOVIInvalid"), None),
            (dolby_vision(8, 6, pq), ("HDR", "DOVIInvalid"), None),
            // A record without a playable layer leaves the base picture.
            (
                {
                    let mut s = dolby_vision(8, 1, pq);
                    s["side_data_list"][0]["rpu_present_flag"] = json!(0);
                    s
                },
                ("HDR", "HDR10"),
                Some(true),
            ),
            (json!({"color_transfer":hlg}), ("HDR", "HLG"), Some(true)),
            (
                json!({"color_transfer":"bt709"}),
                ("SDR", "SDR"),
                Some(false),
            ),
            (json!({}), ("SDR", "SDR"), Some(false)),
        ] {
            assert_eq!(video_range(&stream), expected, "{stream}");
            assert_eq!(needs_tone_mapping(&stream).ok(), tone_map, "{stream}");
        }
    }
}
