//! Database operations for avatars.
use super::*;
use thelxinoe_database::Database;

pub(super) async fn profile(id: String, db: &Database) -> anyhow::Result<Option<String>> {
    db.read("avatars.profile", move |db| {
        Ok(db
            .query_row(
                "SELECT image FROM user_avatars WHERE user_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?)
    })
    .await
}

pub(super) async fn save(
    db: &Database,
    image: Option<String>,
    user_id: String,
) -> anyhow::Result<()> {
    db.write("avatars.save", move |db| {
        if let Some(image) = image {
            db.execute("INSERT INTO user_avatars(user_id,image,updated_at) VALUES (?1,?2,?3)
                ON CONFLICT(user_id) DO UPDATE SET image=excluded.image,updated_at=excluded.updated_at",
                params![user_id, image, now()])?;
        } else {
            db.execute("DELETE FROM user_avatars WHERE user_id=?1", [user_id])?;
        }
        Ok(())
    }).await
}
