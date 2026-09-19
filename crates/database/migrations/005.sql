ALTER TABLE media_state ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_state ADD COLUMN watch_later INTEGER NOT NULL DEFAULT 0;
CREATE INDEX media_state_lists ON media_state(user_id, favorite, watch_later);

CREATE TABLE playlists (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE playlist_items (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY(playlist_id, position)
);
CREATE TABLE playlist_favorites (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    PRIMARY KEY(user_id, playlist_id)
);
CREATE TABLE music_queues (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    items TEXT NOT NULL,
    current_index INTEGER NOT NULL DEFAULT 0,
    position REAL NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(user_id, client_id)
);
ALTER TABLE playback_sessions ADD COLUMN client_id TEXT;
ALTER TABLE playback_sessions ADD COLUMN queue_revision INTEGER;
ALTER TABLE playback_sessions ADD COLUMN queue_index INTEGER;

-- Playback sessions are ephemeral and tied to a login. History is tied to the
-- user instead, so a sign-out or device revocation cannot erase viewing history.
CREATE TABLE playback_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playback_id TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    edition TEXT NOT NULL,
    device_name TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    ended_at INTEGER,
    position REAL NOT NULL,
    duration REAL NOT NULL,
    played_seconds REAL NOT NULL DEFAULT 0,
    state TEXT NOT NULL
);
CREATE INDEX playback_history_user ON playback_history(user_id, id DESC);

CREATE VIEW media_cards AS
SELECT m.id,m.kind,
    COALESCE(json_extract(m.overrides,'$.title'),json_extract(m.metadata,'$.title'),json_extract(m.metadata,'$.name'),m.title) AS title,
    m.parent_id,m.sort_number,m.year,
    EXISTS(SELECT 1 FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=m.id AND f.present=1) AS available
FROM media m;
