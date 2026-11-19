# aidb

Local-first desktop database client for PostgreSQL with an optional natural-language agent.
Built with Rust + Tauri 2 and React + TypeScript + shadcn/ui.

Status: early development. See [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md).

## Development

Prerequisites:

- Rust via `rustup` (the toolchain is pinned in `rust-toolchain.toml`)
- Node.js 24 LTS (22.12+ works) and pnpm 10
- Tauri system dependencies for your OS: <https://v2.tauri.app/start/prerequisites/>

On Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

Then:

```sh
pnpm install
pnpm dev          # run the desktop app with hot reload
pnpm build        # build release bundles for the current OS
pnpm dev:web      # frontend only, in a browser (native calls are unavailable)
```

Checks (CI runs the same):

```sh
pnpm format:check && pnpm lint && pnpm typecheck && pnpm test
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Database integration tests need a disposable PostgreSQL (Docker, or local PostgreSQL binaries):

```sh
scripts/postgres-fixture.sh start 17
source target/postgres-fixture/env
cargo test -p aidb --test postgres -- --ignored
scripts/postgres-fixture.sh stop
```

## Repository layout

```text
apps/desktop/   React + Vite frontend
src-tauri/      Rust application core (Tauri 2)
docs/           Implementation plan and design documents
scripts/        Developer tooling
```

## License

MIT
