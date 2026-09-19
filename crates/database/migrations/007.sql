CREATE TABLE compat_audio_playbacks (
    auth_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    playback_id TEXT NOT NULL REFERENCES playback_sessions(id) ON DELETE CASCADE,
    PRIMARY KEY(auth_session_id, media_id)
);
