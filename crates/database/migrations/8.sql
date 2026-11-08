-- Schema 7 to 8. Runs inside the opening transaction with foreign keys off.

-- A scan finds the entries each changed file stands for by the file, in the order they
-- were linked.
CREATE INDEX media_sources_file ON media_sources(file_id);

-- Music requests can monitor an artist's new releases, and name their artist.
ALTER TABLE acquisition_requests ADD COLUMN target TEXT NOT NULL DEFAULT 'item' CHECK(target IN ('item','artist'));
ALTER TABLE acquisition_requests ADD COLUMN artist TEXT;
ALTER TABLE acquisition_requests ADD COLUMN artist_id TEXT;

-- A user monitors an artist again once their earlier request has reached Lidarr.
DROP INDEX acquisition_own_active;
CREATE UNIQUE INDEX acquisition_own_active ON acquisition_requests(user_id,service_id,external_id) WHERE state NOT IN ('denied','cancelled','failed') AND NOT (target='artist' AND state IN ('requested','available'));
