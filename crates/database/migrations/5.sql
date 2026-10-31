-- Schema 4 to 5. Runs inside the opening transaction with foreign keys off.

-- The movies auto-delete policy chooses whether a deleted movie is added to Radarr's
-- exclusion list, as it always was until now, and whether it also leaves Radarr. A deleted
-- season is no longer excluded.
ALTER TABLE retention_policies ADD COLUMN exclude_from_lists INTEGER NOT NULL DEFAULT 0 CHECK(exclude_from_lists IN (0,1));
ALTER TABLE retention_policies ADD COLUMN remove_from_manager INTEGER NOT NULL DEFAULT 0 CHECK(remove_from_manager IN (0,1));
UPDATE retention_policies SET exclude_from_lists=1 WHERE domain='movies';
