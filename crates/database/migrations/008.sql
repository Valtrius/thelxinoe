CREATE TABLE online_accounts (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 provider TEXT NOT NULL CHECK(provider IN ('youtube','twitch','kick')),
 generation TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'disconnected' CHECK(status IN ('disconnected','connected','reconnect_required')),
 display_name TEXT NOT NULL DEFAULT '',
 external_id TEXT NOT NULL DEFAULT '',
 credential BLOB,
 expires_at INTEGER NOT NULL DEFAULT 0,
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(user_id,provider)
);
CREATE TABLE oauth_attempts (
 state_hash TEXT PRIMARY KEY,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 provider TEXT NOT NULL,
 generation TEXT NOT NULL,
 browser_hash TEXT NOT NULL,
 verifier BLOB NOT NULL,
 redirect_uri TEXT NOT NULL,
 client_hash TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX oauth_expiry ON oauth_attempts(expires_at);
CREATE TABLE youtube_quota (
 day TEXT PRIMARY KEY,
 used INTEGER NOT NULL DEFAULT 0 CHECK(used>=0),
 blocked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE youtube_quota_users (
 day TEXT NOT NULL REFERENCES youtube_quota(day) ON DELETE CASCADE,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 used INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(day,user_id)
);
CREATE TABLE youtube_sync (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 generation TEXT NOT NULL,
 cursor TEXT NOT NULL DEFAULT '{}',
 next_run INTEGER NOT NULL DEFAULT 0,
 last_turn INTEGER NOT NULL DEFAULT 0,
 last_complete INTEGER,
 failures INTEGER NOT NULL DEFAULT 0,
 error TEXT
);
CREATE INDEX youtube_sync_ready ON youtube_sync(next_run,last_turn);
CREATE TABLE youtube_subscriptions (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 channel_id TEXT NOT NULL,
 title TEXT NOT NULL,
 uploads TEXT,
 snapshot TEXT NOT NULL,
 active INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(user_id,channel_id)
);
CREATE TABLE youtube_videos (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 video_id TEXT NOT NULL,
 channel_id TEXT NOT NULL DEFAULT '',
 title TEXT NOT NULL,
 channel_title TEXT NOT NULL DEFAULT '',
 published_at INTEGER NOT NULL DEFAULT 0,
 duration INTEGER,
 broadcast TEXT NOT NULL DEFAULT 'none' CHECK(broadcast IN ('none','live','upcoming','replay')),
 privacy TEXT NOT NULL DEFAULT 'unknown',
 available INTEGER NOT NULL DEFAULT 1,
 is_short INTEGER,
 short_checked INTEGER NOT NULL DEFAULT 0,
 metadata_at INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(user_id,video_id)
);
CREATE INDEX youtube_feed ON youtube_videos(user_id,published_at DESC);
CREATE TABLE youtube_state (
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 video_id TEXT NOT NULL,
 watchlist INTEGER NOT NULL DEFAULT 0,
 pinned INTEGER NOT NULL DEFAULT 0,
 watched INTEGER NOT NULL DEFAULT 0,
 position REAL NOT NULL DEFAULT 0 CHECK(position>=0),
 added_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(user_id,video_id),
 FOREIGN KEY(user_id,video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE
);
