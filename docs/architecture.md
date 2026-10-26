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
  src/stores/             Zustand stores for cross-screen UI state
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

| Layer      | Responsibility                                                                                                       | Status                                                                |
| ---------- | -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `commands` | Deserialize and validate input, call a service, map errors to `AppError`. No business logic.                         | Implemented                                                           |
| `services` | Use cases that coordinate domain rules and adapters (connect, run query, apply approved change).                     | `connection_secrets`, `settings` implemented; more from 4.1           |
| `domain`   | Types and rules with no I/O: connection profiles, policy classification, approvals, plans, migrations.               | Connection profiles, settings implemented; rest _Planned_             |
| `adapters` | I/O: `local_store` (SQLite) and `credentials` (OS keychain) today; `postgres` (SQLx) and `ai_providers` (HTTP) next. | `local_store`, `credentials` implemented; others _Planned_ (4.2, 7.1) |
| `events`   | Typed core-to-UI events and channels (connection status, streaming results, agent progress).                         | _Planned_ (with the first emitter)                                    |

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
| `saved_queries`         | User-authored SQL                                                       | with task 6.3        |
| `agent_tasks`           | Task summaries and plans                                                | with task 8.2        |
| `operation_audit`       | Redacted operation records; append-only (updates rejected by a trigger) | with task 9.3        |
| `schema_cache_metadata` | Cache freshness per connection; never row data                          | with task 5.1        |
| `migration_records`     | Applied migration files, hashes and outcomes                            | with task 11.2       |

Rules: `STRICT` tables with CHECK constraints mirroring domain limits; WAL journal; foreign keys
on; deleting a connection removes its cache but keeps saved queries, tasks, audit and migration
history (detached). Secrets and query results are never stored.

Settings are stored per section and validated in Rust on save. Loading is lenient: a section that
is unreadable or out of range falls back to its defaults (logged), so a damaged preference cannot
lock the user out.

`Maintenance` reports row counts for Settings › Storage and clears agent task history, the audit
log or the schema cache when the user confirms. Connections and saved queries are not cleared
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
`update_settings`, `storage_summary`, `clear_local_data`.

## Frontend

- **Shell**: sidebar at 1024px and wider, icon sidebar from 640px, bottom navigation below 640px.
  The top bar always shows the active connection, database, environment and access mode as text.
- **Routing**: hash history with lazy-loaded feature routes, so editors and grids load on demand.
- **State**: Zustand for cross-screen UI state (for example the active connection); TanStack
  Query for data read from the core; React Hook Form with the contracts' Zod schemas for forms.
  The core is authoritative for anything security-relevant.
- **Native data**: reads are queries that retry only errors the core marks retryable; writes are
  mutations that never retry. Settings saves run one at a time, so the last change wins.
- **External links**: clicks on `http(s)` and `mailto` links go to `open_external_url`, which
  validates them and opens the system browser.

## Data on disk

| What                          | Where                                                                                                                                                               |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local metadata database       | `aidb.sqlite3` in the Tauri app data directory, e.g. `~/.local/share/com.orbitratech.aidb/` on Linux; `aidb.sqlite3.v<N>.bak` is written before each schema upgrade |
| Logs                          | Tauri app log directory, e.g. `~/.local/share/com.orbitratech.aidb/logs/` on Linux                                                                                  |
| Secrets                       | OS credential store (service `com.orbitratech.aidb`, entries such as `connection:<id>`), or memory for the session when no store is available                       |
| Webview storage (theme cache) | Webview local storage inside the app data directory; holds a copy of the saved theme so the first frame paints in the right colors                                  |

Directories follow each OS's conventions through Tauri's path resolver; exact locations per
platform are documented with release hardening (task 12.3).

## Build and CI

- `pnpm build:web` builds the UI; `tauri build` embeds it in the binary.
- CI (GitHub Actions) runs web checks, Rust fmt/clippy/tests with a contract drift check, and a
  Linux `.deb` build. Windows and macOS packaging joins in task 12.2.
