-- User feedback from the Settings form: bug reports, feature ideas and
-- anything else the household wants to wave at the developer. Reports are
-- read (and cleared) from the same Settings screen; `seen` is cosmetic.
CREATE TABLE feedback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,             -- 'bug' | 'feature' | 'other'
    text        TEXT NOT NULL,
    app_version TEXT NOT NULL DEFAULT '',
    seen        INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
