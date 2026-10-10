#[path = "storage/timezones.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) use storage::server_zone;

pub(crate) use storage::maintenance_window;

pub(crate) async fn in_server_window(
    state: &AppState,
    start: u32,
    end: u32,
) -> anyhow::Result<bool> {
    // Resolve the saved zone at the point of use: changing the server setting
    // also changes already queued maintenance, without restarting a scheduler.
    storage::in_server_window(&state.db, start, end).await
}

pub(crate) async fn opened_server_window(
    state: &AppState,
    start: u32,
    end: u32,
) -> anyhow::Result<Option<NaiveDate>> {
    storage::opened_server_window(&state.db, start, end).await
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

pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    Ok(Json(storage::get(&state.db, p).await?))
}

#[derive(Deserialize)]
pub struct Preferences {
    // Null restores inheritance. A named zone, including UTC, is an override.
    timezone: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_time_format")]
    time_format: Option<Option<String>>,
}

fn deserialize_present_time_format<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
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
    if let Some(Some(format)) = &input.time_format
        && !matches!(format.as_str(), "12h" | "24h")
    {
        return Err(ApiError::bad("Unknown time format"));
    }
    let recipient = p.user.id.clone();
    let value = storage::update(&state.db, input, p).await?;
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
        use crate::test_support::{call, fixture};
        let (_temp, state, cookie) = fixture().await;
        state
            .db
            .write("test.fixture", |db| {
                db.execute(
                    "UPDATE users SET role='admin',timezone_override='Asia/Tokyo' WHERE id='alice'",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
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

    #[test]
    fn window_occurrences_are_named_by_their_local_opening_date() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT); INSERT INTO settings VALUES ('timezone','Asia/Tokyo')").unwrap();
        let at = |value: &str| value.parse::<DateTime<Utc>>().unwrap();
        let day = |value: &str| Some(value.parse::<NaiveDate>().unwrap());
        // 03:00 in Tokyo is still the previous day in UTC.
        assert_eq!(
            storage::window_opened(&db, at("2026-07-09T18:30:00Z"), 3, 5).unwrap(),
            day("2026-07-10")
        );
        assert_eq!(
            storage::window_opened(&db, at("2026-07-09T21:00:00Z"), 3, 5).unwrap(),
            None
        );
        // Both sides of midnight belong to the occurrence that opened at 22:00.
        for instant in ["2026-07-10T13:30:00Z", "2026-07-10T17:30:00Z"] {
            assert_eq!(
                storage::window_opened(&db, at(instant), 22, 3).unwrap(),
                day("2026-07-10")
            );
        }
        assert_eq!(
            storage::window_opened(&db, at("2026-07-10T14:59:00Z"), 0, 0).unwrap(),
            day("2026-07-10")
        );
        assert_eq!(
            storage::window_opened(&db, at("2026-07-10T15:00:00Z"), 0, 0).unwrap(),
            day("2026-07-11")
        );
    }
}
