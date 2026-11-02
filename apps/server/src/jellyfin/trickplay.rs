//! Jellyfin's trickplay endpoints, over the sheets of the trickplay worker.

use super::{Query, canonical};
use crate::{
    AppState,
    error::{ApiError, Result},
    trickplay::Info,
};
use axum::{
    extract::Request,
    http::header,
    response::{IntoResponse, Response},
};

/// `/Videos/{item}/Trickplay/{width}/{index}.jpg` and `/Videos/{item}/Trickplay/{width}/tiles.m3u8`
/// of the item's `MediaSourceId` file, or its first one.
pub async fn serve(
    state: &AppState,
    query: &Query,
    item: &str,
    width: &str,
    name: &str,
    request: Request,
) -> Result<Response> {
    let file = query.get("mediasourceid").map(|id| canonical(id));
    let source = crate::playback::source(state, &canonical(item), file.as_deref()).await?;
    let width = width.parse().map_err(|_| ApiError::not_found())?;
    if name == "tiles.m3u8" {
        let info = crate::trickplay::info(state, &source.id, &source.generation)
            .await
            .filter(|info| info.width == width)
            .ok_or_else(ApiError::not_found)?;
        let token = super::auth::token(request.headers(), query)?;
        return Ok((
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
            playlist(&info, &source.id, &token),
        )
            .into_response());
    }
    let index = name
        .strip_suffix(".jpg")
        .and_then(|index| index.parse().ok())
        .ok_or_else(ApiError::not_found)?;
    let path = crate::trickplay::sheet(state, &source, width, index)
        .await
        .ok_or_else(ApiError::not_found)?;
    crate::trickplay::file(path, request).await
}

/// Jellyfin's image playlist: one segment per sheet, every sheet but the last full. Players
/// fetch its sheets without headers, so their URLs carry the caller's token as Jellyfin's do.
fn playlist(info: &Info, source: &str, token: &str) -> String {
    let seconds = |ms: u32| {
        format!("{}.{:03}", ms / 1000, ms % 1000)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    };
    let per_sheet = info.tile_width * info.tile_height;
    let mut playlist = format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:{}\n#EXT-X-VERSION:7\n#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-IMAGES-ONLY\n",
        (per_sheet * info.interval).div_ceil(1000)
    );
    for index in 0..info.sheets() {
        let thumbnails = per_sheet.min(info.thumbnail_count - index * per_sheet);
        playlist.push_str(&format!(
            "#EXTINF:{},\n#EXT-X-TILES:RESOLUTION={}x{},LAYOUT={}x{},DURATION={}\n{index}.jpg?MediaSourceId={source}&ApiKey={token}\n",
            seconds(thumbnails * info.interval),
            info.width,
            info.height,
            info.tile_width,
            info.tile_height,
            seconds(info.interval),
        ));
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    playlist
}
