-- Older versions did not distinguish an explicit UTC choice from the default.
-- Preserve other personal choices and let untouched/default UTC accounts inherit.
ALTER TABLE users ADD COLUMN timezone_inherited INTEGER NOT NULL DEFAULT 1
 CHECK(timezone_inherited IN (0,1));
UPDATE users SET timezone_inherited=0 WHERE timezone<>'UTC';

CREATE VIEW user_profiles AS
WITH defaults AS (
 SELECT COALESCE((SELECT value FROM settings WHERE key='timezone'),'UTC') AS server_timezone
)
SELECT u.id,u.username,u.role,
 CASE WHEN u.timezone_inherited=1 THEN d.server_timezone ELSE u.timezone END AS timezone,
 CASE WHEN u.timezone_inherited=0 THEN u.timezone END AS timezone_override,
 d.server_timezone
FROM users u CROSS JOIN defaults d;
