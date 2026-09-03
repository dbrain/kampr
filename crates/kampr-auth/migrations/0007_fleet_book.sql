-- Commands the operator ran across the herd, or kept to run again.
--
-- **Deliberately not keyed by `device_id`, which every other table here is.** A device row *is*
-- the identity in this schema — there is no account above it — so a phone and a desktop are two
-- of them, and a book keyed by device would be empty on whichever one the operator picked up
-- second. "Persisted on the server" was asked for so the list follows the operator between their
-- devices, and the node is the only grain that does that.
CREATE TABLE fleet_commands (
    id    TEXT PRIMARY KEY,
    -- The argv and cwd together, canonicalised. What makes "the same command" the same row, so
    -- re-running one moves it rather than adding a second copy, and a saved command that is run
    -- again stays saved instead of also appearing in the history.
    key   TEXT NOT NULL UNIQUE,
    kind  TEXT NOT NULL,
    args  TEXT NOT NULL,
    cwd   TEXT,
    label TEXT,
    at    INTEGER NOT NULL,
    -- What "newest first" is read from. `at` is a wall-clock second and a fan-out puts several
    -- commands inside one, so ordering on it left the list in whatever order SQLite felt like and
    -- the trim keeping an arbitrary five. This is bumped on every touch, so re-running an old
    -- command moves it to the top even when the clock has not moved.
    seq   INTEGER NOT NULL
) STRICT;

CREATE INDEX fleet_commands_by_kind ON fleet_commands(kind, seq DESC);
