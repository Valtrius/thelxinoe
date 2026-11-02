use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::http::StatusCode;
use serde::Serialize;
use thelxinoe_playback::{Encoding, HardwareStatus};

#[derive(Clone, Copy, Serialize)]
pub(crate) struct Policy {
    pub video_transcoding: bool,
    pub audio_transcoding: bool,
    pub hardware_conversion: bool,
}
impl Policy {
    pub fn check(self, encoding: Encoding) -> Result<()> {
        let track = if encoding.video && !self.video_transcoding {
            "Video"
        } else if encoding.audio && !self.audio_transcoding {
            "Audio"
        } else {
            return Ok(());
        };
        Err(ApiError(
            StatusCode::FORBIDDEN,
            "transcoding_disabled",
            format!("{track} transcoding is disabled on this server"),
        ))
    }
}
/// Finds what the GPU does and shows administrators the result.
pub async fn probe_hardware(state: AppState) {
    // Startup installs FFmpeg after the server opens.
    state.tools.wait_ready().await;
    match state.playback.probe_hardware().await {
        HardwareStatus::Ready(gpu) => tracing::info!(
            device = %gpu.device,
            driver = %gpu.description,
            scale = gpu.scale,
            decode_h264 = gpu.h264,
            decode_hevc = gpu.hevc,
            decode_hevc_10bit = gpu.hevc_10bit,
            tone_map = gpu.tone_map,
            download = gpu.download,
            deinterlace = gpu.deinterlace,
            "Hardware video conversion is available"
        ),
        HardwareStatus::Unavailable { reason } => {
            tracing::info!(%reason, "Hardware video conversion is unavailable")
        }
        _ => {}
    }
    if let Err(error) = state
        .emit(None, "server.settings.changed", serde_json::json!({}))
        .await
    {
        tracing::warn!(%error, "Hardware probe result was not announced");
    }
}
