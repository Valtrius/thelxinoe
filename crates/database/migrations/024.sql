CREATE TABLE notifications (
 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 source TEXT NOT NULL, severity TEXT NOT NULL, message TEXT NOT NULL,
 created_at INTEGER NOT NULL, read_at INTEGER, UNIQUE(user_id,source)
);
CREATE INDEX notification_user ON notifications(user_id,created_at DESC);
