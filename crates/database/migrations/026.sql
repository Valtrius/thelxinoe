CREATE TABLE ui_preferences (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 value TEXT NOT NULL
);
ALTER TABLE kick_channels ADD COLUMN thumbnail_url TEXT;
ALTER TABLE kick_channels ADD COLUMN started_at TEXT;
ALTER TABLE twitch_streams ADD COLUMN thumbnail_url TEXT;
