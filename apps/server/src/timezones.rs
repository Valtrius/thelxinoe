use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let mut zones = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|zone| zone.name())
        .collect::<Vec<_>>();
    zones.sort_unstable();
    Ok(Json(json!({"timezones":zones})))
}

fn read(db: &rusqlite::Connection, user: &str) -> anyhow::Result<Value> {
    Ok(db.query_row(
        "SELECT timezone,timezone_override,server_timezone FROM user_profiles WHERE id=?1",
        [user],
        |r| {
            Ok(json!({
                "timezone":r.get::<_,String>(0)?,
                "timezone_override":r.get::<_,Option<String>>(1)?,
                "server_timezone":r.get::<_,String>(2)?,
            }))
        },
    )?)
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(state.db.call(move |db| read(db, &p.user.id)).await?))
}

#[derive(Deserialize)]
pub struct Preferences {
    // Null restores inheritance. A named zone, including UTC, is an override.
    timezone: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Preferences>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if let Some(zone) = &input.timezone {
        zone.parse::<chrono_tz::Tz>()
            .map_err(|_| ApiError::bad("Unknown timezone"))?;
    }
    let recipient = p.user.id.clone();
    let value = state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE users SET timezone=?1,timezone_inherited=?2 WHERE id=?3",
                params![
                    input.timezone.as_deref().unwrap_or("UTC"),
                    input.timezone.is_none(),
                    p.user.id
                ],
            )?;
            let value = read(&tx, &p.user.id)?;
            tx.commit()?;
            Ok(value)
        })
        .await?;
    state
        .emit(Some(recipient), "preferences.changed", value.clone())
        .await?;
    Ok(Json(value))
}
