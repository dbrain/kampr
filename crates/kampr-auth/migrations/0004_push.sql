CREATE TABLE push_subscriptions (
    id         TEXT PRIMARY KEY,
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    endpoint   TEXT NOT NULL UNIQUE,
    p256dh     TEXT NOT NULL,
    auth       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_sent_at INTEGER
) STRICT;

CREATE INDEX push_subscriptions_by_device ON push_subscriptions(device_id);

CREATE TABLE push_rules (
    device_id    TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    pane_id      TEXT NOT NULL,
    muted        INTEGER NOT NULL DEFAULT 0,
    snooze_until INTEGER,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (device_id, pane_id)
) STRICT;
