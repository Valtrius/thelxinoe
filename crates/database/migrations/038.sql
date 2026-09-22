ALTER TABLE users ADD COLUMN time_format TEXT NOT NULL DEFAULT '24h'
 CHECK(time_format IN ('12h','24h'));
