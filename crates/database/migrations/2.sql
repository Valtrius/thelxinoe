-- Schema 1 to 2. Runs inside the opening transaction with foreign keys off.

-- Jobs name the thing they act on in an indexed `subject` instead of deduplicating by a
-- key that stayed unique forever; the checkpoint job and its effects table are gone.
CREATE TABLE jobs_new (
    id TEXT NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    error TEXT,
    created_at INTEGER NOT NULL
) STRICT;
INSERT INTO jobs_new(id,kind,subject,payload,state,attempts,available_at,started_at,completed_at,error,created_at)
SELECT id,kind,CAST(COALESCE(json_extract(payload,'$.root_id'),json_extract(payload,'$.request_id'),json_extract(payload,'$.media_id'),json_extract(payload,'$.id'),'') AS TEXT),payload,state,attempts,available_at,started_at,completed_at,error,created_at
FROM jobs WHERE kind<>'checkpoint';
DROP TABLE job_effects;
DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;
CREATE INDEX jobs_ready ON jobs(state, available_at);
CREATE INDEX jobs_subject ON jobs(kind, subject, created_at);

-- Automatic metadata refreshes stopped auditing themselves; their old rows go too.
DELETE FROM audit WHERE action='metadata.refresh' AND actor_id IS NULL;

-- Attention changes are found by comparing digests kept in memory.
DROP TABLE attention_snapshots;

-- Nothing ever marked a download 'deleting'; removing one deletes its row.
CREATE TABLE youtube_downloads_new (
    video_id TEXT NOT NULL PRIMARY KEY,
    generation TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','waiting_for_space','downloading','ready','extractor_authentication_required','network_blocked','unavailable','failed')),
    tools TEXT NOT NULL,
    path TEXT,
    size INTEGER,
    modified TEXT,
    probe TEXT,
    error TEXT,
    requested_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER,
    eta_seconds INTEGER,
    media_kind TEXT
) STRICT;
INSERT INTO youtube_downloads_new(video_id,generation,state,tools,path,size,modified,probe,error,requested_at,updated_at,completed_at,downloaded_bytes,total_bytes,eta_seconds,media_kind)
SELECT video_id,generation,CASE state WHEN 'deleting' THEN 'failed' ELSE state END,tools,path,size,modified,probe,error,requested_at,updated_at,completed_at,downloaded_bytes,total_bytes,eta_seconds,media_kind
FROM youtube_downloads;
DROP TABLE youtube_downloads;
ALTER TABLE youtube_downloads_new RENAME TO youtube_downloads;

-- The saved music queue was written on every play and never read.
DROP TABLE music_queues;
ALTER TABLE playback_sessions DROP COLUMN client_id;
ALTER TABLE playback_sessions DROP COLUMN queue_revision;
ALTER TABLE playback_sessions DROP COLUMN queue_index;

-- A missing file's scan time is when it went missing; maintenance prunes it a month later.
-- Files already missing start that month at the upgrade.
UPDATE media_files SET scanned_at=CAST(strftime('%s','now') AS INTEGER) WHERE present=0;

-- Playback maintenance closes stale history every ten seconds and reads only open rows.
-- Statistics read each played item from its own source by key instead of through a view.
CREATE INDEX playback_history_open ON playback_history(playback_id) WHERE ended_at IS NULL;
DROP VIEW statistics_metadata;

-- Sessions slide 30 days past their last use, up to 400 days after sign-in. Remembered
-- sessions used to never end; every session gets that bound now.
UPDATE sessions SET expires_at=MIN(expires_at,created_at+400*86400,last_seen+30*86400);

-- Browsers, desktops and media clients that signed in to an account before skip its
-- lockout, so others' failed guesses can't keep the owner out.
CREATE TABLE auth_known_browsers (
    token_hash TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY(token_hash,user_id)
) STRICT;
