# Implementation plan

This plan breaks the Agentic Database Client Essential MVP PRD (v1.0, kept outside this
repository) into phases and tasks. It follows the PRD's implementation sequence (section 18). Each task is a vertical slice that leaves the repository
buildable and tested. Work stops after each task for review, and the next task starts only when
the maintainer says so.

Status legend: `[x]` done · `[~]` in progress · `[ ]` not started

## Phase 0: Agent tooling

- [x] **0.1 Install agent skills.** Pinned Rust (`leonardomso/rust-skills`), Tauri v2
      (`dchuk/claude-code-tauri-skills`) and shadcn/ui (`shadcn/ui` official) skills via
      `scripts/install-agent-skills.sh` into `.claude/skills/` (git-ignored). Plan written.

## Phase 1: Scaffold and design system

- [x] **1.1 Workspace scaffold.** pnpm workspace, `apps/desktop` (Vite + React + strict TypeScript),
      `src-tauri` (Tauri 2) at the root, Cargo workspace, `rust-toolchain.toml`, pinned versions,
      `pnpm tauri dev/build` wiring, minimal capability file and CSP. Verify `cargo check`,
      `cargo clippy`, `pnpm build`.
- [ ] **1.2 Tailwind v4 + shadcn/ui.** Initialize shadcn (Radix base, neutral palette), semantic
      tokens for light and dark themes, theme provider (system/light/dark), reduced-motion support,
      system UI and monospace font stacks, base components.
- [ ] **1.3 Responsive app shell.** Routes for Ask, Data, SQL, Changes, Settings. Desktop sidebar,
      compact bottom navigation with 44px targets, Settings in a top-level menu on narrow screens,
      top bar with active-connection and environment identity (text, not only color). Empty states
      that explain the next action; no mock data.
- [ ] **1.4 Quality tooling and CI.** Linting (oxlint from the Vite template, or ESLint if a needed
      rule is missing), Prettier, Vitest + React Testing Library, rustfmt and clippy configuration,
      GitHub Actions for lint, typecheck, unit tests and a Linux build.

## Phase 2: Native command boundary

- [ ] **2.1 Rust core layout and error model.** `commands/`, `domain/`, `adapters/`, `services/`,
      `events/`, `errors/` modules. Structured `AppError` (code, safe message, retryable) with
      redaction. Shared app state.
- [ ] **2.2 Typed IPC.** Typed commands and events, Zod-validated DTOs in `packages/contracts`,
      `lib/tauri.ts` invoke wrappers, capability allowlist, strict CSP, external-link handling.
- [ ] **2.3 Security documentation.** `docs/architecture.md`, `docs/security-model.md`,
      `docs/threat-model.md`, written before any database writes exist.

## Phase 3: Local persistence and credentials

- [ ] **3.1 Local SQLite store.** SQLx SQLite with versioned migrations for `connection_profiles`,
      `provider_profiles`, `app_settings`, `saved_queries`, `agent_tasks`, `operation_audit`,
      `schema_cache_metadata`, `migration_records`. Repository layer and tests.
- [ ] **3.2 Credential store.** OS keyring backend (Windows Credential Manager, macOS Keychain,
      Secret Service) with a session-only fallback and no plaintext fallback. Tests proving secrets
      never reach SQLite, logs or exports.
- [ ] **3.3 Settings.** Settings commands and UI: appearance, safety defaults, storage controls
      (clear history, clear schema cache), diagnostics.

## Phase 4: PostgreSQL connections

- [ ] **4.1 Connection profiles.** Validation, environment labels, SSL modes, URI convenience input
      parsed without persisting passwords, CRUD commands, versioned JSON export and import with
      secrets omitted.
- [ ] **4.2 PostgreSQL adapter.** `DatabaseAdapter` trait with capabilities, SQLx Postgres pools,
      TLS options, connect/test/disconnect, connection and statement timeouts, cancellation, error
      mapping, and integration fixtures against a disposable PostgreSQL.
- [ ] **4.3 Connections UI.** Connection form (React Hook Form + Zod), list, test, connect and
      disconnect, the "Forget connection" flow, live connection and environment identity.

## Phase 5: Schema browser and data grid

- [ ] **5.1 Catalog introspection.** Schemas, tables, views, columns, types, nullability, primary,
      foreign and unique keys, indexes, row estimates. In-memory cache with invalidation.
- [ ] **5.2 Schema browser UI.** Searchable object tree, table definition and relationship panels,
      sheet-based object picker on compact screens.
- [ ] **5.3 Table paging.** Validated identifiers, parameterized filters, sorting, bounded pages,
      safe type mapping (NULL, JSON/JSONB, arrays, UUID, numeric, timestamps, long text).
- [ ] **5.4 Data grid UI.** TanStack Table + Virtual grid with sticky headers, column
      visibility/resize, a cell and row detail panel, and a compact record list layout.
- [ ] **5.5 Export and copy.** CSV and JSON export with visible truncation; copy cell and row.

## Phase 6: SQL editor and read execution

- [ ] **6.1 Statement classification and execution.** `sqlparser` classification, read-only mode,
      multi-statement guard, row/byte caps, statement timeout, cancellation, error positions.
- [ ] **6.2 SQL editor.** Lazy-loaded CodeMirror 6 with PostgreSQL highlighting, tabs with
      preserved drafts, run selection or current statement, keyboard shortcuts.
- [ ] **6.3 Results and history.** Results panel, pagination, errors, safe query history and saved
      queries.

## Phase 7: AI providers

- [ ] **7.1 Provider abstraction.** `AiProvider` trait (streaming, structured output, tool calls,
      cancellation, capabilities). OpenAI-compatible adapter (covers Ollama and local endpoints) and
      an Anthropic adapter.
- [ ] **7.2 Provider profiles.** Storage with keyring-held API keys, endpoint trust policy (plain
      HTTP only for loopback unless acknowledged), "Test provider" command.
- [ ] **7.3 Provider settings UI.** Configure, test and disable AI; disclosure of what is sent.

## Phase 8: Read-only agent

- [ ] **8.1 Agent domain.** Versioned task plan, typed tool registry (R0/R1 tools), context builder
      with budgets and untrusted-content framing.
- [ ] **8.2 Planner loop.** Tool-call validation (unknown or malformed calls rejected), cancellation,
      verification, task persistence.
- [ ] **8.3 Ask UI.** Composer, context review list, attachments (`.sql`, `.csv`, `.json`, `.md`,
      `.txt`), streamed activity trace, results and follow-ups.
- [ ] **8.4 Agent evaluation suite.** Scripted provider fixtures, read-task benchmarks, injection
      cases.

## Phase 9: Policy and approval engine

- [ ] **9.1 Risk classifier.** Deterministic R0–R4 classification from statement, environment,
      permissions and estimated impact.
- [ ] **9.2 Approvals.** State machine, digest binding, short expiry, single use, invalidation on
      connection or environment change.
- [ ] **9.3 Audit and Changes view.** Redacted audit events, Changes/History UI, bypass and
      prompt-injection tests.

## Phase 10: Controlled writes and CSV import

- [ ] **10.1 Data change engine.** INSERT/UPDATE/DELETE preview, predicate requirement, impact
      estimate, transactional apply with an approval token, verification.
- [ ] **10.2 Grid editing.** Pending-change buffer, before/after diff, Save/Discard, delete
      confirmation.
- [ ] **10.3 Agent writes.** `preview_data_change` / `apply_approved_data_change` wired to the
      approval UI.
- [ ] **10.4 CSV import.** Encoding/delimiter/header detection, column mapping, validation preview,
      insert-only default, explicit upsert, transactional apply.

## Phase 11: Migrations and query advisor

- [ ] **11.1 Schema diff.** Schema-to-schema and schema-to-`.sql` comparison, SQL generation, risk
      notes (data loss, locks, non-transactional statements).
- [ ] **11.2 Migration workflow.** Review, approval, apply and `migration_records`.
- [ ] **11.3 Query advisor.** `EXPLAIN (FORMAT JSON)` plan tree, evidence-based suggestions, agent
      explanation.

## Phase 12: Hardening and release

- [ ] **12.1 Responsive and accessibility audit.** 320px through 1440px+, touch targets, keyboard
      navigation, Playwright responsive tests.
- [ ] **12.2 Cross-platform builds.** Windows, macOS and Linux CI packaging, install smoke tests,
      secret-scanning tests.
- [ ] **12.3 Release readiness.** Performance profiling, user-facing privacy and security docs,
      supported platforms, release checklist.

## Version baseline (checked 24 October 2026)

| Component        | Pinned       | Notes                                                             |
| ---------------- | ------------ | ----------------------------------------------------------------- |
| Rust             | 1.99.1       | `rust-toolchain.toml`                                             |
| Tauri (crate/JS) | 2.12.x       | 3.0 is alpha; stay on 2.x                                         |
| React            | 19.3         |                                                                   |
| Vite             | 8.3          |                                                                   |
| TypeScript       | 6.0          | 7.0 is out, but `typescript-eslint` supports only `<6.1` for now |
| Tailwind CSS     | 4.3          |                                                                   |
| SQLx             | 0.9          |                                                                   |
| reqwest          | 0.13         |                                                                   |
| keyring          | 4.x          |                                                                   |
| sqlparser        | 0.63         |                                                                   |
