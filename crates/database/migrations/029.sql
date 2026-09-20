-- Jellyfin clients require UUIDs. Provider IDs remain private to their owner.
CREATE TABLE compat_online_items (
 id TEXT PRIMARY KEY,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 youtube_video_id TEXT,
 twitch_channel_id TEXT,
 kick_slug TEXT,
 watchlist_id INTEGER,
 favorite INTEGER NOT NULL DEFAULT 0,
 CHECK ((youtube_video_id IS NOT NULL) + (twitch_channel_id IS NOT NULL)
      + (kick_slug IS NOT NULL) + (watchlist_id IS NOT NULL) = 1),
 UNIQUE(user_id,youtube_video_id),
 UNIQUE(user_id,twitch_channel_id),
 UNIQUE(user_id,kick_slug),
 UNIQUE(user_id,watchlist_id),
 FOREIGN KEY(user_id,youtube_video_id) REFERENCES youtube_videos(user_id,video_id) ON DELETE CASCADE,
 FOREIGN KEY(user_id,twitch_channel_id) REFERENCES twitch_streams(user_id,channel_id) ON DELETE CASCADE,
 FOREIGN KEY(user_id,kick_slug) REFERENCES kick_channels(user_id,slug) ON DELETE CASCADE,
 FOREIGN KEY(user_id,watchlist_id) REFERENCES youtube_watchlists(user_id,id) ON DELETE CASCADE
);
