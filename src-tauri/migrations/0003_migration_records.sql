-- Applied schema migrations: what was compared, the script's digest, how far it got and whether
-- the schema matched afterwards. No SQL text. Kept, detached, when a connection is deleted.
-- The table from 0001 was never written to; it is replaced with the columns the workflow uses.
DROP TABLE migration_records;

CREATE TABLE migration_records (
    id            TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT,
    environment   TEXT CHECK (environment IS NULL OR environment IN
                      ('local', 'development', 'staging', 'production', 'other')),
    target_schema TEXT NOT NULL CHECK (length(target_schema) BETWEEN 1 AND 63),
    source        TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 400),
    script_sha256 TEXT NOT NULL CHECK (length(script_sha256) = 64),
    steps         INTEGER NOT NULL CHECK (steps >= 1),
    applied_steps INTEGER NOT NULL DEFAULT 0 CHECK (applied_steps BETWEEN 0 AND steps),
    risk          TEXT NOT NULL CHECK (risk IN ('R0', 'R1', 'R2', 'R3', 'R4')),
    started_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at   TEXT,
    status        TEXT NOT NULL CHECK (status IN
                      ('running', 'succeeded', 'failed', 'partial', 'uncertain')),
    verification  TEXT NOT NULL DEFAULT 'not_run'
                  CHECK (verification IN ('passed', 'failed', 'not_run')),
    detail        TEXT CHECK (detail IS NULL OR length(detail) <= 4000)
) STRICT;

CREATE INDEX migration_records_by_connection ON migration_records (connection_id, started_at);
