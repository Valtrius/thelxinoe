-- Schema 3 to 4. Runs inside the opening transaction with foreign keys off.

-- A failed YouTube download counts its automatic retries and records when the next one is due.
ALTER TABLE youtube_downloads ADD COLUMN retries INTEGER NOT NULL DEFAULT 0;
ALTER TABLE youtube_downloads ADD COLUMN retry_at INTEGER;
