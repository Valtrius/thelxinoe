use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) fn server_zone(db: &rusqlite::Connection) -> anyhow::Result<Tz> {
    let zone: Option<String> = db
        .query_row(
            "SELECT value FROM settings WHERE key='timezone'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    zone.as_deref()
        .unwrap_or("UTC")
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid server timezone"))
}

fn maintenance_window(
    db: &rusqlite::Connection,
    instant: DateTime<Utc>,
    start: u32,
    end: u32,
) -> anyhow::Result<bool> {
    let hour = instant.with_timezone(&server_zone(db)?).hour();
    Ok(start == end
        || if start < end {
            (start..end).contains(&hour)
        } else {
            hour >= start || hour < end
        })
}

pub(crate) async fn in_server_window(
    state: &AppState,
    start: u32,
    end: u32,
) -> anyhow::Result<bool> {
    // Resolve the saved zone at the point of use: changing the server setting
    // also changes already queued maintenance, without restarting a scheduler.
    state
        .db
        .call(move |db| maintenance_window(db, Utc::now(), start, end))
        .await
}

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
        "SELECT p.timezone,p.timezone_override,p.server_timezone,u.time_format FROM user_profiles p JOIN users u ON u.id=p.id WHERE p.id=?1",
        [user],
        |r| {
            Ok(json!({
                "timezone":r.get::<_,String>(0)?,
                "timezone_override":r.get::<_,Option<String>>(1)?,
                "server_timezone":r.get::<_,String>(2)?,
                "time_format":r.get::<_,String>(3)?,
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
    time_format: Option<String>,
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
    if let Some(format) = &input.time_format
        && !matches!(format.as_str(), "12h" | "24h")
    {
        return Err(ApiError::bad("Unknown time format"));
    }
    let recipient = p.user.id.clone();
    let value = state
        .db
        .call(move |db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE users SET timezone=?1,timezone_inherited=?2,time_format=COALESCE(?3,time_format) WHERE id=?4",
                params![
                    input.timezone.as_deref().unwrap_or("UTC"),
                    input.timezone.is_none(),
                    input.time_format,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_maintenance_policies_expose_the_current_zone() {
        use crate::online::oauth::tests::{call, fixture};
        let (_temp, state, cookie) = fixture().await;
        state.db.call(|db| {
            db.execute("UPDATE users SET role='admin',timezone='Asia/Tokyo',timezone_inherited=0 WHERE id='alice'", [])?;
            Ok(())
        }).await.unwrap();
        for zone in ["America/New_York", "Europe/Paris"] {
            assert_eq!(
                call(
                    &state,
                    "/api/v1/admin/settings",
                    "PUT",
                    json!({"timezone":zone}),
                    &cookie
                )
                .await
                .0,
                axum::http::StatusCode::OK
            );
            for endpoint in [
                "/api/v1/admin/product-update",
                "/api/v1/admin/service-updates",
            ] {
                let (status, _, body) = call(&state, endpoint, "GET", Value::Null, &cookie).await;
                assert_eq!(status, axum::http::StatusCode::OK);
                assert_eq!(body["timezone"], zone);
            }
        }
    }

    #[test]
    fn server_maintenance_uses_saved_zone_and_follows_dst() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT); INSERT INTO settings VALUES ('timezone','America/New_York')").unwrap();
        let at = |value: &str| value.parse::<DateTime<Utc>>().unwrap();
        // The same 03:00–05:00 policy follows both offsets of New York.
        assert!(maintenance_window(&db, at("2026-01-10T08:00:00Z"), 3, 5).unwrap());
        assert!(maintenance_window(&db, at("2026-07-10T07:00:00Z"), 3, 5).unwrap());
        assert!(!maintenance_window(&db, at("2026-07-10T03:00:00Z"), 3, 5).unwrap());
        // Missing local hour during spring-forward never opens a 02:00 window.
        assert!(!maintenance_window(&db, at("2026-03-08T06:59:00Z"), 2, 3).unwrap());
        assert!(!maintenance_window(&db, at("2026-03-08T07:00:00Z"), 2, 3).unwrap());
        // Both occurrences of a repeated local hour fall in that local window.
        assert!(maintenance_window(&db, at("2026-11-01T05:30:00Z"), 1, 2).unwrap());
        assert!(maintenance_window(&db, at("2026-11-01T06:30:00Z"), 1, 2).unwrap());
        assert!(maintenance_window(&db, at("2026-07-10T03:00:00Z"), 22, 3).unwrap());
        assert!(!maintenance_window(&db, at("2026-07-10T07:00:00Z"), 22, 3).unwrap());
        assert!(maintenance_window(&db, at("2026-07-10T07:00:00Z"), 0, 0).unwrap());
        db.execute(
            "UPDATE settings SET value='Europe/Paris' WHERE key='timezone'",
            [],
        )
        .unwrap();
        assert!(maintenance_window(&db, at("2026-07-10T01:00:00Z"), 3, 5).unwrap());
        assert!(!maintenance_window(&db, at("2026-07-10T07:00:00Z"), 3, 5).unwrap());
        db.execute("DELETE FROM settings", []).unwrap();
        assert!(maintenance_window(&db, at("2026-07-10T03:00:00Z"), 3, 5).unwrap());
    }
}
