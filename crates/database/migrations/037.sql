CREATE TABLE playback_history_next (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 playback_id TEXT NOT NULL,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 platform TEXT NOT NULL CHECK(platform IN ('youtube','twitch','kick','movies','shows','music')),
 media_id TEXT NOT NULL,
 media_title TEXT NOT NULL,
 content_type TEXT NOT NULL,
 edition TEXT NOT NULL DEFAULT '',
 device_name TEXT NOT NULL,
 started_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 ended_at INTEGER,
 position REAL NOT NULL,
 duration REAL NOT NULL DEFAULT 0,
 played_seconds REAL NOT NULL DEFAULT 0,
 state TEXT NOT NULL CHECK(state IN ('playing','paused','stopped')),
 UNIQUE(playback_id,platform)
);

INSERT INTO playback_history_next(
 playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,
 started_at,updated_at,ended_at,position,duration,played_seconds,state
)
SELECT h.playback_id,h.user_id,
       CASE c.kind WHEN 'movie' THEN 'movies' WHEN 'episode' THEN 'shows' WHEN 'track' THEN 'music' END,
       h.media_id,c.title,c.kind,h.edition,h.device_name,
       h.started_at,h.updated_at,h.ended_at,h.position,h.duration,h.played_seconds,h.state
FROM playback_history h
JOIN media_cards c ON c.id=h.media_id;

INSERT INTO playback_history_next(
 playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,
 started_at,updated_at,ended_at,position,duration,played_seconds,state
)
SELECT h.playback_id,h.user_id,'youtube','youtube:'||h.video_id,h.title,
       COALESCE(
         CASE
           WHEN v.broadcast='live' THEN 'live'
           WHEN v.broadcast='upcoming' THEN 'upcoming'
           WHEN v.is_short=1 THEN 'short'
           WHEN v.broadcast='replay' THEN 'live_replay'
           ELSE 'upload'
         END,
         'upload'
       ),
       'public',h.device_name,h.started_at,h.updated_at,
       CASE WHEN h.state='stopped' THEN h.updated_at ELSE NULL END,
       h.position,h.duration,h.played_seconds,h.state
FROM youtube_history h
LEFT JOIN youtube_videos v ON v.user_id=h.user_id AND v.video_id=h.video_id;

INSERT INTO playback_history_next(
 playback_id,user_id,platform,media_id,media_title,content_type,edition,device_name,
 started_at,updated_at,ended_at,position,duration,played_seconds,state
)
SELECT playback_id,user_id,
       CASE
         WHEN media_id LIKE 'twitch:%' THEN 'twitch'
         WHEN media_id LIKE 'kick:%' THEN 'kick'
       END,
       media_id,title,'live','live',device_name,started_at,updated_at,
       CASE WHEN state='stopped' THEN updated_at ELSE NULL END,
       position,0,played_seconds,state
FROM live_history;

DROP TABLE playback_history;
DROP TABLE youtube_history;
DROP TABLE live_history;
ALTER TABLE playback_history_next RENAME TO playback_history;
CREATE INDEX playback_history_time ON playback_history(started_at DESC,id DESC);
CREATE INDEX playback_history_user_platform_time
 ON playback_history(user_id,platform,started_at DESC,id DESC);

ALTER TABLE playback_sessions ADD COLUMN reported_at_ms INTEGER;
ALTER TABLE playback_sessions ADD COLUMN client_active_seconds REAL;
UPDATE playback_sessions
SET reported_at_ms=(
      SELECT reported_at_ms
      FROM playback_activity_clocks c
      WHERE c.playback_id=playback_sessions.id
    ),
    client_active_seconds=(
      SELECT client_active_seconds
      FROM playback_activity_clocks c
      WHERE c.playback_id=playback_sessions.id
    )
WHERE EXISTS(
  SELECT 1 FROM playback_activity_clocks c WHERE c.playback_id=playback_sessions.id
);
DROP TABLE playback_activity_clocks;
