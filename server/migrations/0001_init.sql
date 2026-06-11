-- Files currently in the active vault. Path is the canonical relative path
-- (forward-slash, never starting with /). etag is blake3 of the on-disk bytes.
CREATE TABLE files (
    path        TEXT PRIMARY KEY NOT NULL,
    etag        TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL,
    modified_at TEXT NOT NULL,                  -- ISO-8601 UTC
    modified_by TEXT,                           -- device_id of last writer (NULL until W2 auth)
    is_binary   INTEGER NOT NULL DEFAULT 0      -- 0/1; informational, not enforced
) STRICT;

CREATE INDEX idx_files_modified_at ON files(modified_at);

-- Paired devices. W2 fleshes out pairing; for now this just records who has touched what.
CREATE TABLE devices (
    id          TEXT PRIMARY KEY NOT NULL,      -- UUIDv4
    name        TEXT NOT NULL,
    platform    TEXT,                           -- "macos" | "windows" | "android" | other
    created_at  TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at  TEXT
) STRICT;

-- Append-only audit log. Every state change goes here.
CREATE TABLE audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          TEXT NOT NULL,                  -- ISO-8601 UTC
    op          TEXT NOT NULL,                  -- "create" | "modify" | "delete" | "restore"
                                                -- | "conflict" | "conflict_resolved"
                                                -- | "trash_purged"
    path        TEXT NOT NULL,
    device_id   TEXT,
    etag_before TEXT,
    etag_after  TEXT,
    size_bytes  INTEGER,
    extra       TEXT                            -- JSON blob for op-specific fields
) STRICT;

CREATE INDEX idx_audit_ts ON audit(ts);
CREATE INDEX idx_audit_path ON audit(path);
CREATE INDEX idx_audit_op ON audit(op);

-- Soft-deleted files, awaiting either restore or TTL purge.
-- stored_path is relative to the trash root, typically <ts>/<original-path>.
CREATE TABLE trash (
    id          TEXT PRIMARY KEY NOT NULL,      -- UUIDv4
    original_path TEXT NOT NULL,
    stored_path TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL,
    etag        TEXT NOT NULL,
    deleted_at  TEXT NOT NULL,
    deleted_by  TEXT,
    expires_at  TEXT NOT NULL,
    restored_at TEXT
) STRICT;

CREATE INDEX idx_trash_expires_at ON trash(expires_at);
CREATE INDEX idx_trash_original_path ON trash(original_path);

-- Pending conflicts: a write arrived with stale etag, the loser is preserved.
-- stored_path is relative to the conflicts root.
CREATE TABLE conflicts (
    id          TEXT PRIMARY KEY NOT NULL,      -- UUIDv4
    path        TEXT NOT NULL,                  -- vault path the conflict is about
    active_etag TEXT NOT NULL,                  -- etag of the version that won
    losing_etag TEXT NOT NULL,                  -- etag of the version saved aside
    stored_path TEXT NOT NULL,                  -- where the losing version lives
    losing_device TEXT,
    detected_at TEXT NOT NULL,
    resolved_at TEXT,
    resolution  TEXT                            -- "keep_active" | "use_other" | "keep_both"
) STRICT;

CREATE INDEX idx_conflicts_path ON conflicts(path);
CREATE INDEX idx_conflicts_unresolved ON conflicts(resolved_at) WHERE resolved_at IS NULL;
