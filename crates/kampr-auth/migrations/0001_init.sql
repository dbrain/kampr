CREATE TABLE devices (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    role         TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER,
    expires_at   INTEGER,
    revoked_at   INTEGER,
    user_agent   TEXT,
    origin       TEXT
) STRICT;

CREATE TABLE tokens (
    hash       TEXT PRIMARY KEY,
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    revoked_at INTEGER
) STRICT;

CREATE INDEX tokens_by_device ON tokens(device_id);

CREATE TABLE pairings (
    hash       TEXT PRIMARY KEY,
    role       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    attempts   INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE credentials (
    id           TEXT PRIMARY KEY,
    device_id    TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    rp_id        TEXT NOT NULL,
    passkey      TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
) STRICT;

CREATE INDEX credentials_by_device ON credentials(device_id);

CREATE TABLE pane_prefs (
    device_id  TEXT NOT NULL,
    pane_id    TEXT NOT NULL,
    prefs      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, pane_id)
) STRICT;
