CREATE TABLE library_roots (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('movies','shows','music')),
 path TEXT NOT NULL UNIQUE, automatic_unmanaged_deletion INTEGER NOT NULL DEFAULT 0,
 last_scan INTEGER, scan_error TEXT
);
CREATE TABLE media (
 id TEXT PRIMARY KEY, root_id TEXT NOT NULL REFERENCES library_roots(id),
 kind TEXT NOT NULL CHECK(kind IN ('movie','show','season','episode','artist','album','track')),
 parent_id TEXT REFERENCES media(id), evidence_key TEXT NOT NULL, title TEXT NOT NULL,
 sort_number INTEGER, year INTEGER, metadata TEXT NOT NULL DEFAULT '{}', overrides TEXT NOT NULL DEFAULT '{}',
 created_at INTEGER NOT NULL, UNIQUE(root_id,kind,evidence_key)
);
CREATE INDEX media_parent ON media(parent_id);
CREATE TABLE media_files (
 id TEXT PRIMARY KEY, root_id TEXT NOT NULL REFERENCES library_roots(id), path TEXT NOT NULL UNIQUE,
 generation TEXT NOT NULL, size INTEGER NOT NULL, modified TEXT NOT NULL, fingerprint TEXT NOT NULL,
 probe TEXT NOT NULL, edition TEXT NOT NULL DEFAULT '', present INTEGER NOT NULL DEFAULT 1,
 ownership TEXT NOT NULL DEFAULT 'unresolved' CHECK(ownership IN ('managed','unmanaged','unresolved','ambiguous')),
 scanned_at INTEGER NOT NULL
);
CREATE TABLE media_sources (media_id TEXT NOT NULL REFERENCES media(id), file_id TEXT NOT NULL REFERENCES media_files(id), PRIMARY KEY(media_id,file_id));
CREATE TABLE provider_ids (media_id TEXT NOT NULL REFERENCES media(id), provider TEXT NOT NULL, external_id TEXT NOT NULL, coordinates TEXT, mapping_state TEXT NOT NULL DEFAULT 'unresolved', PRIMARY KEY(media_id,provider,external_id));
