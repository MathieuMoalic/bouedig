-- Manual category choices win forever: a pinned cache entry is written when
-- the user moves an item via the edit sheet and is never overwritten by the
-- auto-classifier.
ALTER TABLE ingredient_categories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
