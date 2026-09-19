CREATE TABLE media_segments (
 id TEXT PRIMARY KEY, media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
 file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE, generation TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('Intro','Recap','Credits','Preview')),
 start REAL NOT NULL CHECK(start>=0), end REAL NOT NULL CHECK(end>start),
 source TEXT NOT NULL CHECK(source IN ('manual','local','theintrodb')),
 confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1), created_at INTEGER NOT NULL
);
CREATE INDEX media_segments_source ON media_segments(media_id,file_id,generation,source);
CREATE TABLE segment_overrides (
 media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
 file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
 generation TEXT NOT NULL, PRIMARY KEY(media_id,file_id,generation)
);
CREATE TABLE segment_preferences (
 user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 value TEXT NOT NULL
);
CREATE TABLE segment_fingerprints (
 file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE, generation TEXT NOT NULL,
 window TEXT NOT NULL, offset REAL NOT NULL, hashes TEXT NOT NULL,
 PRIMARY KEY(file_id,generation,window)
);
CREATE TABLE segment_analysis (
 media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
 file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE, generation TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed')),
 requested_at INTEGER NOT NULL, completed_at INTEGER, error TEXT,
 PRIMARY KEY(media_id,file_id,generation)
);
INSERT INTO settings VALUES ('segments.config','{"local":true,"external":false}');
