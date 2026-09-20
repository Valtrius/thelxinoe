CREATE TABLE youtube_watchlists (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 name TEXT NOT NULL,
 is_default INTEGER NOT NULL DEFAULT 0,
 auto_download INTEGER NOT NULL DEFAULT 0,
 auto_remove_watched INTEGER NOT NULL DEFAULT 0,
 sort_mode TEXT NOT NULL DEFAULT 'manual',
 sort_direction TEXT NOT NULL DEFAULT 'asc',
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(user_id,id)
);
CREATE UNIQUE INDEX youtube_default_watchlist ON youtube_watchlists(user_id) WHERE is_default=1;
CREATE TABLE youtube_watchlist_items (
 user_id TEXT NOT NULL,
 watchlist_id INTEGER NOT NULL,
 video_id TEXT NOT NULL,
 manual_position REAL NOT NULL,
 added_at INTEGER NOT NULL,
 PRIMARY KEY(watchlist_id,video_id),
 FOREIGN KEY(user_id,watchlist_id) REFERENCES youtube_watchlists(user_id,id) ON DELETE CASCADE,
 FOREIGN KEY(user_id,video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE
);
INSERT INTO youtube_watchlists(user_id,name,is_default,created_at,updated_at)
 SELECT id,'Watch Later',1,unixepoch(),unixepoch() FROM users;
INSERT INTO youtube_watchlist_items
 SELECT s.user_id,w.id,s.video_id,-s.added_at,s.added_at FROM youtube_state s
 JOIN youtube_watchlists w ON w.user_id=s.user_id AND w.is_default=1 WHERE s.watchlist=1;
CREATE TRIGGER youtube_watchlist_item_added AFTER INSERT ON youtube_watchlist_items BEGIN
 INSERT INTO youtube_state(user_id,video_id,watchlist,added_at,updated_at)
 VALUES(NEW.user_id,NEW.video_id,1,NEW.added_at,unixepoch())
 ON CONFLICT(user_id,video_id) DO UPDATE SET watchlist=1,updated_at=unixepoch();
END;
CREATE TRIGGER youtube_watchlist_item_removed AFTER DELETE ON youtube_watchlist_items BEGIN
 UPDATE youtube_state SET watchlist=EXISTS(SELECT 1 FROM youtube_watchlist_items
 WHERE user_id=OLD.user_id AND video_id=OLD.video_id),updated_at=unixepoch()
 WHERE user_id=OLD.user_id AND video_id=OLD.video_id;
END;
ALTER TABLE youtube_subscriptions ADD COLUMN thumbnail_url TEXT;
ALTER TABLE youtube_videos ADD COLUMN scheduled_start TEXT;
ALTER TABLE youtube_videos ADD COLUMN actual_start TEXT;
ALTER TABLE youtube_videos ADD COLUMN actual_end TEXT;
ALTER TABLE twitch_streams ADD COLUMN profile_image_url TEXT;
ALTER TABLE kick_channels ADD COLUMN display_name TEXT;
ALTER TABLE kick_channels ADD COLUMN profile_image_url TEXT;
ALTER TABLE kick_channels ADD COLUMN language TEXT;
ALTER TABLE kick_channels ADD COLUMN mature INTEGER NOT NULL DEFAULT 0;
ALTER TABLE kick_channels ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
CREATE TABLE youtube_download_suppressed (video_id TEXT PRIMARY KEY);
