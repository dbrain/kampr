CREATE TABLE mesh_nodes (
    pubkey       TEXT PRIMARY KEY,
    node_id      TEXT NOT NULL,
    name         TEXT NOT NULL,
    role         TEXT NOT NULL,
    url          TEXT,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER,
    revoked_at   INTEGER
) STRICT;

CREATE INDEX mesh_nodes_by_role ON mesh_nodes(role);

CREATE TABLE mesh_invites (
    hash       TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at    INTEGER,
    attempts   INTEGER NOT NULL DEFAULT 0
) STRICT;
