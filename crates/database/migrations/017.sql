CREATE TABLE support_services (
 id TEXT PRIMARY KEY, name TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('bazarr','prowlarr','nzbget')),
 container_id TEXT NOT NULL UNIQUE, port INTEGER NOT NULL,
 generation TEXT NOT NULL, credential BLOB NOT NULL, mappings TEXT NOT NULL,
 native_url TEXT NOT NULL DEFAULT '', version TEXT NOT NULL,
 checked_at INTEGER NOT NULL, error TEXT
);
