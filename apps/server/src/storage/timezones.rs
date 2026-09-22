//! Database operations for timezones.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn in_server_window(db: &Database, start: u32, end: u32) -> anyhow::Result<bool> {
    db.read("timezones.in_server_window", move |db| {
        maintenance_window(db, Utc::now(), start, end)
    })
    .await
}

pub(super) async fn get(db: &Database, p: thelxinoe_core::Principal) -> anyhow::Result<Value> {
    db.read("timezones.get", move |db| read(db, &p.user.id))
        .await
}

pub(super) async fn update(
    db: &Database,
    input: Preferences,
    p: thelxinoe_core::Principal,
) -> anyhow::Result<Value> {
    db.write("timezones.update", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE users SET timezone_override=?1 WHERE id=?2",
            params![input.timezone, p.user.id],
        )?;
        if let Some(format) = input.time_format {
            tx.execute(
                "UPDATE users SET time_format_override=?1 WHERE id=?2",
                params![format, p.user.id],
            )?;
        }
        let value = read(&tx, &p.user.id)?;
        tx.commit()?;
        Ok(value)
    })
    .await
}

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

pub(super) fn maintenance_window(
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

pub(super) fn read(db: &rusqlite::Connection, user: &str) -> anyhow::Result<Value> {
    Ok(db.query_row(
        "SELECT timezone,timezone_override,server_timezone,time_format,time_format_override,server_time_format FROM user_profiles WHERE id=?1",
        [user],
        |r| {
            Ok(json!({
                "timezone":r.get::<_,String>(0)?,
                "timezone_override":r.get::<_,Option<String>>(1)?,
                "server_timezone":r.get::<_,String>(2)?,
                "time_format":r.get::<_,String>(3)?,
                "time_format_override":r.get::<_,Option<String>>(4)?,
                "server_time_format":r.get::<_,String>(5)?,
            }))
        },
    )?)
}
