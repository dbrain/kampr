CREATE TABLE recovery_codes (
    hash       TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    used_at    INTEGER,
    attempts   INTEGER NOT NULL DEFAULT 0
) STRICT;
