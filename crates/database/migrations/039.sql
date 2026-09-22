ALTER TABLE users ADD COLUMN time_format_inherited INTEGER NOT NULL DEFAULT 1
 CHECK(time_format_inherited IN (0,1));
UPDATE users SET time_format_inherited=0 WHERE time_format<>'24h';
DELETE FROM service_update_policy WHERE service_id='default';

DROP VIEW user_profiles;
CREATE VIEW user_profiles AS
WITH defaults AS (
 SELECT
  COALESCE((SELECT value FROM settings WHERE key='timezone'),'UTC') AS server_timezone,
  COALESCE((SELECT value FROM settings WHERE key='time_format'),'24h') AS server_time_format
)
SELECT u.id,u.username,u.role,
 CASE WHEN u.timezone_inherited=1 THEN d.server_timezone ELSE u.timezone END AS timezone,
 CASE WHEN u.timezone_inherited=0 THEN u.timezone END AS timezone_override,
 d.server_timezone,
 CASE WHEN u.time_format_inherited=1 THEN d.server_time_format ELSE u.time_format END AS time_format,
 CASE WHEN u.time_format_inherited=0 THEN u.time_format END AS time_format_override,
 d.server_time_format
FROM users u CROSS JOIN defaults d;
