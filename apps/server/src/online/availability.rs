#[path = "../storage/online/availability.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use thelxinoe_core::Capability;

pub(crate) const PROVIDERS: [&str; 3] = ["youtube", "twitch", "kick"];

#[derive(Serialize)]
pub(crate) struct Availability {
    youtube: bool,
    twitch: bool,
    kick: bool,
}

pub(crate) async fn enabled(state: &AppState, provider: &str) -> anyhow::Result<bool> {
    storage::enabled(&state.db, provider.to_owned()).await
}

pub(crate) async fn require_enabled(state: &AppState, provider: &str) -> Result<()> {
    if !enabled(state, provider).await? {
        return Err(ApiError::not_found());
    }
    Ok(())
}

pub(crate) async fn require_media(state: &AppState, media: &str) -> Result<()> {
    if let Some((provider, _)) = media.split_once(':')
        && PROVIDERS.contains(&provider)
    {
        require_enabled(state, provider).await?;
    }
    Ok(())
}

pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Availability>> {
    security::principal(&state, &headers).await?;
    Ok(Json(storage::list(&state.db).await?))
}

#[derive(Deserialize)]
pub(super) struct Change {
    enabled: bool,
}

pub(super) async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(change): Json<Change>,
) -> Result<Json<Availability>> {
    let principal = security::require(&state, &headers, Capability::ManageServer).await?;
    if !PROVIDERS.contains(&provider.as_str()) {
        return Err(ApiError::not_found());
    }
    let stopped = storage::save(
        &state.db,
        provider.clone(),
        change.enabled,
        principal.user.id,
    )
    .await?;
    for session in stopped {
        let _lifecycle = state.playback_lifecycle.write(&[&session]).await;
        state.playback.stop(&session).await;
    }
    let availability = storage::list(&state.db).await?;
    state
        .emit(
            None,
            "online.configuration.changed",
            serde_json::to_value(&availability).expect("provider availability serialization"),
        )
        .await?;
    Ok(Json(availability))
}

pub(super) async fn gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response> {
    if let Some(provider) = request
        .uri()
        .path()
        .strip_prefix("/api/v1/online/")
        .and_then(|path| path.split('/').next())
        && PROVIDERS.contains(&provider)
    {
        require_enabled(&state, provider).await?;
    }
    Ok(next.run(request).await)
}
