-- Schema 2 to 3. Runs inside the opening transaction with foreign keys off.

-- YouTube sync stops paging a channel's uploads at a stored video only once the channel's
-- backfill completed. Existing channels backfill once more after the upgrade.
ALTER TABLE youtube_subscriptions ADD COLUMN backfilled INTEGER NOT NULL DEFAULT 0 CHECK(backfilled IN (0,1));
