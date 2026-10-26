-- aidb local metadata, schema version 1.
--
-- Secrets are never stored here. `secret_ref` columns hold only a reference to an entry in the
-- OS credential store. Query results and row samples are not stored either.
-- Timestamps are UTC ISO-8601 text.

CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY NOT NULL CHECK (key GLOB '[a-z]*' AND length(key) <= 64),
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE connection_profiles (
    id                   TEXT PRIMARY KEY NOT NULL,
    name                 TEXT NOT NULL COLLATE NOCASE UNIQUE
                         CHECK (length(trim(name)) BETWEEN 1 AND 100),
    engine               TEXT NOT NULL DEFAULT 'postgres' CHECK (engine IN ('postgres')),
    host                 TEXT NOT NULL CHECK (length(host) BETWEEN 1 AND 255),
    port                 INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    database_name        TEXT NOT NULL CHECK (length(database_name) BETWEEN 1 AND 63),
    username             TEXT NOT NULL CHECK (length(username) BETWEEN 1 AND 63),
    ssl_mode             TEXT NOT NULL DEFAULT 'verify-full'
                         CHECK (ssl_mode IN ('disable', 'require', 'verify-ca', 'verify-full')),
    ca_cert_path         TEXT CHECK (ca_cert_path IS NULL OR length(ca_cert_path) BETWEEN 1 AND 4096),
    connect_timeout_secs INTEGER NOT NULL DEFAULT 10 CHECK (connect_timeout_secs BETWEEN 1 AND 300),
    environment          TEXT NOT NULL DEFAULT 'development'
                         CHECK (environment IN ('local', 'development', 'staging', 'production', 'other')),
    -- New connections are read-only until the user explicitly enables writes.
    read_only            INTEGER NOT NULL DEFAULT 1 CHECK (read_only IN (0, 1)),
    secret_ref           TEXT CHECK (secret_ref IS NULL OR length(secret_ref) BETWEEN 1 AND 255),
    created_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at           TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE provider_profiles (
    id                    TEXT PRIMARY KEY NOT NULL,
    name                  TEXT NOT NULL COLLATE NOCASE UNIQUE
                          CHECK (length(trim(name)) BETWEEN 1 AND 100),
    kind                  TEXT NOT NULL
                          CHECK (kind IN ('openai', 'anthropic', 'google', 'ollama', 'openai_compatible')),
    endpoint              TEXT CHECK (endpoint IS NULL OR length(endpoint) BETWEEN 1 AND 2048),
    model                 TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 200),
    secret_ref            TEXT CHECK (secret_ref IS NULL OR length(secret_ref) BETWEEN 1 AND 255),
    request_timeout_secs  INTEGER NOT NULL DEFAULT 60 CHECK (request_timeout_secs BETWEEN 1 AND 600),
    context_budget_tokens INTEGER CHECK (context_budget_tokens IS NULL OR context_budget_tokens > 0),
    -- Plain HTTP to a non-loopback endpoint needs an explicit, recorded acknowledgement.
    allow_insecure_http   INTEGER NOT NULL DEFAULT 0 CHECK (allow_insecure_http IN (0, 1)),
    capabilities_json     TEXT CHECK (capabilities_json IS NULL OR json_valid(capabilities_json)),
    created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE saved_queries (
    id            TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT REFERENCES connection_profiles (id) ON DELETE SET NULL,
    name          TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    sql_text      TEXT NOT NULL CHECK (length(sql_text) <= 1000000),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX saved_queries_by_connection ON saved_queries (connection_id);

CREATE TABLE agent_tasks (
    id            TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT REFERENCES connection_profiles (id) ON DELETE SET NULL,
    title         TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 500),
    status        TEXT NOT NULL CHECK (status IN (
                      'draft', 'validated', 'preview_ready', 'awaiting_approval', 'approved',
                      'executing', 'verifying', 'succeeded', 'failed', 'partial', 'cancelled')),
    plan_json     TEXT CHECK (plan_json IS NULL OR json_valid(plan_json)),
    summary       TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX agent_tasks_by_created ON agent_tasks (created_at);

-- Redacted operation lifecycle records. Rows can be cleared by the user but never edited.
-- `connection_id` has no foreign key so history survives deleting a connection.
CREATE TABLE operation_audit (
    id             INTEGER PRIMARY KEY,
    occurred_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    connection_id  TEXT,
    environment    TEXT CHECK (environment IS NULL OR environment IN
                       ('local', 'development', 'staging', 'production', 'other')),
    actor          TEXT NOT NULL CHECK (actor IN ('user', 'agent', 'system')),
    operation      TEXT NOT NULL CHECK (length(operation) BETWEEN 1 AND 64),
    risk           TEXT CHECK (risk IS NULL OR risk IN ('R0', 'R1', 'R2', 'R3', 'R4')),
    -- Digest of the statement or operation; never raw values.
    fingerprint    TEXT CHECK (fingerprint IS NULL OR length(fingerprint) <= 128),
    approval_state TEXT CHECK (approval_state IS NULL OR length(approval_state) <= 32),
    status         TEXT NOT NULL CHECK (status IN
                       ('succeeded', 'failed', 'partial', 'cancelled', 'rejected')),
    rows_affected  INTEGER CHECK (rows_affected IS NULL OR rows_affected >= 0),
    detail         TEXT CHECK (detail IS NULL OR length(detail) <= 4000)
) STRICT;

CREATE INDEX operation_audit_by_time ON operation_audit (occurred_at);

CREATE TRIGGER operation_audit_append_only
BEFORE UPDATE ON operation_audit
BEGIN
    SELECT RAISE(ABORT, 'operation_audit is append-only');
END;

-- Freshness of cached schema metadata. Row data is never cached.
CREATE TABLE schema_cache_metadata (
    connection_id      TEXT PRIMARY KEY NOT NULL
                       REFERENCES connection_profiles (id) ON DELETE CASCADE,
    schema_fingerprint TEXT NOT NULL CHECK (length(schema_fingerprint) <= 128),
    object_count       INTEGER NOT NULL CHECK (object_count >= 0),
    refreshed_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Local record of applied migrations. Like the audit log, it keeps history after a connection is
-- deleted.
CREATE TABLE migration_records (
    id            TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT,
    environment   TEXT CHECK (environment IS NULL OR environment IN
                      ('local', 'development', 'staging', 'production', 'other')),
    file_path     TEXT CHECK (file_path IS NULL OR length(file_path) <= 4096),
    file_sha256   TEXT NOT NULL CHECK (length(file_sha256) = 64),
    started_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at   TEXT,
    status        TEXT NOT NULL CHECK (status IN
                      ('running', 'succeeded', 'failed', 'partial', 'cancelled')),
    verification  TEXT NOT NULL DEFAULT 'not_run'
                  CHECK (verification IN ('passed', 'failed', 'not_run')),
    detail        TEXT CHECK (detail IS NULL OR length(detail) <= 4000)
) STRICT;
