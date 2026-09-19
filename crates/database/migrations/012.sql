CREATE TABLE twitch_attempts (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 generation TEXT NOT NULL,
 client_hash TEXT NOT NULL,
 device BLOB NOT NULL,
 expires_at INTEGER NOT NULL,
 next_poll INTEGER NOT NULL,
 interval INTEGER NOT NULL,
 error TEXT
);
CREATE TABLE twitch_sync (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 generation TEXT NOT NULL,
 next_run INTEGER NOT NULL DEFAULT 0,
 last_turn INTEGER NOT NULL DEFAULT 0,
 validated_at INTEGER NOT NULL DEFAULT 0,
 last_complete INTEGER,
 cursor TEXT NOT NULL DEFAULT '',
 snapshot TEXT NOT NULL DEFAULT '',
 pages INTEGER NOT NULL DEFAULT 0,
 failures INTEGER NOT NULL DEFAULT 0,
 error TEXT
);
CREATE TABLE twitch_streams (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 channel_id TEXT NOT NULL,
 login TEXT NOT NULL,
 display_name TEXT NOT NULL,
 title TEXT NOT NULL,
 category TEXT NOT NULL,
 viewers INTEGER NOT NULL,
 started_at TEXT NOT NULL,
 snapshot TEXT NOT NULL,
 active INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(user_id,channel_id)
);
