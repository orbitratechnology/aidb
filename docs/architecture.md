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
| `app_settings`          | JSON preferences, one key per section (`appearance`, `safety`)          | `Settings`           |
| `connection_profiles`   | Connection settings, environment, read-only flag, credential-store ref  | `ConnectionProfiles` |
| `provider_profiles`     | AI provider settings and credential-store ref                           | with task 7.2        |
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

## AI providers

Provider-independent conversations live in `domain::ai`; `adapters::ai_providers` speaks each
API; `services::ai::chat` runs one request. No command uses them yet: provider profiles and
"Test provider" arrive in 7.2, the settings UI in 7.3 and the agent in 8.x.

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
| Secrets                 | OS credential store (service `com.orbitratech.aidb`, entries such as `connection:<id>`), or memory for the session when no store is available                                                                                          |
| Webview storage         | Webview local storage inside the app data directory; holds a copy of the saved theme (so the first frame paints in the right colors) and the SQL editor tabs with their drafts and saved-query links (cleared from Settings › Storage) |

Directories follow each OS's conventions through Tauri's path resolver; exact locations per
platform are documented with release hardening (task 12.3).

## Build and CI

- `pnpm build:web` builds the UI; `tauri build` embeds it in the binary.
- CI (GitHub Actions) runs web checks, Rust fmt/clippy/tests with a contract drift check, and a
  Linux `.deb` build. Windows and macOS packaging joins in task 12.2.
