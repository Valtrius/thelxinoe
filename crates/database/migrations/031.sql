-- Integrations now record one shared /media bind source, not path translations.
-- Previous JSON evidence remains different from a host path and requires reconnecting.
ALTER TABLE manager_services RENAME COLUMN mappings TO media_source;
ALTER TABLE support_services RENAME COLUMN mappings TO media_source;
