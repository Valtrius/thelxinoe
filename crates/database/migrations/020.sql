CREATE TABLE service_update_policy (
 service_id TEXT PRIMARY KEY,
 policy TEXT NOT NULL CHECK(policy IN ('automatic','notify','manual','inherit')),
 window_start INTEGER NOT NULL DEFAULT 0 CHECK(window_start BETWEEN 0 AND 23),
 window_end INTEGER NOT NULL DEFAULT 0 CHECK(window_end BETWEEN 0 AND 23),
 checked_at INTEGER NOT NULL DEFAULT 0,
 candidate TEXT,
 error TEXT
);
INSERT INTO service_update_policy(service_id,policy) VALUES ('default','notify');
CREATE TABLE service_updates (
 id TEXT PRIMARY KEY,
 service_id TEXT NOT NULL,
 actor_id TEXT REFERENCES users(id) ON DELETE SET NULL,
 state TEXT NOT NULL,
 candidate TEXT,
 error TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
