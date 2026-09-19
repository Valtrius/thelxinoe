CREATE TABLE users (
 id TEXT PRIMARY KEY, username TEXT NOT NULL COLLATE NOCASE UNIQUE,
 password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK(role IN ('admin','user')),
 timezone TEXT NOT NULL DEFAULT 'UTC', created_at INTEGER NOT NULL
);
CREATE TABLE sessions (
 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 token_hash TEXT NOT NULL UNIQUE, transport TEXT NOT NULL CHECK(transport IN ('web','device','jellyfin')),
 name TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, last_seen INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE TABLE secrets (scope TEXT PRIMARY KEY, ciphertext BLOB NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE audit (id INTEGER PRIMARY KEY, actor_id TEXT, action TEXT NOT NULL, target TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE jobs (
 id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload TEXT NOT NULL, dedupe_key TEXT NOT NULL UNIQUE,
 state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed')),
 attempts INTEGER NOT NULL DEFAULT 0, available_at INTEGER NOT NULL, started_at INTEGER,
 completed_at INTEGER, error TEXT, created_at INTEGER NOT NULL
);
CREATE INDEX jobs_ready ON jobs(state, available_at);
CREATE TABLE job_effects (job_id TEXT PRIMARY KEY REFERENCES jobs(id), value TEXT NOT NULL);
CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE playback_grants (token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, resource TEXT NOT NULL, expires_at INTEGER NOT NULL);
CREATE TABLE login_attempts (address TEXT PRIMARY KEY, count INTEGER NOT NULL, window_start INTEGER NOT NULL);
