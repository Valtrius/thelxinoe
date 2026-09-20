//! Public extraction never receives provider credentials or browser state.
use super::{process, sync, tools};
use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::{Value, json};
use std::{ffi::OsString, time::Duration};

pub async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = security::principal(&state, &headers).await?.user.id;
    if !sync::identifier(&id, 11) {
        return Err(ApiError::bad("Invalid YouTube video"));
    }
    let video = id.clone();
    let owned = state
        .db
        .call(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM youtube_videos WHERE user_id=?1 AND video_id=?2)",
                rusqlite::params![user, video],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if !owned {
        return Err(ApiError::not_found());
    }
    let metadata = match metadata(&state, &id).await {
        Ok(value) => value,
        Err(error)
            if error.1 == "extractor_authentication_required"
                || error.1 == "extraction_failed"
                || error.1 == "unavailable" =>
        {
            return Ok(Json(json!({"state":error.1})));
        }
        Err(error) => return Err(error),
    };
    Ok(Json(
        json!({"state":"available","duration":metadata["duration"],"live":metadata["is_live"].as_bool().unwrap_or(false),"format_count":metadata["formats"].as_array().map_or(0,Vec::len)}),
    ))
}
pub(super) async fn metadata(state: &AppState, id: &str) -> Result<Value> {
    if !sync::identifier(id, 11) {
        return Err(ApiError::bad("Invalid YouTube video"));
    }
    let _slot = state
        .online
        .extraction
        .try_acquire()
        .map_err(|_| ApiError::conflict("Both extraction slots are busy; try again shortly"))?;
    let bundle = tools::ready(state).await?;
    tools::verify(&bundle.yt_dlp).await?;
    tools::verify(&bundle.deno).await?;
    let args = arguments(&bundle, id);
    let output = process::run(
        &bundle.yt_dlp.path,
        &args,
        Duration::from_secs(120),
        8 * 1024 * 1024,
    )
    .await
    .map_err(|_| ApiError::conflict("Public extraction failed or timed out; try again later"))?;
    if !output.success {
        let code = failure(&output.stderr);
        return Err(ApiError(axum::http::StatusCode::CONFLICT,code,match code {"extractor_authentication_required"=>"Extractor authentication required; this video cannot be played with public access", "unavailable"=>"This video is unavailable with public access",_=>"Public extraction failed; try again later"}.into()));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| ApiError::conflict("The extractor returned invalid metadata"))?;
    if metadata["id"].as_str() != Some(id) {
        return Err(ApiError::conflict(
            "The extractor returned a different video",
        ));
    }
    Ok(metadata)
}
pub(super) fn arguments(bundle: &tools::Bundle, id: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-cache-dir",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--no-playlist",
        "--no-remote-components",
        "--no-js-runtimes",
        "--js-runtimes",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let mut runtime = OsString::from("deno:");
    runtime.push(&bundle.deno.path);
    args.push(runtime);
    args.extend(
        [
            "--socket-timeout",
            "15",
            "--retries",
            "2",
            "--extractor-retries",
            "2",
            "--dump-single-json",
            "--skip-download",
            "--",
            &format!("https://www.youtube.com/watch?v={id}"),
        ]
        .into_iter()
        .map(Into::into),
    );
    args
}
pub(super) fn failure(stderr: &[u8]) -> &'static str {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if [
        "sign in",
        "login required",
        "log in",
        "use --cookies",
        "members-only",
        "private video",
        "confirm your age",
    ]
    .iter()
    .any(|s| text.contains(s))
    {
        "extractor_authentication_required"
    } else if [
        "video unavailable",
        "not available",
        "has been removed",
        "copyright",
    ]
    .iter()
    .any(|s| text.contains(s))
    {
        "unavailable"
    } else {
        "extraction_failed"
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authentication_failures_are_explicit_and_redacted() {
        assert_eq!(
            failure(b"ERROR: Sign in to confirm your age. secret=private"),
            "extractor_authentication_required"
        );
        assert_eq!(failure(b"ERROR: Video unavailable"), "unavailable");
        assert_eq!(
            failure(b"HTTP Error 503 with secret URL"),
            "extraction_failed"
        );
    }
}
