ALTER TABLE youtube_downloads ADD COLUMN downloaded_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE youtube_downloads ADD COLUMN total_bytes INTEGER;
ALTER TABLE youtube_downloads ADD COLUMN eta_seconds INTEGER;
ALTER TABLE youtube_downloads ADD COLUMN media_kind TEXT;
