CREATE TABLE manager_bindings (
 file_id TEXT NOT NULL REFERENCES media_files(id), service_id TEXT NOT NULL REFERENCES manager_services(id),
 generation TEXT NOT NULL, service_generation TEXT NOT NULL, manager_file_id INTEGER NOT NULL,
 entity_id INTEGER NOT NULL, manager_path TEXT NOT NULL, external_id TEXT NOT NULL,
 members TEXT NOT NULL, checked_at INTEGER NOT NULL,
 PRIMARY KEY(file_id,service_id)
);
CREATE TABLE manager_reconciliations (
 service_id TEXT PRIMARY KEY REFERENCES manager_services(id), generation TEXT NOT NULL,
 checked_at INTEGER NOT NULL, error TEXT
);
CREATE TABLE media_operations (
 id TEXT PRIMARY KEY, actor_id TEXT REFERENCES users(id) ON DELETE SET NULL,
 media_id TEXT NOT NULL REFERENCES media(id), action TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','executing','complete','blocked','uncertain')),
 targets TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, error TEXT
);
CREATE TABLE media_protection (
 media_id TEXT PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
 keep INTEGER NOT NULL DEFAULT 0 CHECK(keep IN (0,1))
);
