//! Opt-in VAAPI conversion. A probe runs the production filter graphs on each render node
//! and records what one verifiably does; conversions use only that, and software otherwise.
use crate::{Capabilities, Conversion, Options, TONE_MAP};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    path::Path,
    process::Stdio,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thelxinoe_tools::MediaTools;

#[cfg(test)]
#[path = "hardware_tests.rs"]
mod tests;

/// A render node, the libva driver conversions load for it, and what a probe verified.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accelerator {
    pub device: String,
    pub driver: String,
    /// How the driver describes itself.
    pub description: String,
    /// GPU scaling, which every GPU-decoded conversion uses.
    pub scale: bool,
    pub h264: bool,
    pub hevc: bool,
    pub hevc_10bit: bool,
    /// PQ to SDR on the GPU.
    pub tone_map: bool,
    /// GPU frames through software filters and back, to tone-map on the CPU.
    pub download: bool,
    pub deinterlace: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HardwareStatus {
    #[default]
    Unprobed,
    Probing,
    /// Encodes H.264; the rest as listed.
    Ready(Accelerator),
    Unavailable {
        reason: String,
    },
}
#[derive(Default)]
pub(crate) struct Hardware {
    enabled: AtomicBool,
    status: Mutex<HardwareStatus>,
    probe: tokio::sync::Mutex<()>,
}
impl Hardware {
    pub(crate) fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
    pub(crate) fn status(&self) -> HardwareStatus {
        self.status.lock().unwrap().clone()
    }
    /// What new conversions use.
    pub(crate) fn accelerator(&self) -> Option<Accelerator> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        match &*self.status.lock().unwrap() {
            HardwareStatus::Ready(accelerator) => Some(accelerator.clone()),
            _ => None,
        }
    }
    pub(crate) async fn probe(&self, tools: Result<MediaTools>, work: &Path) -> HardwareStatus {
        let _probe = self.probe.lock().await;
        *self.status.lock().unwrap() = HardwareStatus::Probing;
        let status = match tools {
            Ok(tools) => probe(&tools, work).await,
            Err(error) => HardwareStatus::Unavailable {
                reason: error.to_string(),
            },
        };
        let _ = tokio::fs::remove_dir_all(work).await;
        *self.status.lock().unwrap() = status.clone();
        status
    }
}

/// How a conversion uses the GPU. It always encodes there; it also decodes and filters there
/// when the probe verified everything the source needs, else software decodes and filters.
pub(crate) struct Plan<'a> {
    pub accelerator: &'a Accelerator,
    pub decode: bool,
}
impl Accelerator {
    pub(crate) fn plan(
        &self,
        video: Option<&Value>,
        options: &Options,
        tone_map: bool,
    ) -> Plan<'_> {
        let decodable = match video.map(|v| (v["codec_name"].as_str(), v["pix_fmt"].as_str())) {
            Some((Some("h264"), Some("yuv420p" | "yuvj420p"))) => self.h264,
            Some((Some("hevc"), Some("yuv420p"))) => self.hevc,
            Some((Some("hevc"), Some("yuv420p10le"))) => self.hevc_10bit,
            _ => false,
        };
        let conversion = options.capabilities.conversion.as_ref();
        Plan {
            accelerator: self,
            decode: self.scale
                && decodable
                // FFmpeg rotates and pads in software only.
                && !video.is_some_and(rotated)
                && !conversion.is_some_and(|c| c.fit)
                && (self.deinterlace || !conversion.is_some_and(|c| c.deinterlace))
                && (!tone_map || self.download || (self.tone_map && video.is_some_and(hdr10))),
        }
    }
    /// The FFmpeg device, `va`, on this node and driver only.
    fn device(&self) -> String {
        format!(
            "vaapi=va:{},driver={},connection_type=drm",
            self.device, self.driver
        )
    }
}
/// PQ with the mastering display metadata `tonemap_vaapi` requires.
fn hdr10(video: &Value) -> bool {
    video["color_transfer"] == "smpte2084"
        && video["side_data_list"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["side_data_type"] == "Mastering display metadata")
}
fn rotated(video: &Value) -> bool {
    video["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| {
            s["side_data_type"] == "Display Matrix" && s["rotation"].as_f64().unwrap_or(0.0) != 0.0
        })
        || video["tags"]["rotate"].as_str().is_some_and(|r| r != "0")
}
impl Plan<'_> {
    /// Options before the video input.
    pub(crate) fn input_args(&self) -> Vec<String> {
        let mut args = vec![
            "-init_hw_device".to_owned(),
            self.accelerator.device(),
            "-filter_hw_device".into(),
            "va".into(),
        ];
        if self.decode {
            args.extend(
                [
                    "-hwaccel",
                    "vaapi",
                    "-hwaccel_device",
                    "va",
                    "-hwaccel_output_format",
                    "vaapi",
                ]
                .map(str::to_owned),
            );
        }
        args
    }
    /// The filter graph, given the `software` one that runs when the GPU doesn't decode.
    pub(crate) fn filter(
        &self,
        options: &Options,
        video: Option<&Value>,
        tone_map: bool,
        software: &str,
    ) -> String {
        if !self.decode {
            return format!("{software},format=nv12,hwupload");
        }
        let conversion = options.capabilities.conversion.as_ref();
        let scale = match (conversion, crate::resolution(&options.quality)) {
            (Some(c), _) => format!("w={}:h={}", c.width, c.height),
            (None, Some(height)) => format!("w=-2:h='min({height},ih)'"),
            (None, None) => "w='trunc(iw/2)*2':h='trunc(ih/2)*2'".into(),
        };
        // Scaled first, so tone mapping processes the output's pixels only.
        let tone = if !tone_map {
            String::new()
        } else if self.accelerator.tone_map && video.is_some_and(hdr10) {
            ",tonemap_vaapi=format=nv12:t=bt709:m=bt709:p=bt709,sidedata=mode=delete:type=MASTERING_DISPLAY_METADATA,sidedata=mode=delete:type=CONTENT_LIGHT_LEVEL".into()
        } else {
            format!(",hwdownload,format=p010le{TONE_MAP},format=nv12,hwupload")
        };
        format!(
            "{}scale_vaapi={scale}:format={}{tone}{}",
            if conversion.is_some_and(|c| c.deinterlace) {
                "deinterlace_vaapi,"
            } else {
                ""
            },
            if tone_map { "p010" } else { "nv12" },
            conversion.map_or(String::new(), |c| format!(
                ",setsar=1,fps={:.6}",
                c.frame_rate
            )),
        )
    }
}

/// The first render node and driver that encode H.264, and what else they do.
async fn probe(tools: &MediaTools, work: &Path) -> HardwareStatus {
    let mut devices = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("renderD"))
        })
        .collect::<Vec<_>>();
    devices.sort();
    if devices.is_empty() {
        return HardwareStatus::Unavailable {
            reason: "No VAAPI device".into(),
        };
    }
    // libva's own choice, which an administrator can set.
    let drivers = match std::env::var("LIBVA_DRIVER_NAME") {
        Ok(name) if !name.is_empty() => vec![name],
        _ => vec!["iHD".to_owned(), "i965".to_owned()],
    };
    let samples = samples(tools, work).await;
    let mut reasons = Vec::new();
    for device in &devices {
        let device = device.to_string_lossy();
        for driver in &drivers {
            match probe_driver(tools, &samples, &device, driver).await {
                Ok(accelerator) => return HardwareStatus::Ready(accelerator),
                Err(error) => reasons.push(
                    format!("{device} {driver}: {error:#}")
                        .trim_end_matches('.')
                        .to_owned(),
                ),
            }
        }
    }
    HardwareStatus::Unavailable {
        reason: reasons.join("; "),
    }
}
#[derive(Default)]
struct Samples {
    h264: Option<String>,
    hevc: Option<String>,
    hevc_10bit: Option<String>,
    pq: Option<String>,
}
/// Short clips of each format the GPU may decode, encoded in software.
async fn samples(tools: &MediaTools, work: &Path) -> Samples {
    let mut samples = Samples::default();
    if let Err(error) = tokio::fs::create_dir_all(work).await {
        tracing::warn!(%error, "Hardware probe samples are unavailable");
        return samples;
    }
    for (slot, name, encoder, format, hdr10) in [
        (&mut samples.h264, "h264", "libx264", "yuv420p", None),
        (&mut samples.hevc, "hevc", "libx265", "yuv420p", None),
        (
            &mut samples.hevc_10bit,
            "hevc-10bit",
            "libx265",
            "yuv420p10le",
            None,
        ),
        (
            &mut samples.pq,
            "pq",
            "libx265",
            "yuv420p10le",
            Some(
                ":hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400",
            ),
        ),
    ] {
        let path = work
            .join(format!("{name}.mkv"))
            .to_string_lossy()
            .into_owned();
        let x265 = format!("log-level=none{}", hdr10.unwrap_or(""));
        let mut args = vec![
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=24:d=0.5",
            "-c:v",
            encoder,
            "-preset",
            "ultrafast",
            "-pix_fmt",
            format,
        ];
        if encoder == "libx265" {
            args.extend(["-x265-params", &x265]);
        }
        if hdr10.is_some() {
            // Tagged on the frames: FFmpeg 8 encodes their tags rather than -color_trc.
            args.extend([
                "-vf",
                "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
            ]);
        }
        args.push(&path);
        match ffmpeg(tools, args).await {
            Ok(_) => *slot = Some(path),
            Err(error) => tracing::warn!(%error, sample = name, "Hardware probe sample failed"),
        }
    }
    samples
}
type Capability = fn(&mut Accelerator) -> &mut bool;
async fn probe_driver(
    tools: &MediaTools,
    samples: &Samples,
    device: &str,
    driver: &str,
) -> Result<Accelerator> {
    let mut accelerator = Accelerator {
        device: device.into(),
        driver: driver.into(),
        ..Accelerator::default()
    };
    let device_arg = accelerator.device();
    let log = ffmpeg(
        tools,
        [
            "-v",
            "verbose",
            "-init_hw_device",
            &device_arg,
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x64:d=0.1",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ],
    )
    .await
    .context("VAAPI initialization failed")?;
    accelerator.description = log
        .lines()
        .find_map(|line| line.split_once("VAAPI driver: "))
        // iHD ends its name with an empty build note: "… - 25.2.3 ()".
        .map_or(driver, |(_, v)| {
            v.trim().trim_end_matches('.').trim_end_matches(" ()")
        })
        .into();
    let pattern = json!({"codec_type":"video","codec_name":"rawvideo","pix_fmt":"yuv420p","width":320,"height":240});
    conversion(tools, &accelerator, PATTERN, &pattern, &quality("144p"))
        .await
        .context("H.264 encoding failed")?;
    accelerator.scale = ffmpeg(
        tools,
        [
            "-v",
            "error",
            "-init_hw_device",
            &device_arg,
            "-filter_hw_device",
            "va",
            "-f",
            "lavfi",
            "-i",
            PATTERN,
            "-vf",
            "format=nv12,hwupload,scale_vaapi=w=160:h=120:format=nv12",
            "-c:v",
            "h264_vaapi",
            "-f",
            "null",
            "-",
        ],
    )
    .await
    .inspect_err(|error| tracing::info!(device, driver, %error, "VAAPI scaling is unavailable"))
    .is_ok();
    if !accelerator.scale {
        return Ok(accelerator);
    }
    let stream = |codec: &str, format: &str| json!({"codec_type":"video","codec_name":codec,"pix_fmt":format,"width":320,"height":240});
    let decoders: [(Capability, _, _, _); 3] = [
        (
            |a| &mut a.h264,
            &samples.h264,
            stream("h264", "yuv420p"),
            "H.264 decoding",
        ),
        (
            |a| &mut a.hevc,
            &samples.hevc,
            stream("hevc", "yuv420p"),
            "HEVC decoding",
        ),
        (
            |a| &mut a.hevc_10bit,
            &samples.hevc_10bit,
            stream("hevc", "yuv420p10le"),
            "HEVC 10-bit decoding",
        ),
    ];
    for (capability, sample, stream, name) in decoders {
        let Some(sample) = sample else { continue };
        *capability(&mut accelerator) = verified(
            tools,
            &accelerator,
            capability,
            sample,
            &stream,
            &quality("144p"),
            name,
        )
        .await;
    }
    if let Some(sample) = samples.pq.as_ref().filter(|_| accelerator.hevc_10bit) {
        let mut pq = stream("hevc", "yuv420p10le");
        pq["color_transfer"] = json!("smpte2084");
        pq["color_primaries"] = json!("bt2020");
        pq["color_space"] = json!("bt2020nc");
        pq["side_data_list"] = json!([{"side_data_type":"Mastering display metadata"}]);
        accelerator.tone_map = verified(
            tools,
            &accelerator,
            |a| &mut a.tone_map,
            sample,
            &pq,
            &quality("144p"),
            "Tone mapping",
        )
        .await;
        // The CPU tone-maps when the GPU doesn't.
        let gpu = std::mem::take(&mut accelerator.tone_map);
        accelerator.download = verified(
            tools,
            &accelerator,
            |a| &mut a.download,
            sample,
            &pq,
            &quality("auto"),
            "Frame download",
        )
        .await;
        accelerator.tone_map = gpu;
    }
    let interlaced = [
        (&samples.h264, stream("h264", "yuv420p"), accelerator.h264),
        (&samples.hevc, stream("hevc", "yuv420p"), accelerator.hevc),
    ]
    .into_iter()
    .find_map(|(sample, stream, decoded)| Some((sample.as_ref().filter(|_| decoded)?, stream)));
    if let Some((sample, stream)) = interlaced {
        let options = Options {
            capabilities: Capabilities {
                conversion: Some(Conversion {
                    width: 160,
                    height: 120,
                    frame_rate: 25.0,
                    profile: "high".into(),
                    level: 41,
                    deinterlace: true,
                    fit: false,
                }),
                ..Capabilities::default()
            },
            ..quality("auto")
        };
        accelerator.deinterlace = verified(
            tools,
            &accelerator,
            |a| &mut a.deinterlace,
            sample,
            &stream,
            &options,
            "Deinterlacing",
        )
        .await;
    }
    Ok(accelerator)
}
const PATTERN: &str = "testsrc2=s=320x240:r=24:d=0.5";
fn quality(quality: &str) -> Options {
    Options {
        quality: quality.into(),
        audio: None,
        subtitle: None,
        capabilities: Capabilities::default(),
    }
}
/// Whether the GPU decodes and converts a sample once it has `capability`, as production would.
async fn verified(
    tools: &MediaTools,
    accelerator: &Accelerator,
    capability: Capability,
    sample: &str,
    stream: &Value,
    options: &Options,
    name: &str,
) -> bool {
    let mut candidate = accelerator.clone();
    *capability(&mut candidate) = true;
    let tone_map = crate::needs_tone_mapping(stream).unwrap_or(false);
    if !candidate.plan(Some(stream), options, tone_map).decode {
        return false;
    }
    conversion(tools, &candidate, sample, stream, options)
        .await
        .inspect_err(|error| {
            tracing::info!(device = %candidate.device, driver = %candidate.driver, %error, "{name} on the GPU is unavailable");
        })
        .is_ok()
}
/// Runs a production conversion of `input`, a file or a lavfi graph, to nothing.
async fn conversion(
    tools: &MediaTools,
    accelerator: &Accelerator,
    input: &str,
    stream: &Value,
    options: &Options,
) -> Result<()> {
    let args =
        crate::video_conversion_args(options, &json!({ "streams": [stream] }), Some(accelerator))?;
    let mut command = vec!["-v".to_owned(), "error".into()];
    command.extend(args.input);
    if stream["codec_name"] == "rawvideo" {
        command.extend(["-f".into(), "lavfi".into()]);
    }
    command.extend(["-i".into(), input.into(), "-map".into(), "0:v:0".into()]);
    command.extend(args.output);
    command.extend(["-f".into(), "null".into(), "-".into()]);
    ffmpeg(tools, command).await?;
    Ok(())
}
/// FFmpeg's log, or its first error, usually the cause, as the error.
async fn ffmpeg(
    tools: &MediaTools,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<String> {
    tools.ffmpeg.verify_unchanged().await?;
    let mut command = tokio::process::Command::new(&tools.ffmpeg.path);
    command
        .env_remove("FFREPORT")
        .args(["-hide_banner", "-nostdin", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .context("FFmpeg timed out")??;
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        return Ok(log);
    }
    let mut lines = log.lines().map(str::trim).filter(|l| !l.is_empty());
    let error = lines
        .clone()
        .find(|line| {
            let line = line.to_ascii_lowercase();
            line.contains("error") || line.contains("fail")
        })
        .or_else(|| lines.next_back())
        .unwrap_or("FFmpeg failed");
    bail!("{}", error.chars().take(200).collect::<String>())
}
