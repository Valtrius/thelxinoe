ALTER TABLE online_accounts ADD COLUMN avatar_url TEXT;
ALTER TABLE online_accounts ADD COLUMN profile_checked_at INTEGER NOT NULL DEFAULT 0;
