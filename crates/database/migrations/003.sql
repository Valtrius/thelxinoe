CREATE TABLE provider_episodes (
 provider TEXT NOT NULL, series_id TEXT NOT NULL, episode_id TEXT NOT NULL,
 season_number INTEGER NOT NULL, episode_number INTEGER NOT NULL, metadata TEXT NOT NULL,
 PRIMARY KEY(provider,episode_id)
);
CREATE TABLE local_trailers (
 media_id TEXT NOT NULL REFERENCES media(id), file_id TEXT NOT NULL REFERENCES media_files(id),
 PRIMARY KEY(media_id,file_id)
);
CREATE TABLE episode_mappings (
 media_id TEXT NOT NULL REFERENCES media(id), provider TEXT NOT NULL, episode_id TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('confirmed','unresolved','complex')),
 PRIMARY KEY(media_id,provider,episode_id),
 FOREIGN KEY(provider,episode_id) REFERENCES provider_episodes(provider,episode_id)
);
