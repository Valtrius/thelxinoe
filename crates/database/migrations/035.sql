CREATE TABLE metadata_bindings (
 media_id TEXT PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
 service_id TEXT NOT NULL REFERENCES manager_services(id),
 service_generation TEXT NOT NULL,
 external_id TEXT NOT NULL,
 manager_entity_id INTEGER,
 refreshed_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE manager_episodes (
 service_id TEXT NOT NULL REFERENCES manager_services(id),
 service_generation TEXT NOT NULL,
 series_external_id TEXT NOT NULL,
 manager_episode_id INTEGER NOT NULL,
 season_number INTEGER NOT NULL,
 episode_number INTEGER NOT NULL, refreshed_at INTEGER NOT NULL,
 metadata TEXT NOT NULL,
 PRIMARY KEY(service_id,service_generation,manager_episode_id)
);

CREATE TABLE manager_episode_mappings (
 media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
 service_id TEXT NOT NULL,
 service_generation TEXT NOT NULL,
 manager_episode_id INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('confirmed','unresolved','complex')),
 PRIMARY KEY(media_id,service_id,service_generation,manager_episode_id),
 FOREIGN KEY(service_id,service_generation,manager_episode_id) REFERENCES manager_episodes(service_id,service_generation,manager_episode_id) ON DELETE CASCADE
);

DROP TABLE episode_mappings;
DROP TABLE provider_episodes;
DROP TABLE provider_ids;
UPDATE media SET metadata='{}';
DELETE FROM settings WHERE key='musicbrainz_contact';
DELETE FROM secrets WHERE scope='provider.tmdb';
