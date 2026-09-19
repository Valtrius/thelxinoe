-- Preserve the most recent operational download toggle from the initial worker
-- slice, then use the existing application setting everywhere.
INSERT INTO settings(key,value)
SELECT 'youtube_downloads',value FROM settings WHERE key='online.youtube.downloads'
ON CONFLICT(key) DO UPDATE SET value=excluded.value;
DELETE FROM settings WHERE key='online.youtube.downloads';
