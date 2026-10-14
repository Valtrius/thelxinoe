use crate::error::{ApiError, Result};
use axum::http::StatusCode;
use serde::Serialize;
use thelxinoe_playback::Encoding;

#[derive(Clone, Copy, Serialize)]
pub(crate) struct Policy {
    pub video_transcoding: bool,
    pub audio_transcoding: bool,
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
