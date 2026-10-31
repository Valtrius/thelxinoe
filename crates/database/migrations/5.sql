-- Schema 4 to 5. Runs inside the opening transaction with foreign keys off.

-- An auto-delete policy chooses whether its deletions exclude the title from its manager's
-- lists, and whether a deleted movie also leaves Radarr.
ALTER TABLE retention_policies ADD COLUMN exclude_from_lists INTEGER NOT NULL DEFAULT 1 CHECK(exclude_from_lists IN (0,1));
ALTER TABLE retention_policies ADD COLUMN remove_from_manager INTEGER NOT NULL DEFAULT 0 CHECK(remove_from_manager IN (0,1));
