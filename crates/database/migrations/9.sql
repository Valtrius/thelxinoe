-- Schema 8 to 9. Runs inside the opening transaction with foreign keys off.

-- A watchlist entry records when a list that removes watched videos drops it, so an entry
-- added after its video was watched stays listed.
ALTER TABLE youtube_watchlist_items ADD COLUMN remove_at INTEGER;

-- Entries of watched videos keep the undo window schema 8 gave them.
UPDATE youtube_watchlist_items SET remove_at=(
    SELECT MAX(s.updated_at,youtube_watchlist_items.added_at)+5 FROM youtube_state s
    WHERE s.user_id=youtube_watchlist_items.user_id AND s.video_id=youtube_watchlist_items.video_id AND s.watched=1
);
