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
