# aidb: agent notes

Local-first PostgreSQL desktop client with an optional natural-language agent.
Stack: Rust + Tauri 2 core, React + TypeScript + Vite + Tailwind v4 + shadcn/ui (Radix) frontend.

## Workflow

- Work follows `docs/IMPLEMENTATION_PLAN.md`, one task at a time. Finish a task, run its checks,
  tick it in the plan, commit, push, then **stop and wait** for the maintainer to say continue.
- Never replace working functionality with mock data. Fixtures belong only in tests.

## Agent skills

Run `scripts/install-agent-skills.sh` once per fresh checkout. It installs pinned skills into
`.claude/skills/` (git-ignored):

- `rust-skills`: Rust rules (read `SKILL.md`, then only the `rules/*.md` that apply).
- `tauri-*`: Tauri v2 guidance (capabilities, CSP, IPC, testing, packaging, ...).
- `shadcn`: shadcn/ui rules. Add components with `pnpm dlx shadcn@latest add <name>` from
  `apps/desktop`; never hand-copy component source.

## Layout

- `src-tauri/`: Rust application core (the only code that touches databases, credentials, AI
  providers). Commands stay thin; logic lives in `domain/`, `services/`, `adapters/`.
- `apps/desktop/`: React UI. Native calls go through typed wrappers in `src/lib/`.
- `docs/`: plan, architecture, security model.

## Commands

- `pnpm install`: install JS dependencies.
- `pnpm dev`: run the desktop app (`tauri dev`).
- `pnpm build:web`: build the frontend only.
- `pnpm format` / `pnpm format:check` / `pnpm lint` / `pnpm typecheck` / `pnpm test`: web checks.
- `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace`: Rust checks. `pnpm build:web` must run first (Tauri embeds the dist).

## Testing

- Rust: unit and integration tests for all policy, persistence and database logic.
- Frontend: Vitest only for pure logic (`src/**/*.test.ts`). Do not write React Testing Library
  component tests. UI behavior is verified end to end (Playwright, running app).

## Non-negotiables (from the PRD)

- The model never executes SQL, sees credentials, or bypasses the Rust policy/approval engine.
- Secrets live only in the OS credential store (or session memory); never in SQLite, logs, exports.
- New connections are read-only by default; writes need preview + explicit approval.
- Use semantic shadcn tokens; environment status is never conveyed by color alone.
