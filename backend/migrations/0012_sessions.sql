-- Single-password auth: server-side sessions. The password itself lives in
-- the BOUEDIG_PASSWORD env var; only the issued tokens are stored here.
CREATE TABLE sessions (
    token TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
