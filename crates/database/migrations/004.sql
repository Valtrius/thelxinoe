CREATE TABLE playback_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    auth_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES media_files(id),
    generation TEXT NOT NULL,
    edition TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('ready','playing','paused','stopped','failed')),
    mode TEXT NOT NULL CHECK(mode IN ('direct','remux','transcode')),
    options TEXT NOT NULL,
    duration REAL NOT NULL,
    position REAL NOT NULL DEFAULT 0,
    sequence INTEGER NOT NULL DEFAULT -1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX playback_active ON playback_sessions(state, updated_at);
CREATE TABLE media_state (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    watched INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(user_id, media_id)
);
CREATE TABLE edition_progress (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    edition TEXT NOT NULL,
    position REAL NOT NULL,
    duration REAL NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(user_id, media_id, edition)
);
CREATE TABLE playback_preferences (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    value TEXT NOT NULL
);
