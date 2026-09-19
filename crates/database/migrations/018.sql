CREATE TABLE stack_provisions (
 id TEXT PRIMARY KEY, kind TEXT NOT NULL UNIQUE, actor_id TEXT NOT NULL REFERENCES users(id),
 host_port INTEGER NOT NULL, credential BLOB NOT NULL,
 state TEXT NOT NULL, container_id TEXT, service_id TEXT, error TEXT,
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
