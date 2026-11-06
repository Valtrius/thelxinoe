-- Schema 5 to 6. Runs inside the opening transaction with foreign keys off.

-- A playlist's owner chooses who else sees and edits it. Every playlist was readable by
-- everyone until now, so existing ones stay that way. Its sort is shared by its viewers.
ALTER TABLE playlists ADD COLUMN access TEXT NOT NULL DEFAULT 'private' CHECK(access IN ('private','read','add','edit'));
ALTER TABLE playlists ADD COLUMN sort_mode TEXT NOT NULL DEFAULT 'manual' CHECK(sort_mode IN ('manual','added','title','artist'));
ALTER TABLE playlists ADD COLUMN sort_direction TEXT NOT NULL DEFAULT 'asc' CHECK(sort_direction IN ('asc','desc'));
UPDATE playlists SET access='read';

-- A track is in a playlist once, at its first position. Rows did not record when they
-- were added: the playlist's last update stands in, and ties keep the manual order.
CREATE TABLE playlist_items_new (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY(playlist_id, media_id)
) STRICT;
INSERT INTO playlist_items_new(playlist_id,media_id,position,added_at)
    SELECT i.playlist_id,i.media_id,MIN(i.position),p.updated_at
    FROM playlist_items i JOIN playlists p ON p.id=i.playlist_id
    GROUP BY i.playlist_id,i.media_id;
DROP TABLE playlist_items;
ALTER TABLE playlist_items_new RENAME TO playlist_items;
CREATE INDEX playlist_items_by_position ON playlist_items(playlist_id, position);
