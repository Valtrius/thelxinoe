use super::{Target, Update};
use rusqlite::{OptionalExtension, params};
use thelxinoe_core::now;
use thelxinoe_database::Database;

pub(super) async fn update(
    db: &Database,
    user: String,
    target: Target,
    input: Update,
) -> anyhow::Result<u16> {
    db.write("jellyfin.user_data.update", move |db| {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let position = input.playback_position_ticks.map(|ticks| ticks as f64 / 10_000_000.0);
        match target {
            Target::Media(id) => {
                let change = crate::watching::Change {
                    watched: input.played,
                    favorite: input.is_favorite,
                    watch_later: None,
                    resume: position,
                };
                match crate::watching::change(&tx, &user, &id, change, now())? {
                    crate::watching::Changed::Done => {}
                    crate::watching::Changed::Missing => return Ok(404),
                    crate::watching::Changed::Unsupported | crate::watching::Changed::NoEdition => return Ok(400),
                }
            }
            Target::YouTube(video) => {
                let duration: Option<f64> = tx.query_row("SELECT COALESCE(duration,0) FROM youtube_videos WHERE user_id=?1 AND video_id=?2 AND privacy='public'", params![user,video], |row| row.get(0)).optional()?;
                let Some(duration) = duration else { return Ok(404); };
                if input.is_favorite == Some(true) && tx.query_row("SELECT COUNT(*) FROM youtube_video_state WHERE user_id=?1 AND video_id<>?2 AND (watchlist=1 OR pinned=1)", params![user,video], |row| row.get::<_,u32>(0))? >= 1000 {
                    return Ok(409);
                }
                let position = if input.played == Some(true) { Some(duration) } else { position.map(|value| if duration > 0.0 {value.min(duration)} else {value}).or_else(|| (input.played == Some(false)).then_some(0.0)) };
                tx.execute("INSERT INTO youtube_state(user_id,video_id,pinned,watched,position,updated_at) VALUES(?1,?2,COALESCE(?3,0),COALESCE(?4,0),COALESCE(?5,0),?6) ON CONFLICT(user_id,video_id) DO UPDATE SET pinned=COALESCE(?3,pinned),watched=COALESCE(?4,watched),position=COALESCE(?5,position),updated_at=?6", params![user,video,input.is_favorite,input.played,position,now()])?;
                if let Some(played) = input.played {
                    crate::online::watchlists::mark_watched(&tx, &user, &video, played)?;
                }
            }
            Target::Playlist(id) => {
                if !tx.query_row("SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1 AND (access<>'private' OR owner_id=?2))", [&id, &user], |row| row.get::<_,bool>(0))? {
                    return Ok(404);
                }
                if let Some(favorite) = input.is_favorite {
                    if favorite {
                        tx.execute("INSERT INTO playlist_favorites(user_id,playlist_id) VALUES(?1,?2) ON CONFLICT DO NOTHING", params![user,id])?;
                    } else {
                        tx.execute("DELETE FROM playlist_favorites WHERE user_id=?1 AND playlist_id=?2", params![user,id])?;
                    }
                }
            }
            Target::Online(id) => {
                if !tx.query_row("SELECT EXISTS(SELECT 1 FROM compat_online_items WHERE id=?2 AND user_id=?1)", params![user,id], |row| row.get::<_,bool>(0))? {
                    return Ok(404);
                }
                if let Some(favorite) = input.is_favorite {
                    tx.execute("UPDATE compat_online_items SET favorite=?3 WHERE id=?2 AND user_id=?1", params![user,id,favorite])?;
                }
            }
        }
        tx.commit()?;
        Ok(200)
    }).await
}
