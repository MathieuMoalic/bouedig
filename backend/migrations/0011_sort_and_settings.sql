-- Sort support: recipes remember when they last changed (Recently updated
-- order), and a tiny key-value store keeps UI preferences (the chosen sort)
-- in sync between the web and Android clients. SQLite rejects non-constant
-- ALTER defaults, so existing rows are backfilled instead.
ALTER TABLE recipes ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE recipes SET updated_at = datetime('now') WHERE updated_at = '';

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
