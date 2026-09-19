ALTER TABLE service_updates ADD COLUMN automatic INTEGER NOT NULL DEFAULT 0 CHECK(automatic IN (0,1));
