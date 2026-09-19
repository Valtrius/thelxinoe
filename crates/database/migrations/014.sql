CREATE TABLE kick_channels (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 slug TEXT NOT NULL, generation TEXT NOT NULL,
 title TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT '',
 live INTEGER, viewers INTEGER NOT NULL DEFAULT 0,
 updated_at INTEGER NOT NULL DEFAULT 0, next_run INTEGER NOT NULL DEFAULT 0,
 last_turn INTEGER NOT NULL DEFAULT 0, failures INTEGER NOT NULL DEFAULT 0, error TEXT,
 PRIMARY KEY(user_id,slug)
);
CREATE INDEX kick_ready ON kick_channels(next_run,last_turn);
