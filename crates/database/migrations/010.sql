CREATE TABLE youtube_media (video_id TEXT PRIMARY KEY);
INSERT INTO youtube_media SELECT video_id FROM youtube_downloads;
CREATE TABLE playback_sessions_next (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    auth_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    media_id TEXT REFERENCES media(id) ON DELETE CASCADE,
    file_id TEXT REFERENCES media_files(id),
    generation TEXT NOT NULL,
    edition TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('ready','playing','paused','stopped','failed')),
    mode TEXT NOT NULL CHECK(mode IN ('direct','remux','transcode')),
    options TEXT NOT NULL,
    duration REAL NOT NULL,
    position REAL NOT NULL DEFAULT 0,
    sequence INTEGER NOT NULL DEFAULT -1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    client_id TEXT,
    queue_revision INTEGER,
    queue_index INTEGER,
    youtube_video_id TEXT REFERENCES youtube_media(video_id),
    streaming INTEGER NOT NULL DEFAULT 0 CHECK(streaming IN (0,1)),
    CHECK((media_id IS NOT NULL AND file_id IS NOT NULL AND youtube_video_id IS NULL) OR
          (media_id IS NULL AND file_id IS NULL AND youtube_video_id IS NOT NULL))
);
INSERT INTO playback_sessions_next SELECT *,0 FROM playback_sessions;
CREATE TEMP TABLE saved_compat_playbacks AS SELECT * FROM compat_playbacks;
CREATE TEMP TABLE saved_compat_audio AS SELECT * FROM compat_audio_playbacks;
DROP TABLE compat_playbacks;
DROP TABLE compat_audio_playbacks;
DROP TABLE playback_sessions;
ALTER TABLE playback_sessions_next RENAME TO playback_sessions;
CREATE INDEX playback_active ON playback_sessions(state,updated_at);
CREATE INDEX playback_youtube ON playback_sessions(youtube_video_id,state,updated_at);
CREATE TABLE compat_playbacks (
    playback_id TEXT PRIMARY KEY REFERENCES playback_sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL DEFAULT 0
);
INSERT INTO compat_playbacks SELECT * FROM saved_compat_playbacks;
CREATE TABLE compat_audio_playbacks (
    auth_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    playback_id TEXT NOT NULL REFERENCES playback_sessions(id) ON DELETE CASCADE,
    PRIMARY KEY(auth_session_id,media_id)
);
INSERT INTO compat_audio_playbacks SELECT * FROM saved_compat_audio;
DROP TABLE saved_compat_playbacks;
DROP TABLE saved_compat_audio;
