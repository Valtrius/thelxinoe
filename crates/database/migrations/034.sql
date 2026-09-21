-- Snapshot identities are independent of the media files and login sessions:
-- removing a file, disconnecting a provider or signing out must not erase stats.
CREATE VIEW statistics_metadata AS
SELECT NULL AS user_id, m.id AS media_id,
       CASE m.kind WHEN 'movie' THEN 'movies' WHEN 'episode' THEN 'shows' ELSE 'music' END AS platform,
       m.title AS media_title,
       COALESCE(grandparent.id,parent.id,m.id) AS channel_id,
       COALESCE(grandparent.title,parent.title,m.title) AS channel_name,
       CASE WHEN m.kind='track' THEN COALESCE(parent.title,'') ELSE '' END AS category_name,
       m.kind AS content_type
FROM media_cards m
LEFT JOIN media_cards parent ON parent.id=m.parent_id
LEFT JOIN media_cards grandparent ON grandparent.id=parent.parent_id
WHERE m.kind IN ('movie','episode','track')
UNION ALL
SELECT user_id,'youtube:'||video_id,'youtube',title,channel_id,channel_title,'',
       CASE WHEN broadcast='live' THEN 'live' WHEN broadcast='upcoming' THEN 'upcoming'
            WHEN is_short=1 THEN 'short' WHEN broadcast='replay' THEN 'live_replay' ELSE 'upload' END
FROM youtube_videos
UNION ALL
SELECT user_id,'twitch:'||channel_id,'twitch',title,channel_id,display_name,category,'live'
FROM twitch_streams
UNION ALL
SELECT user_id,'kick:'||slug,'kick',title,slug,slug,category,'live'
FROM kick_channels;

CREATE TABLE playback_statistics (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
    media_id TEXT NOT NULL,
    media_title TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    category_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    first_played_at INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL,
    duration REAL NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(user_id,platform,media_id)
);
CREATE TABLE playback_activity (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    bucket_started_at INTEGER NOT NULL,
    platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
    media_id TEXT NOT NULL,
    active_seconds REAL NOT NULL CHECK(active_seconds>=0),
    estimated_seconds REAL NOT NULL DEFAULT 0 CHECK(estimated_seconds>=0),
    media_title TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    category_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    PRIMARY KEY(user_id,bucket_started_at,platform,media_id)
) WITHOUT ROWID;
CREATE INDEX playback_activity_date ON playback_activity(bucket_started_at,platform);
CREATE TABLE playback_activity_clocks (
    playback_id TEXT PRIMARY KEY REFERENCES playback_sessions(id) ON DELETE CASCADE,
    reported_at_ms INTEGER NOT NULL,
    client_active_seconds REAL
);

-- Old history has session totals, not minute samples. Keep those totals, flag
-- their approximate time attribution explicitly, and never manufacture detail.
CREATE TEMP VIEW statistics_legacy AS
SELECT h.user_id,h.media_id,h.title,h.started_at,h.updated_at,h.position,h.duration,h.played_seconds,h.state,
       CASE h.kind WHEN 'movie' THEN 'movies' WHEN 'episode' THEN 'shows' ELSE 'music' END AS platform
FROM (SELECT p.*,c.title,c.kind FROM playback_history p JOIN media_cards c ON c.id=p.media_id) h
UNION ALL
SELECT user_id,'youtube:'||video_id,title,started_at,updated_at,position,duration,played_seconds,state,'youtube'
FROM youtube_history
UNION ALL
SELECT user_id,media_id,title,started_at,updated_at,position,0,played_seconds,state,
       CASE WHEN media_id LIKE 'twitch:%' THEN 'twitch' ELSE 'kick' END
FROM live_history;
INSERT INTO playback_statistics
SELECT h.user_id,h.platform,h.media_id,COALESCE(m.media_title,h.title),
       COALESCE(NULLIF(m.channel_id,''),h.media_id),COALESCE(NULLIF(m.channel_name,''),h.title),
       COALESCE(m.category_name,''),COALESCE(m.content_type,CASE WHEN h.platform IN ('twitch','kick') THEN 'live' ELSE h.platform END),
       MIN(h.started_at),MAX(h.updated_at),MAX(h.duration),
       MAX(COALESCE(y.watched,l.watched,h.duration>0 AND h.position>=h.duration*0.9))
FROM statistics_legacy h
LEFT JOIN statistics_metadata m ON m.media_id=h.media_id AND (m.user_id=h.user_id OR m.user_id IS NULL)
LEFT JOIN youtube_state y ON y.user_id=h.user_id AND 'youtube:'||y.video_id=h.media_id
LEFT JOIN media_state l ON l.user_id=h.user_id AND l.media_id=h.media_id
WHERE h.played_seconds>0 OR h.state='playing'
GROUP BY h.user_id,h.platform,h.media_id;
INSERT INTO playback_activity
SELECT h.user_id,h.started_at-h.started_at%60,h.platform,h.media_id,
       SUM(h.played_seconds),SUM(h.played_seconds),s.media_title,s.channel_id,s.channel_name,s.category_name,s.content_type
FROM statistics_legacy h
JOIN playback_statistics s ON s.user_id=h.user_id AND s.platform=h.platform AND s.media_id=h.media_id
WHERE h.played_seconds>0
GROUP BY h.user_id,h.started_at-h.started_at%60,h.platform,h.media_id;
DROP VIEW statistics_legacy;

CREATE TRIGGER statistics_media_watched_insert AFTER INSERT ON media_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;
CREATE TRIGGER statistics_media_watched_update AFTER UPDATE OF watched ON media_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;
CREATE TRIGGER statistics_youtube_watched_insert AFTER INSERT ON youtube_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id='youtube:'||NEW.video_id;
END;
CREATE TRIGGER statistics_youtube_watched_update AFTER UPDATE OF watched ON youtube_state BEGIN
    UPDATE playback_statistics SET completed=NEW.watched WHERE user_id=NEW.user_id AND media_id='youtube:'||NEW.video_id;
END;
