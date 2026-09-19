//! Bounded Data API requests. These credentials are never extractor inputs.
use super::{bounded_response, google, oauth, quota, redirect_uri};
use crate::{
    AppState,
    error::{ApiError, Result},
};
use oauth2::{RefreshToken, RequestTokenError, TokenResponse, basic::BasicErrorResponseType};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use thelxinoe_core::now;
#[cfg(test)]
mod tests;

pub(super) struct Access {
    pub generation: String,
    token: String,
}
struct Stored {
    generation: String,
    encrypted: Vec<u8>,
    expires: i64,
}
async fn stored(state: &AppState, user: &str) -> Result<Stored> {
    let user = user.to_owned();
    state.db.call(move|db|Ok(db.query_row("SELECT generation,credential,expires_at FROM online_accounts WHERE user_id=?1 AND provider='youtube' AND status='connected' AND credential IS NOT NULL",[user],|r|Ok(Stored{generation:r.get(0)?,encrypted:r.get(1)?,expires:r.get(2)?})).optional()?)).await?
        .ok_or_else(||ApiError::conflict("Connect your YouTube account to synchronize"))
}
fn decrypt(state: &AppState, user: &str, bytes: &[u8]) -> Result<oauth::Credential> {
    let bytes = state
        .secrets
        .decrypt(&format!("online:youtube:{user}"), bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::bad("Stored YouTube credentials are invalid; reconnect your account")
    })
}
async fn invalidate(state: &AppState, user: &str, generation: &str) -> Result<()> {
    let user = user.to_owned();
    let generation = generation.to_owned();
    state.db.call(move|db|{
        db.execute("UPDATE online_accounts SET status='reconnect_required',credential=NULL,expires_at=0,updated_at=?1 WHERE user_id=?2 AND provider='youtube' AND generation=?3",params![now(),user,generation])?;
        Ok(())
    }).await?;
    Ok(())
}
pub(super) async fn access(state: &AppState, user: &str) -> Result<Access> {
    let saved = stored(state, user).await?;
    if saved.expires > now() + 120 {
        return Ok(Access {
            generation: saved.generation,
            token: decrypt(state, user, &saved.encrypted)?.access_token,
        });
    }
    // Serialize refreshes, then reread. Concurrent requests never rotate the
    // same refresh token twice or overwrite a replacement connection.
    let _refresh = state.online.refresh.lock().await;
    let saved = stored(state, user).await?;
    let mut credential = decrypt(state, user, &saved.encrypted)?;
    if saved.expires > now() + 120 {
        return Ok(Access {
            generation: saved.generation,
            token: credential.access_token,
        });
    }
    let refresh = credential
        .refresh_token
        .clone()
        .ok_or_else(|| ApiError::conflict("Reconnect your YouTube account"))?;
    let client = oauth::client(state, google(state).await?, redirect_uri(state)?)?;
    let transport = |request| oauth::send(state.online.http.clone(), request);
    let token = match client
        .exchange_refresh_token(&RefreshToken::new(refresh))
        .request_async(&transport)
        .await
    {
        Ok(token) => token,
        Err(RequestTokenError::ServerResponse(error))
            if matches!(
                error.error(),
                BasicErrorResponseType::InvalidGrant | BasicErrorResponseType::InvalidClient
            ) =>
        {
            invalidate(state, user, &saved.generation).await?;
            return Err(ApiError::conflict(
                "YouTube access expired or was revoked; reconnect your account",
            ));
        }
        Err(_) => {
            return Err(ApiError::conflict(
                "Google token refresh is temporarily unavailable",
            ));
        }
    };
    if let Some(scopes) = token.scopes() {
        if !scopes.iter().any(|s| s.as_str() == oauth::SCOPE) {
            invalidate(state, user, &saved.generation).await?;
            return Err(ApiError::conflict(
                "YouTube read access was removed; reconnect your account",
            ));
        }
        credential.scope = scopes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    credential.access_token = token.access_token().secret().clone();
    if let Some(refresh) = token.refresh_token() {
        credential.refresh_token = Some(refresh.secret().clone());
    }
    let encrypted = state.secrets.encrypt(
        &format!("online:youtube:{user}"),
        &serde_json::to_vec(&credential).map_err(anyhow::Error::from)?,
    )?;
    let expires = now()
        + token
            .expires_in()
            .map(|d| d.as_secs().min(86400) as i64)
            .unwrap_or(3600);
    let owner = user.to_owned();
    let generation = saved.generation.clone();
    let updated=state.db.call(move|db|Ok(db.execute("UPDATE online_accounts SET credential=?1,expires_at=?2 WHERE user_id=?3 AND provider='youtube' AND generation=?4 AND status='connected' AND credential IS NOT NULL",params![encrypted,expires,owner,generation])?==1)).await?;
    if !updated {
        return Err(ApiError::conflict(
            "YouTube connection changed during refresh",
        ));
    }
    Ok(Access {
        generation: saved.generation,
        token: credential.access_token,
    })
}
pub(super) async fn get(
    state: &AppState,
    user: &str,
    access: &Access,
    endpoint: &str,
    query: &[(&str, &str)],
) -> Result<Value> {
    // The endpoint is internal, never a viewer-supplied URL.
    if !matches!(
        endpoint,
        "subscriptions" | "channels" | "playlistItems" | "videos"
    ) {
        return Err(ApiError::bad("Unsupported YouTube operation"));
    }
    let _slot = state
        .online
        .slots
        .acquire()
        .await
        .map_err(|_| ApiError::conflict("Online provider is stopping"))?;
    quota::reserve(state, user).await?;
    let response = state
        .online
        .http
        .get(format!("{}/{endpoint}", state.online.api))
        .bearer_auth(&access.token)
        .query(query)
        .send()
        .await
        .map_err(|_| ApiError::conflict("YouTube is temporarily unavailable"))?;
    let (status, _, bytes) = bounded_response(response)
        .await
        .map_err(|_| ApiError::conflict("YouTube returned an invalid response"))?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::conflict("YouTube returned an invalid response"))?;
    if status == axum::http::StatusCode::UNAUTHORIZED {
        // Force a refresh next time. Repeated denial does not spin or retry for
        // free: the scheduler applies backoff and every request reserves quota.
        let user = user.to_owned();
        let generation = access.generation.clone();
        state.db.call(move|db|{db.execute("UPDATE online_accounts SET expires_at=0 WHERE user_id=?1 AND provider='youtube' AND generation=?2",params![user,generation])?;Ok(())}).await?;
        return Err(ApiError::conflict(
            "YouTube rejected access; refreshing before the next sync",
        ));
    }
    if !status.is_success() {
        if endpoint == "playlistItems"
            && status.as_u16() == 404
            && data["error"]["errors"]
                .as_array()
                .is_some_and(|errors| errors.iter().any(|e| e["reason"] == "playlistNotFound"))
        {
            return Ok(serde_json::json!({"items":[]}));
        }
        let quota_exceeded = data["error"]["errors"].as_array().is_some_and(|errors| {
            errors.iter().any(|e| {
                matches!(
                    e["reason"].as_str(),
                    Some("quotaExceeded" | "dailyLimitExceeded")
                )
            })
        });
        if quota_exceeded {
            quota::block(state).await?;
            return Err(ApiError::conflict(
                "YouTube's shared daily API quota is exhausted",
            ));
        }
        return Err(ApiError::conflict(if status.as_u16() == 403 {
            "YouTube denied this request; check the application's API access and granted permissions"
        } else {
            "YouTube is temporarily unavailable; synchronization will retry"
        }));
    }
    Ok(data)
}
