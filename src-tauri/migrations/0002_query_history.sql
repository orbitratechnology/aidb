-- SQL run from the editor, newest last. The text is redacted and capped by the core before it is
-- written. Forgetting a connection removes its history.
CREATE TABLE query_history (
    id            INTEGER PRIMARY KEY,
    executed_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    connection_id TEXT REFERENCES connection_profiles (id) ON DELETE CASCADE,
    sql_text      TEXT NOT NULL CHECK (length(sql_text) BETWEEN 1 AND 100002),
    status        TEXT NOT NULL CHECK (status IN ('completed', 'refused', 'failed', 'cancelled')),
    row_count     INTEGER CHECK (row_count IS NULL OR row_count >= 0),
    elapsed_ms    INTEGER CHECK (elapsed_ms IS NULL OR elapsed_ms >= 0),
    error_code    TEXT CHECK (error_code IS NULL OR length(error_code) <= 32)
) STRICT;

CREATE INDEX query_history_by_connection ON query_history (connection_id, id);
