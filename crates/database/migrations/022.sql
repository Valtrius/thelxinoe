ALTER TABLE media_state ADD COLUMN watched_revision INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER retention_watched_revision AFTER UPDATE OF watched ON media_state
WHEN OLD.watched != NEW.watched BEGIN
 UPDATE media_state SET watched_revision=OLD.watched_revision+1 WHERE user_id=NEW.user_id AND media_id=NEW.media_id;
END;
CREATE TABLE retention_policies (
 domain TEXT PRIMARY KEY CHECK(domain IN ('movies','shows')),
 enabled INTEGER NOT NULL DEFAULT 0,
 grace_seconds INTEGER NOT NULL DEFAULT 604800 CHECK(grace_seconds BETWEEN 0 AND 31536000),
 exclude_specials INTEGER NOT NULL DEFAULT 1,
 trigger_users TEXT NOT NULL DEFAULT '[]',
 updated_at INTEGER NOT NULL
);
INSERT INTO retention_policies(domain,updated_at) VALUES ('movies',0),('shows',0);
CREATE TABLE retention_candidates (
 id TEXT PRIMARY KEY,
 media_id TEXT NOT NULL REFERENCES media(id),
 operation_id TEXT REFERENCES media_operations(id),
 stamp TEXT NOT NULL,
 trigger_user TEXT REFERENCES users(id) ON DELETE SET NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','executing','complete','cancelled','blocked')),
 eligible_at INTEGER NOT NULL,
 due_at INTEGER NOT NULL,
 error TEXT,
 UNIQUE(media_id,stamp)
);
CREATE UNIQUE INDEX retention_pending_media ON retention_candidates(media_id) WHERE state IN ('pending','executing');
CREATE TABLE retention_exclusions (
 service_id TEXT NOT NULL REFERENCES manager_services(id),
 external_id TEXT NOT NULL,
 exclusion_id INTEGER NOT NULL,
 created_here INTEGER NOT NULL,
 PRIMARY KEY(service_id,external_id)
);
