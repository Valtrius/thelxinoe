CREATE TABLE manager_services (
 id TEXT PRIMARY KEY, name TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('radarr','sonarr','lidarr')),
 container_id TEXT NOT NULL UNIQUE, port INTEGER NOT NULL,
 generation TEXT NOT NULL, credential BLOB NOT NULL,
 mappings TEXT NOT NULL, defaults TEXT NOT NULL DEFAULT '{}',
 version TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
 checked_at INTEGER NOT NULL, error TEXT
);
CREATE TABLE acquisition_users (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 auto_approve INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE acquisition_requests (
 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 service_id TEXT NOT NULL REFERENCES manager_services(id),
 generation TEXT NOT NULL, external_id TEXT NOT NULL, title TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','approved','adding','searching','requested','available','failed','denied','cancelled','uncertain')),
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 manager_id INTEGER, error TEXT
);
CREATE UNIQUE INDEX acquisition_own_active ON acquisition_requests(user_id,service_id,external_id) WHERE state NOT IN ('denied','cancelled','failed');
