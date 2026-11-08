-- Schema 7 to 8. Runs inside the opening transaction with foreign keys off.

-- A scan finds the entries each changed file stands for by the file, in the order they
-- were linked.
CREATE INDEX media_sources_file ON media_sources(file_id);
