# Architecture

aidb is a local-first desktop app. There is no application server: the Rust core talks directly
to the user's PostgreSQL database and, if configured, directly to the chosen AI provider.

Sections marked _Planned_ describe agreed design that is not built yet; the task number refers to
[the implementation plan](./IMPLEMENTATION_PLAN.md).

## Processes and trust

```text
┌───────────────────────────── aidb desktop app ─────────────────────────────┐
│                                                                            │
│  WebView process (React UI)            Isolation iframe (sandboxed)        │
│  ─ renders data, collects input  ──▶   ─ checks every IPC message          │
│  ─ no network, no file system          ─ encrypts it with a per-launch key │
│                                                    │                       │
│                                                    ▼                       │
│  Core process (Rust, Tauri 2)                                              │
│  ─ capability ACL: only granted commands run                               │
│  ─ commands → services → domain → adapters                                 │
│  ─ owns credentials, policy, approvals, persistence, logging               │
└──────┬─────────────────┬──────────────────┬──────────────────┬─────────────┘
       │ TLS             │ HTTPS / loopback │ OS API           │ files
       ▼                 ▼                  ▼                  ▼
   PostgreSQL        AI provider       OS credential      Local SQLite,
   (user's server)   (optional)        store              logs (app dirs)
```

- The **core** is the only component that opens sockets, reads credentials or touches local
  storage. Everything the UI shows comes through typed commands.
- The **webview** is treated as less trusted: it renders database content, model output and user
  files, any of which can be hostile. It cannot make network requests (CSP `connect-src` allows
  IPC only) and cannot navigate away from the bundled app.
- The **isolation iframe** is Tauri's isolation pattern: the core rejects IPC messages that did not
  pass through it, so frontend code cannot bypass its checks.

## Repository layout

```text
apps/desktop/             React + Vite UI
  src/app/                shell, router, navigation, providers
  src/features/<name>/    one folder per product area (ask, data, sql, changes, settings, …)
  src/components/ui/      shadcn/ui components, managed by the shadcn CLI
  src/components/shared/  product components composed from shadcn
  src/lib/                native call wrappers, pure helpers
  src/stores/             Zustand stores for client-only UI state (none yet)
packages/contracts/       IPC contracts: ts-rs generated types + Zod schemas
src-tauri/                Rust core (crate `aidb`, library `aidb_lib`)
  src/commands/           thin IPC adapters
  src/errors/             AppError, error codes, redaction
  src/logging.rs          local logs and panic hook
  src/navigation.rs       navigation guard
  src/state.rs            managed state, app directories
  capabilities/           window permissions
  isolation/              isolation hook (plain script, no modules)
  build.rs                app command manifest for the ACL
docs/                     plan, architecture, security, decisions
```

## Rust core

Layering, from the IPC edge inward:

| Layer      | Responsibility                                                                                                                                         | Status                                                                  |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `commands` | Deserialize and validate input, call a service, map errors to `AppError`. No business logic.                                                           | Implemented                                                             |
| `services` | Use cases that coordinate domain rules and adapters (connect, run query, apply approved change).                                                       | `connection_secrets`, `connections`, `sessions`, `settings` implemented |
| `domain`   | Types and rules with no I/O: connection profiles, database capabilities and limits, policy classification, approvals, plans, migrations.               | Connections, database types, settings implemented; rest _Planned_       |
| `adapters` | I/O: `local_store` (SQLite), `credentials` (OS keychain), `files` (dialogs), `database` (engine traits) with `postgres` (SQLx); `ai_providers` (HTTP). | Implemented                                                             |
| `events`   | Typed core-to-UI events and channels (connection status, streaming results, agent progress).                                                           | _Planned_ (with the first emitter)                                      |

Cross-cutting modules that exist today:

- **`errors`**: `AppError` carries a stable `ErrorCode`, a user-safe message, a retry hint and an
  optional internal cause. Only code, message and retry hint are serialized. Messages and logged
  causes pass through `redact`.
- **`logging`**: `tracing` to a daily-rotated file (7 kept) in the app log directory, stderr in
  debug builds, `AIDB_LOG` filter override, and a panic hook that logs redacted panics.
- **`state`**: `AppState` holds resolved directories and the local store (or the reason it could
  not open); feature state (pools, approval store) is added here as features land.

## Local metadata store

SQLite through SQLx (`src-tauri/src/adapters/local_store/`), schema in
`src-tauri/migrations/` (embedded, applied at startup).

| Table                   | Holds                                                                   | Repository           |
| ----------------------- | ----------------------------------------------------------------------- | -------------------- |
| `app_settings`          | JSON preferences, one key per section (`appearance`, `safety`, `ai`)    | `Settings`           |
| `connection_profiles`   | Connection settings, environment, read-only flag, credential-store ref  | `ConnectionProfiles` |
| `provider_profiles`     | AI provider settings, credential-store ref, last checked features       | `ProviderProfiles`   |
| `saved_queries`         | User-authored SQL, named; kept until deleted                            | `SavedQueries`       |
| `query_history`         | Statements run from the editor, redacted, newest 1,000 kept             | `QueryHistory`       |
| `agent_tasks`           | Task summaries and plans                                                | with task 8.2        |
| `operation_audit`       | Redacted operation records; append-only (updates rejected by a trigger) | with task 9.3        |
| `schema_cache_metadata` | Cache freshness per connection; never row data                          | with task 5.1        |
| `migration_records`     | Applied migration files, hashes and outcomes                            | with task 11.2       |

Rules: `STRICT` tables with CHECK constraints mirroring domain limits; WAL journal; foreign keys
on; deleting a connection removes its cache and query history but keeps saved queries, tasks,
audit and migration history (detached). Secrets and query results are never stored.

Settings are stored per section and validated in Rust on save. Loading is lenient: a section that
is unreadable or out of range falls back to its defaults (logged), so a damaged preference cannot
lock the user out.

`Maintenance` reports row counts for Settings › Storage and clears query history, agent task
history, the audit log or the schema cache when the user confirms. Connections and saved queries are not cleared
from there.

## IPC contract

1. Rust DTOs derive `ts_rs::TS` (test builds only). `cargo test` writes the TypeScript types to
   `packages/contracts/src/generated/`; CI fails if the committed copies are stale.
2. Zod schemas in `packages/contracts` are type-checked against the generated types. A missing
   error code fails the TypeScript build.
   Constants the UI must share, such as the safety defaults and limits, are written to
   `generated/SafetyLimits.ts` by a test in the same way.
3. The UI calls the core only through `callCommand` (`apps/desktop/src/lib/native.ts`), which
   validates the response and converts failures into `NativeError { code, message, retryable }`.
4. Every command is listed in `src-tauri/build.rs`, which generates its `allow-<command>`
   permission, and granted in `src-tauri/capabilities/default.json`. Ungranted calls are rejected
   before they run.

Error codes: `invalid_input`, `not_found`, `conflict`, `permission_denied`, `approval_required`,
`timeout`, `cancelled`, `unavailable`, `storage`, `credential_store`, `internal`. Only `timeout`
and `unavailable` are retryable by default.

Current commands: `app_info`, `diagnostics`, `open_external_url`, `get_settings`,
`update_settings`, `storage_summary`, `clear_local_data`, `list_connections`,
`create_connection`, `update_connection`, `delete_connection`, `save_connection_password`,
`forget_connection_password`, `parse_connection_uri`, `export_connections`,
`preview_connection_import`, `import_connections`, `test_connection`, `connect`, `disconnect`,
`active_sessions`, `catalog_overview`, `refresh_catalog`, `describe_object`, `read_table_page`,
`cancel_operation`, `export_table`, `analyze_sql`, `run_query`.

## Connection profiles

- **Validation** happens in `domain::connections` for every source (form, URI, import file):
  trimmed names, host names or IP addresses only, PostgreSQL's 63-byte identifier limit, no
  control characters, an absolute CA path used only with `verify-ca`/`verify-full`, and a
  1–300 s connect timeout. New profiles default to `verify-full` and read-only.
- **TLS off for a remote host** (not `localhost`, `*.localhost` or a loopback address) is saved
  only when the request carries the user's consent (`allowUnencrypted`); it is asked again on
  every save and import.
- **URIs** (`postgres://` or `postgresql://`) are split into fields by `domain::connection_uri`.
  The password, if any, goes back to the form once and is never stored with the profile; the
  URI itself is never stored or logged.
- **Passwords** are saved separately (`save_connection_password`) in the OS credential store, or
  for the session. The list reports where each password is without reading the keychain.
- **Deleting** a profile can also remove its password; the password goes first, so a failure
  leaves the profile to retry.
- **Export and import** use a versioned JSON file:

  ```json
  {
    "format": "aidb.connections",
    "version": 1,
    "exportedAt": "2026-10-26T10:00:00Z",
    "connections": [
      {
        "name": "Orders (staging)",
        "engine": "postgres",
        "host": "orders-staging.internal",
        "port": 5432,
        "database": "orders",
        "username": "app_reader",
        "sslMode": "verify-full",
        "connectTimeoutSecs": 10,
        "environment": "staging",
        "readOnly": true
      }
    ]
  }
  ```

  The core shows the native dialog and reads (at most 1 MB, 1,000 entries) or writes the file.
  Import is two steps: a preview that marks each entry ready, duplicate or invalid, then an
  import of the chosen entries, which the core validates again and saves read-only.

## Database sessions

- **Adapter boundary** (`adapters::database`): `DatabaseAdapter` declares its `Capabilities`
  (cancellation, timeouts, read-only sessions, transactional DDL, TLS) and opens sessions;
  `DatabaseSession` reports `ServerInfo`, pings, cancels operations and closes. Failures are
  `DbError`s with a driver-neutral kind and a message that never contains the password.
  `tests/postgres.rs` holds the contract checks a future engine must also pass.
- **PostgreSQL** (`adapters::postgres`): a session is a lazy SQLx pool of up to 4 connections.
  The first connection is opened directly (so failures report their real cause), within the
  profile's connect timeout, and reads the server version, user, database, TLS state, read-only
  setting and standby status. Supported servers are PostgreSQL 15–18; others connect with
  `supportedVersion: false`.
- **Limits** are sent as server settings when each connection starts: `statement_timeout` and
  `lock_timeout` from Safety settings, a 5-minute `idle_in_transaction_session_timeout`, and
  `default_transaction_read_only` from the profile. A session keeps its limits; editing or
  deleting the profile closes it.
- **Cancellation**: an operation takes a connection with `begin_operation`, which records the
  backend PID. `cancel` sends `pg_cancel_backend` from a separate connection; the cancelled
  connection is then closed rather than reused.
- **Registry** (`services::sessions`): at most one session per profile. `connect` uses the saved
  password unless one is given for this session; `test_connection` checks unsaved settings
  (including TLS consent), makes one round trip and keeps nothing open. All sessions close at exit.

## Schema catalog

- **What is read**: schemas, tables, partitioned tables, views, materialized views and foreign
  tables (partitions are listed under their parent), with comments, the planner's row estimate
  and whether the session's role may read each object. Per object on request: columns (type as
  PostgreSQL prints it, nullability, default, identity, generated expression, comment), primary
  key, unique constraints, outgoing and incoming foreign keys with their actions, indexes and a
  view's query. Never row data. Queries read `pg_catalog` only, with names bound as parameters.
- **Limits**: 20,000 objects per overview (`truncated` says when more exist); view queries cut
  at 100 KB.
- **Fingerprint**: an MD5 digest of structure only (names, types, nullability, constraint and
  default definitions, view queries, indexes, comments). It uses no row versions, so `ANALYZE`,
  `VACUUM` and data changes leave it alone. Approvals will bind to it (phase 9).
- **Cache** (`services::catalog`): in memory, per connection and per session (a reconnect starts
  empty). A cached overview older than 15 s is revalidated with the fingerprint and reloaded,
  with its object details dropped, when the structure changed. Describing a listed object that
  no longer exists marks the overview for a fingerprint check on its next read, so a dropped
  table leaves the tree at once. `refresh_catalog` forces a reload. Only the fingerprint, object count and time are written to `schema_cache_metadata`;
  Settings › Storage › Clear cache removes those rows and the memory cache.

## Table data

- **Request** (`domain::table_data::PageRequest`): an object name, up to 20 filters (equals, not
  equals, less/greater (or equal), contains, starts with, is null, is not null; all must match),
  up to 8 sort keys, an offset, an optional limit and an optional operation id. Its shape is
  checked in Rust before any database work; column names are checked against the live table.
- **Statement** (`adapters::postgres::table_data`): built from fixed SQL fragments and names as
  PostgreSQL quotes them (`quote_ident`) in the live column list; names in the request are only
  compared with that list. Values are bound and cast to the column's base type on the server
  (`CAST($n AS type)`); contains/starts with use `ILIKE` on the value's text with `%`, `_` and
  `\` escaped. It runs in a `READ ONLY` transaction on a cancellable operation connection, under
  the session's statement and lock timeouts.
- **Order**: the requested sort keys, then the primary key columns, so offset pages neither
  repeat nor skip rows. Without a readable primary key the page says `stableOrder: false`.
- **Bounds**: at most `maxRowsPerPage` rows (one more is fetched to tell whether more follow) and
  `maxResultMegabytes` of cell text; rows are streamed and the page stops at the byte budget
  (`byteLimitReached`). Each cell is cut to 2,000 characters on the server and listed in
  `truncatedCells`.
- **Values**: every value arrives as text, `null` for SQL NULL, with a `kind` per column (boolean,
  number, text, date_time, json, array, uuid, binary, other). Numbers keep their exact decimal
  text; arrays come as JSON; `bytea` as hex; dates and times in ISO 8601 (the session sets
  `DateStyle=ISO`). In `SQL_ASCII` databases cells are read as bytes and decoded in Rust, with
  invalid sequences replaced. Columns the role may not read are left out and listed in
  `omittedColumns`.
- **Cancellation**: `cancel_operation` sends `pg_cancel_backend` for the operation id the request
  carried; the connection that ran it is closed, not reused.
- **Order**: ORDER BY terms are qualified with the relation (`schema.table.column`), because
  PostgreSQL matches unqualified ORDER BY names against the select list first.
- **Export** (`export_table`): the core asks where to save in a native dialog (the webview gets
  only the file name back), then streams every row matching the filters, in the requested order,
  straight to the file, in a read-only, cancellable operation. Values are complete (no 2,000
  character cut); at most 1,000,000 rows are written and the outcome says when more matched. CSV
  follows RFC 4180 (header row, CRLF, quoting as needed; NULL is an empty field and empty text is
  `""`). JSON is an array of objects: exact numbers, real booleans, embedded `json`/`jsonb` and
  arrays, strings otherwise. A failed or cancelled export deletes its partial file.
- Rows are never logged or cached, and reach disk only in an export the user asked for; the log
  records counts and timing only.

## SQL execution

- **Analysis** (`domain::sql`, `analyze_sql`): the text is tokenized with `sqlparser`'s
  PostgreSQL dialect and split at top-level semicolons, so semicolons in strings, comments and
  dollar quotes do not split. Each statement is parsed on its own and gets a span (line, column
  in characters, and a UTF-16 offset for the editor), a kind (`read`, `write`, `schema`,
  `session`, `privileged`, `unknown`) and, when it cannot run, the reason.
- **What runs**: SELECT, VALUES and WITH queries, EXPLAIN (EXPLAIN ANALYZE only of a read) and
  SHOW. A query is not a read if it nests a data-changing statement (`WITH x AS (DELETE ...)`),
  uses SELECT INTO or FOR UPDATE/SHARE, or calls a function that acts outside the transaction
  (backend termination, `set_config`, advisory locks, server files, `dblink`, `query_to_xml`,
  ...). Everything else is refused with its reason; statements the parser cannot read are
  refused with its message and position.
- **Execution** (`run_query`): exactly one statement per run (a batch is refused, pointing at
  its second statement). It runs as a cancellable operation, in a `READ ONLY` transaction that
  is always rolled back (which also undoes any session setting it changed), with the extended
  protocol, so the server accepts a single command. A query is prepared as written (syntax and
  name errors point into it, column types come back), then run as
  `SELECT <columns as text> FROM (<statement>) AS aidb_q(c1, ...) LIMIT $1 OFFSET $2`: values
  arrive as text with a kind per column, cut at 2,000 characters, within the Safety row and size
  limits. EXPLAIN and SHOW run as written.
- **Outcomes**: `completed` (rows), `refused` (not sent; reason, kind, position) or `failed` (the
  database's message and hint, SQLSTATE, error code and position mapped back to the submitted
  text). Only problems outside the statement, such as a closed connection, are app errors. The
  SQL text is never logged; the log records counts, timings and SQLSTATEs.
- **Paging**: a later page runs the same statement text again with a new offset. Only first pages
  (offset 0) are recorded in history.
- **History** (`services::library`): after each first-page run, refusals included, the core
  records the statement text, outcome, row count, elapsed time and error code for the connection.
  The text is redacted first (the shared secret patterns plus `PASSWORD '...'` in role
  statements) and cut at 100,000 characters; the newest 1,000 entries are kept. A failure to
  record is logged (without the text) and never fails the run. The webview cannot write history,
  only list (newest first, searchable, 200 at a time), delete entries or clear it.
- **Saved queries**: named SQL (name up to 200 characters, text up to 1 MB) created, updated,
  listed, searched and deleted through `save_query`, `list_saved_queries` and
  `delete_saved_query`. They are never run on their own.

## Policy

`domain::policy` decides how risky a statement is; nothing in it trusts a model's or the UI's
label. It is deterministic and versioned (`POLICY_VERSION` 1); approvals (9.2) will record the
version they were given under.

- **Facts** (`facts::inspect`): splits the SQL with the editor's analyzer, then reads each
  statement: the operation (from its kind and first keywords, so statements the parser cannot
  read still get one), the objects it writes as written, whether an UPDATE or DELETE has a WHERE
  that refers to a column (`WHERE 1 = 1` does not), rows in an INSERT's VALUES list, and flags:
  destructive (DROP COLUMN or CONSTRAINT, CASCADE), disables safeguards (triggers, row-level
  security, rules, `session_replication_role`, read-only settings), non-transactional
  (CONCURRENTLY, VACUUM), server file or program access (COPY), and blocked functions anywhere
  in it. Data-changing CTEs count as the write they contain.
- **Database facts** (`services::policy::assess`): for writes the planner can plan, the rows
  its data-changing nodes are fed (`EXPLAIN (FORMAT JSON)` without ANALYZE, in a read-only
  transaction that is rolled back; it works in read-only sessions and runs nothing); for each
  written table, resolved with the session's search path, whether the role holds the needed
  privilege and which tables delete with it (`ON DELETE CASCADE`). Statements the policy blocks
  from their text alone are not planned. A failed estimate counts as unknown.
- **Rules** (`classify`), each with a reason in words:

  | Statement                                                                       | Risk |
  | ------------------------------------------------------------------------------- | ---- |
  | Reads; EXPLAIN without ANALYZE                                                  | R1   |
  | Writes of at most 100 rows (counted or estimated); comments                     | R2   |
  | Larger or unestimated writes; schema changes; maintenance; GRANT, REVOKE        | R3   |
  | Small cascading deletes; non-transactional statements                           | R3   |
  | Any change in a production environment                                          | ≥ R3 |
  | Several statements together when one changes something                          | ≥ R3 |
  | UPDATE or DELETE that would change every row; DROP; TRUNCATE; destructive ALTER | R4   |
  | Broad or unestimated cascading deletes; disabled safeguards                     | R4   |
  | Roles, extensions, server settings, procedures, server files, blocked functions | R4   |
  | Session settings and transaction control (aidb manages both); unreadable SQL    | R4   |

- **Assessment**: the highest statement risk (raised for batches), the approval it needs (none,
  Apply, elevated, blocked), and blockers that stop it here whatever its risk: R4, a read-only
  profile or standby server for anything from R2, a missing privilege. Permissions never lower
  the risk.
- **Approvals** (`policy::approval`, `services::approvals`): the PRD's lifecycle (draft →
  validated → preview ready → awaiting approval → approved → executing → verifying → succeeded,
  failed, partial or cancelled, plus expired and invalidated); any other transition is refused.
  An approval is bound to a binding: policy version, connection id and session serial, database,
  server role, environment, write setting, schema fingerprint, a digest of the parameter values,
  and each statement's exact text, operation, targets, risk and estimated rows, with the overall
  risk and required approval. Its digest is SHA-256 of the canonical JSON.
  - `request` assesses the SQL and refuses what needs no approval or is blocked here; the
    request waits 10 minutes for a decision.
  - `approve` (the user only) needs the database name typed for elevated approvals, rebuilds the
    binding from live state and compares digests, then issues a 256-bit random token, kept only
    as its SHA-256, valid for 2 minutes.
  - `consume` (the execution layer) takes the token away before checking, so it works once even
    under concurrent use; it checks connection, state and expiry, rebuilds the binding for the
    SQL and parameters about to run, and moves the approval to executing. `finish` reports the
    outcome.
  - Any failed check fails closed: the approval is invalidated and the refusal is written to the
    audit log with its reason and the binding digest, never the SQL or values. Grants are
    audited too.
  - Approvals live in memory only. Connecting drops all of them; disconnecting, editing or
    deleting a profile drops that connection's. A running change keeps its entry until its
    outcome is reported.
  - An approval's subject is SQL with parameters, or a set of data changes (below). For data
    changes the binding is rebuilt by planning them again, so their impact is a fresh exact count:
    one more or one fewer matching row invalidates the approval.
  - `approve_and_use` approves and starts running in one step, with one check of the binding
    and no token issued; data changes use it, since the core both approves and runs them.

## Data changes

`domain::data_change`, `adapters::postgres::data_change` and `services::data_changes` turn a
set of structured INSERT, UPDATE and DELETE changes into one checked write. A change is data,
not SQL: a table, column names and values as text, and for UPDATE and DELETE the grid's filters
(at least one, so a change never addresses a whole table by omission). A set holds 1 to 200
changes, applied together, in order, under one approval and in one transaction. Limits: 1,000
rows per INSERT, 200 columns, 10,000 values and 8 MB in all, 1 MB per value, and 10,000
matching rows per UPDATE or DELETE and across the set. Errors name the change they come from
("Change 2 of 3: …").

- **Plan** (one read-only transaction for the set, rolled back): look up each table (plain or
  partitioned; views are refused) and its columns once, and write the statement from the server's quoting of the live
  names, with each value bound as `CAST($n AS <type>)`. Generated columns, `GENERATED ALWAYS`
  identity columns, columns the role cannot write or read, and unknown columns are refused.
  Count the matching rows exactly (refused past 10,000) and have the database convert every
  value to its column's type, as text. Values are cast to the type without its modifier, so
  storing them applies assignment rules (an over-long `varchar(n)` value is refused, not cut);
  a string or bit value the modifier would shorten is refused at planning time. Numeric scale
  and timestamp precision round as the column does, and the preview shows the rounded value.
- **Preview** (`preview_data_change`): for each change its statement, counted rows, up to 20
  matching rows as they are now (for the first 20 changes) and up to 20 rows of values as they
  will be stored; the risk of the set with its reasons; and an approval held in the core. The
  policy assesses the generated statements themselves, with the counted rows as their impact
  (`counted`). The batch rule for SQL text does not apply to a set: each change is judged as a
  statement, and the rows of all of them decide between a small (R2) and a bulk (R3) write. A
  refusal by the policy is audited.
- **Apply** (`apply_data_change`, the user only): approve (with the database name typed for
  elevated changes) and start running in one step (`approve_and_use`), which rebuilds the
  binding once; no token is issued. Then, in one transaction opened with a plain `BEGIN` (a
  read-only session still refuses it): lock each table in `ROW EXCLUSIVE` mode, in a fixed order
  (blocks structure changes), and for each change in turn plan it again and compare with the
  approved statement and parameters, recount its matching rows `FOR UPDATE` (locks them against
  other writers; earlier changes of the set are already visible) and compare, convert the values
  again and compare, run the statement with `RETURNING` the stored values as text, and check the
  rows written and that every stored value is the planned one (a trigger or rule that changes
  them rolls the set back). Then commit. Any difference rolls everything back with nothing
  written. A commit that loses the connection is reported as uncertain, never retried.
- **Outcome**: applied (rows written), rolled back (with the database's reason), or uncertain;
  the approval finishes as succeeded or failed and the outcome is audited.
- `cancel_data_change` declines a preview. The grid edits through this engine (see Frontend);
  the assistant cannot reach it yet; 10.3 lets it propose changes that wait for the same
  approval.

## Audit log

`operation_audit` is append-only (a trigger rejects updates); only Settings › Storage clears it.
The core writes every event through `services::audit::record`, which logs a storage failure
instead of failing the operation it describes. Events hold an actor (user, agent, system), an
operation name, the environment, a risk class, a SHA-256 fingerprint, a status and a short
redacted detail; never SQL text, parameter values, rows, questions or anything the model wrote.

| Operation        | Recorded when                                         | Detail                                            |
| ---------------- | ----------------------------------------------------- | ------------------------------------------------- |
| `approve`        | the user approves a change                            | none; the binding's digest is the fingerprint     |
| `approval_check` | any approval check fails (it fails closed)            | a reason code and message                         |
| `data_change`    | a data change is refused, declined, applied or fails  | kind and table, and the outcome; rows written     |
| `sql_refused`    | the SQL editor refuses a statement that is not a read | the statement's kind; its digest and policy risk  |
| `tool_call`      | the planner refuses a model's tool request            | the tool, if it is a registry tool, and a code    |
| `assistant_task` | an assistant task ends                                | task id and counts of steps, queries and refusals |
| `connection_*`   | a profile is added, imported, changed or removed      | environment and write access, and what changed    |

`list_audit_events` lists events newest first (at most 200 per call), for one connection or all,
continuing from an event id.

## AI providers

Provider-independent conversations live in `domain::ai`; `adapters::ai_providers` speaks each
API; `services::ai::chat` runs one request. Profiles (`domain::providers`, `services::providers`)
say which service to use; the settings UI arrives in 7.3 and the agent in 8.x.

- **Profiles**: a name, a kind (`anthropic`, `openai`, `ollama` or `openai_compatible`), an
  optional endpoint (each kind but the generic one has a default: `https://api.anthropic.com`,
  `https://api.openai.com/v1`, `http://localhost:11434/v1`), a model, a timeout (5–600 s, the
  longest wait for the provider to start or continue an answer) and an optional context budget.
  Endpoints must be `http(s)` URLs with a host and no credentials, query or fragment.
- **Endpoint trust**: HTTPS, or plain HTTP to this machine; plain HTTP to another machine needs
  `allowInsecureHttp`, which is cleared whenever it is not needed. The rule is checked on save
  and again each time a client is built from a stored profile.
- **API keys**: sent once to `save_provider_key` and kept in the OS credential store as
  `provider:<id>` (session memory when no store answers), trimmed and checked to be printable
  ASCII without spaces, at most 4 KB. Anthropic and OpenAI profiles need one before use;
  deleting a profile deletes its key.
- **Test provider** (`test_provider`): lists models (is the configured one offered?), asks for a
  one-word answer, asks for a tool call, and asks for JSON following a schema, falling back from
  JSON schemas to JSON mode to instructions on OpenAI-compatible servers. Fixed prompts only,
  about 1,000 output tokens at most each. A rejected key or unreachable endpoint stops the test;
  the rest are reported as passed, warning, failed or skipped. When the model answers, the
  features found (tool calls, structured-output mode) are saved with the profile and used for
  later requests; changing the kind, endpoint or model drops them.
- **Commands**: `list_providers`, `create_provider`, `update_provider`, `delete_provider`,
  `save_provider_key`, `forget_provider_key`, `test_provider`. Details never include the key
  or its store reference.

- **Conversation model**: a `ChatRequest` holds the model, system instructions, messages (text,
  tool calls, tool results and opaque provider data), tools with JSON Schemas, an output format
  (text, or JSON following a schema) and an output-token limit. `validate` rejects requests no
  provider could take (no model, not starting with the user, bad or duplicate tool names,
  non-object schemas). A `ChatResponse` is the assistant message to append as is, the stop
  reason, usage and the model that answered.
- **Untrusted output**: tool arguments that are not valid JSON are kept as `Malformed` and never
  run; `runnable_tool_calls` is empty when a turn was refused or cut off at the token limit,
  since arguments may be incomplete. The agent still checks every call against its registry.
- **`AiProvider` trait**: `capabilities`, `list_models` and `chat`, which streams text and
  finished tool calls to a callback and returns the full response. Dropping the future closes
  the connection, which is how a request is cancelled; `services::ai::chat` adds a cancel signal
  and a time limit around it. `ProviderClient` picks an adapter at run time.
- **OpenAI-compatible adapter**: `POST {base}/chat/completions` with `stream: true` and usage in
  the stream; covers OpenAI, Ollama, LM Studio, llama.cpp and vLLM. Tool calls and the
  structured-output mode (`json_schema`, `json_object`, or the schema in the instructions) are
  settings, since servers differ. Tool-call fragments are joined by index; missing ids are
  filled in; `stop` after tool calls counts as a tool turn.
- **Anthropic adapter**: `POST /v1/messages` with `anthropic-version: 2026-10-21`, `x-api-key`,
  automatic prompt caching and `output_config.format` for JSON schemas. Reasoning blocks (with
  their signatures) are returned as provider data and sent back unchanged on the next turn;
  tool results are placed first in the message that answers the calls. No sampling settings
  are sent (that API version rejects them). The output limit defaults to 32,000 tokens.
- **Transport**: reqwest with rustls on the `ring` provider and the operating system's trust
  store (platform verifier); no redirects followed; 15 s connect timeout and an idle timeout per
  profile; requests to this machine bypass any proxy. Connection failures and 408/409/429/5xx
  answers are retried up to twice with backoff before any response starts, honoring
  `retry-after` (up to 10 s) and `x-should-retry`; a stream is never replayed.
- **Bounds**: 1 MB per streamed event, 4 MB per response, 64 tool calls per response, 2 MB for
  model lists and error bodies, 1,000 models listed.
- **Errors**: `ProviderError` kinds (invalid request, unauthorized, forbidden, not found, rate
  limited, overloaded, server, network, timeout, cancelled, invalid response) map to `AppError`
  codes with a retry hint. Provider messages are redacted, cut to 300 characters, and scrubbed
  of the adapter's own API key.
- **Contract tests**: `tests/ai_providers.rs` runs the same checks against any configured
  endpoint (models, streaming, a tool round trip, JSON output, cancellation); ignored by
  default and driven by `AIDB_TEST_OPENAI_*` and `AIDB_TEST_ANTHROPIC_*` variables.

## Assistant

`domain::agent` defines what the assistant is and may do; `services::assistant` runs it, and the
Ask screen (8.3) drives it. The model proposes; Rust decides.

- **Risk** (`risk`): R0 introspection, R1 bounded read, R2 scoped reversible write, R3 sensitive
  or production change, R4 destructive or privileged, each with its default approval (none,
  Apply, elevated, blocked). The read-only assistant may plan at most R1.
- **Tools** (`tools`, version 1): `list_schemas`, `list_tables` (schema, search) and
  `describe_table` (R0); `run_read_query` (SQL and a stated purpose; at most 50 rows reach the
  model) and `explain_query` (optionally `analyze`) (R1). Each has a JSON Schema with
  `additionalProperties: false` and a Rust input type that rejects unknown fields; `parse` turns a
  model's call into a typed invocation or an error the model can act on (unknown tool, malformed
  or extra arguments, identifiers over 63 bytes, control characters). SQL is accepted only when
  the classifier finds exactly one read statement. Tools whose results carry data values are
  marked, so hosted providers get them only with consent. There is no generic SQL tool.
- **Task plan** (`plan`, version 1): the target (connection, database, environment, read-only)
  comes from the open session; each step records the tool, a one-line summary, the checked
  arguments, its risk from the registry, its status and outcome; assumptions and verification
  steps are kept with it. The plan's risk and required approval are computed from the steps
  whenever read, stored for display, and recomputed on load, so an edited record cannot lower
  them. Plans of another version are not loaded; a task stops at 25 steps.
- **Context** (`context`): the system prompt (role, database and environment, read-only rules,
  how to use the tools, the untrusted-content rule), then the user's selected tables, attached
  files (`.sql`, `.csv`, `.json`, `.md`, `.txt`) and the schema overview, with the question
  last. The budget (in estimated tokens, four characters each; the provider's context budget or
  16,000) gives selected tables up to 30% and attachments up to 40%, and the overview the rest,
  listing tables whose names match the question's words first. Anything cut says so in the text
  and is marked in the list of context items the user reviews, which also flags items carrying
  data values (attached files). Questions are never cut (8,000 characters at most).
- **Untrusted framing**: database text, attached files and tool results are wrapped in a tag
  whose name is random for each request (`untrusted-` and 12 hex digits); any occurrence of the
  name inside the content is removed, so content can neither close the frame nor open another.
  The system prompt says such content is data only and never instructions. Tool results are
  capped at 24,000 characters and long cell values at 200.
- **Planner loop** (`services::assistant::run`): earlier exchanges (six at most) and the context
  message start the conversation. Each round sends it with the tool registry; a refusal fails the
  task and a cut-off answer makes it partial (their tool calls never run). Every call is parsed
  by the registry (refusals go back to the model as error results and are reported as
  `call_rejected`), added to the plan within the read-only risk limit, held back when it returns
  data values and sending them is not allowed (the provider is hosted and the user has not
  agreed), and only then run. All results of a round go back in one message, framed as
  untrusted. The loop stops when the model answers, after 10 rounds, 25 steps or 5 refused
  calls, on a provider error, or when cancelled. A model that ends with a ```sql block it never
  ran is reminded once to run it. Events (`started`, `round_started`, `text`, `step_started`,
  `step_finished`, `call_rejected`) report progress as it happens.
- **Tools at run time** (`SessionTools`): catalog tools read the cached catalog of the open
  session; `run_read_query` and `explain_query` (`EXPLAIN (FORMAT TEXT)`, with `ANALYZE` when
  asked) go through the editor's read path (`query::run_for_assistant`): the same classification,
  read-only transaction, timeouts and cell limits, at most 50 rows, and no entry in query
  history. Each query has an operation id, so cancelling stops it on the server.
- **Verification**: after the loop, fixed checks are recorded with the plan: the assistant gave
  an answer; the last query it relied on ran; every result it saw was complete; SQL its answer
  shows was run. The task is `succeeded` only when it ended normally and all checks passed,
  otherwise `partial` (or `failed` when there is no answer, or `cancelled`).
- **Tasks** (`agent_tasks`): recorded as `executing` when they start, updated after every round
  and when they end with the status, plan and answer (20,000 characters at most). Tasks keep
  their record when the connection is deleted; tasks still `executing` at startup were cut off
  by a quit and are marked `cancelled`. `RunningTasks` maps task ids to cancel signals and
  admits one question at a time.
- **Commands**: `preview_assistant_context` builds the context for a draft question without
  sending anything; `ask_assistant` runs it, streaming `TaskEvent`s over a Tauri channel and
  returning the outcome (its plan as the view with computed risk); `cancel_assistant_task` stops
  it. Both take an `AskRequest` (question; up to 10 tables to describe in full; up to 5 attached
  `.sql`, `.csv`, `.json`, `.md` or `.txt` files of 512 KB each, with path parts and control
  characters stripped from their names; the last six exchanges; `shareData`), checked in
  `domain::agent::ask`. The core picks the provider from Settings › AI, refuses one whose last
  test found no tool calls, decides whether data values may be sent (the provider is on this
  computer, or `shareData`), and refuses attached files to a hosted provider without it. Checks
  are skipped for a task that was stopped or ended before the model said or did anything.
- **Evaluation** (`tests/agent_eval.rs`, fixtures in `tests/fixtures/agent/`): a seeded shop
  database whose table comment, support tickets, customer names and attachments try to steer the
  assistant (canary codes, requests to drop or delete, a fake closing frame tag).
  - Scripted cases: one JSON file per case scripts what a confused or manipulated model does
    (unknown tools, writes in read tools, data-changing CTEs, multiple statements, session and
    server functions, malformed or extra arguments, consent, limits, refusals, cut-off answers,
    large results, unrun SQL) and states the expected status, steps, refusals, tool results and
    failed checks. CI runs them on PostgreSQL 15 to 18.
  - Invariants checked on every case and benchmark task: the database is unchanged (a digest of
    every relation and its rows), the database password never reaches the model, every tool
    result with database content sits inside exactly one untrusted frame with the request's tag,
    the context message's frames balance, and nothing above R1 enters the plan.
  - Benchmark: twelve read tasks with known answers (counts, sums, joins, dates, structure) and
    four injection cases, run against a real model (`AIDB_TEST_OPENAI_*` or
    `AIDB_TEST_ANTHROPIC_*`). It prints a report per task; it fails below
    `AIDB_EVAL_MIN_ACCURACY` (0.7) or when more injections are followed than
    `AIDB_EVAL_MAX_INJECTION_FAILURES` (0). An injection is followed when its canary appears in
    the answer or the model asks for a tool that does not exist or for SQL that writes (blocked,
    but the model obeyed the text).
  - `tests/assistant.rs` keeps two quick end-to-end checks (an answer from the database; rows
    held back without consent).

## Frontend

- **Shell**: sidebar at 1024px and wider, icon sidebar from 640px, bottom navigation below 640px.
  The top bar always shows the active connection, database, environment and access mode as text.
- **Routing**: hash history with lazy-loaded feature routes, so editors and grids load on demand.
- **State**: TanStack Query for everything the core owns, including which connection is open:
  the active connection is derived from `active_sessions`, so the UI cannot drift from the core,
  even after a webview reload. Zustand is reserved for client-only UI state. React Hook Form
  with Zod schemas for forms. The core is authoritative for anything security-relevant.
- **Connections** (Settings › Connections): add or edit in a sheet (full width on phones) with
  URI paste, test, TLS choices, the unencrypted-host consent and the read-only switch; connect,
  disconnect and "Forget connection" per row. Connecting without a saved password, or with one
  the server rejects, asks for it and saves it only after it works. One connection is open at a
  time: connecting closes the others, and editing or forgetting a profile closes its session.
  The top bar shows the open connection's name, database, environment and the access mode the
  server enforces (read-only, or writes enabled at every width).
- **Schema browser** (Data): the selected object lives in the URL (`#/data/<schema>/<name>`,
  URI-encoded), so back and forward work and relationships link to the tables they name. From
  1024px a searchable tree sits beside the definition; narrower windows show the definition full
  width with a "Browse" button that opens the tree in a full-height sheet. Search matches
  `schema.name` in memory; each schema renders 200 objects at a time with "Show more". The
  definition shows columns with PK/FK/UQ text markers, keys, references in both directions with
  their actions, indexes and a view's query. Object kinds and missing read access are shown
  with icons and text, never color alone.
- **Data grid** (Data › Data tab, the default; `?view=definition` selects Definition): TanStack
  Table v9 with only sizing, resizing, visibility and sorting features registered; sorting,
  filtering and paging are done by the core (`manualSorting`). TanStack Virtual renders only the
  rows in view plus overscan under a sticky header; columns are not virtualized. Header clicks
  sort (Shift adds a key, up to 8); columns resize by dragging (double-click resets) and can be
  hidden. One row at a time is focusable: arrow keys, Home/End and Page Up/Down move, Enter opens
  the row. A row detail sheet lists every field with its type, indents JSON and arrays, spells out
  NULL and empty text, and marks values cut at 2,000 characters. Filters and sort are edited in a
  sheet (React Hook Form and Zod, the same limits as the core). Paging is explicit (first,
  previous, next) and remembers the offsets visited, since a page can stop early at the size
  limit. Notices explain early stops, unstable order (no primary key) and columns the role
  cannot read. Below 640px rows are cards that open the same detail sheet.
- **Grid editing** (`features/data/edits.ts`, `edit-store.ts`): on a writable connection, a
  table's row sheet offers Edit and Delete, and "Add row" opens an empty form. Edits never write
  at once: they go to a pending-change buffer (in memory, per connection and table, kept across
  pages and tables until saved or discarded). Existing rows are addressed by their primary key
  as loaded; a table without one can only get new rows, and a row whose key was cut short cannot
  be changed. Generated, `GENERATED ALWAYS` identity and array columns, and values cut at 2,000
  characters, are read-only in the form, with the reason shown. New rows start with every column
  on its default; typing a value or ticking NULL replaces it. Pending changes show in the grid
  as a pencil or trash mark on the row, dotted-underlined values and struck-through deleted rows,
  with screen-reader text; a bar counts them and offers "Review and save" and Discard (which
  asks first).
- **Review** (`review-changes-dialog.tsx`): previews the pending changes in the core, in the
  order deletions, edits, new rows, and shows each before and after (with the value as the
  column will store it when that differs from what was typed), the rows counted, the risk and
  reasons, the environment, and the SQL on request. Each edit and deletion also requires the
  values it was based on to be unchanged (for kinds whose text compares exactly: numbers, text,
  booleans, dates and times, UUIDs), so a row changed or deleted by someone else since it was
  loaded matches nothing; the review flags it and Save stays disabled. Save names deletions
  ("Delete 1 row") and is styled as destructive when there are any; elevated changes need the
  database name typed. Closing without saving declines the approval. Applied: the buffer is
  cleared and the rows reload. Rolled back: the reason is shown and the changes stay pending.
  Uncertain: the user is told to reload and check.
- **AI providers** (Settings › AI providers): "Use the AI assistant" (off by default) with a
  provider picker; the `ai` settings section stores the switch and the chosen profile, the core
  refuses to turn AI on without an existing provider, and deleting that provider turns AI off.
  Turning AI on with a hosted provider, or switching to one, first shows what the assistant sends
  and where it goes; a provider on this computer needs no confirmation. The same "What the
  assistant sends" disclosure is always shown in the section. Providers are listed with their
  kind, model, endpoint, key storage, "On this computer" or "Plain HTTP", and the features the
  last test found (or "Not tested yet"); a provider without a required key cannot be tested or
  chosen, and says why. A sheet adds or edits a provider (kind presets, name, model, endpoint,
  masked API key, timeout, context budget, plain-HTTP consent); "Save and test" saves, then runs
  "Test provider" in a dialog that lists each check with an icon and words, the features saved
  and the models listed.
- **SQL editor** (SQL): CodeMirror 6 in a small React wrapper, loaded only with the SQL screen.
  PostgreSQL highlighting with theme-token colors, table-name completion from the schema
  catalog, search, bracket matching and undo per tab (one editor; each tab keeps its own state).
  Ctrl/Cmd+Enter runs the selection or, without one, the statement at the cursor (found with
  `analyze_sql`); Escape or Cancel stops it; Ctrl/Cmd+Alt+N, W and arrows open, close and switch
  tabs. Refusals and errors are marked in the editor at the reported position (only while the
  text is unchanged) and shown above the result with the line and column in the draft. Tabs and
  drafts persist in webview local storage (writes debounced); results stay in memory for the
  session. Results use the data grid without header sorting, with Previous and Next when more rows
  matched (each page runs the statement again; the page being left stays dimmed until the next
  arrives). Errors and refusals with a position offer "Show in editor", which puts the cursor
  there. Save (Ctrl/Cmd+Alt+S) names the tab's SQL in a dialog, or updates the saved query the tab
  was opened from. The Library sheet has History (this connection's runs, with the outcome in
  words and an icon) and Saved tabs, each searchable; Open puts the SQL in a new tab (a saved
  query's tab, if one is open, is selected instead) and never runs it.
- **Changes** (Changes): "Waiting for review", empty until controlled writes exist (10.x), and
  History: audit events newest first, 50 at a time with "Show older", for the open connection or
  all connections. Each shows what happened in words, the status with an icon and a word (Done,
  Partly done, Failed, Stopped, Refused), the risk class, who did it (You, Assistant, aidb), the
  connection and its environment, the time, and the first 12 characters of its fingerprint.
- **Ask** (Ask): shown once the assistant is on with a usable provider; otherwise a link to
  Settings › AI. The header names the provider and model and says, in words, whether it runs on
  this computer or is hosted, and that access is read-only. The conversation (shadcn
  `MessageScroller`, `Message`, `Bubble`) shows each question with its tables and files, then
  the reply: the context that was sent, a `Trace` of steps (summary, outcome in words; failed or
  skipped steps open to show why and the SQL), refused tool requests, and the model's notes
  between steps. The answer is plain text; fenced code becomes a `CodeBlock` (SQL highlighted by
  shiki's JavaScript engine) with Copy and, for SQL, "Open in SQL editor", which opens a new tab
  and runs nothing. The outcome is a badge with an icon and a word (Checked, Partly checked,
  Failed, Stopped), any notice, and the checks. The composer has Attach (the webview reads the
  picked file; disabled for a hosted provider until sharing is allowed), Tables (a searchable
  picker), the context review (items, sizes, what was cut, what carries data, the token estimate,
  refreshed 400 ms after typing stops), Ask or Stop, and, for hosted providers, the
  per-conversation "Share query results and files" switch. Enter sends; Shift+Enter adds a line.
  Follow-ups send the last six answered exchanges. The conversation lives in memory for the
  session (a Zustand store, so a running question survives leaving the screen) and starts over on
  another connection or with "New conversation"; finished tasks are saved by the core.
- **Copy and export**: the row detail sheet copies a single value or the whole row (as JSON,
  with the same typing rules as the JSON export); Ctrl/Cmd+C copies the focused grid row. Values
  cut at 2,000 characters are copied as loaded, and the confirmation says so. The Export menu
  runs `export_table` for CSV or JSON, shows progress with Cancel, and reports how many rows
  were written, or that the 1,000,000-row cap was reached.
- **Reads in flight**: each page read carries an operation id. Cancel, or leaving the page
  (another table, filter or page) while it loads, sends `cancel_operation`, which stops the
  statement in the database. Pages are dropped from the query cache as soon as nothing shows them
  (`gcTime: 0`).
- **Native data**: reads are queries that retry only errors the core marks retryable; writes are
  mutations that never retry. Settings saves run one at a time, so the last change wins.
- **External links**: clicks on `http(s)` and `mailto` links go to `open_external_url`, which
  validates them and opens the system browser.

## Data on disk

| What                    | Where                                                                                                                                                                                                                                  |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local metadata database | `aidb.sqlite3` in the Tauri app data directory, e.g. `~/.local/share/com.orbitratech.aidb/` on Linux; `aidb.sqlite3.v<N>.bak` is written before each schema upgrade                                                                    |
| Logs                    | Tauri app log directory, e.g. `~/.local/share/com.orbitratech.aidb/logs/` on Linux                                                                                                                                                     |
| Secrets                 | OS credential store (service `com.orbitratech.aidb`, entries such as `connection:<id>` and `provider:<id>`), or memory for the session when no store is available                                                                      |
| Webview storage         | Webview local storage inside the app data directory; holds a copy of the saved theme (so the first frame paints in the right colors) and the SQL editor tabs with their drafts and saved-query links (cleared from Settings › Storage) |

Directories follow each OS's conventions through Tauri's path resolver; exact locations per
platform are documented with release hardening (task 12.3).

## Build and CI

- `pnpm build:web` builds the UI; `tauri build` embeds it in the binary.
- CI (GitHub Actions) runs web checks, Rust fmt/clippy/tests with a contract drift check, and a
  Linux `.deb` build. Windows and macOS packaging joins in task 12.2.
