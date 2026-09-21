CREATE TABLE user_avatars (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    image TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
