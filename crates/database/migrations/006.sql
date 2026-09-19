CREATE TABLE compat_devices (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    client TEXT NOT NULL,
    version TEXT NOT NULL
);
CREATE TABLE compat_preferences (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client TEXT NOT NULL,
    preference_id TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(user_id,client,preference_id)
);
CREATE TABLE compat_playbacks (
    playback_id TEXT PRIMARY KEY REFERENCES playback_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE quick_connect (
    secret_hash TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL UNIQUE,
    device_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    client TEXT NOT NULL,
    version TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    authorizer_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE
);
