# aidb

Local-first desktop database client for PostgreSQL with an optional natural-language agent.
Built with Rust + Tauri 2 and React + TypeScript + shadcn/ui.

Status: preparing the first release (0.1.0) for a private beta. See the
[changelog](CHANGELOG.md) and the [implementation plan](docs/IMPLEMENTATION_PLAN.md).

## Install

Packages for Windows x64 (MSI or NSIS installer), macOS (Apple Silicon and Intel) and Linux x64
(`.deb`, `.rpm`, AppImage) are attached to each release; check them against `SHA256SUMS`.
Requirements and known limitations are in [supported platforms](docs/supported-platforms.md).

Until release signing is in place, macOS asks for confirmation the first time (Control-click
the app, then Open) and Windows SmartScreen warns before the installer runs.

## Privacy and security

aidb has no account, no server of its own and no telemetry; it connects straight to your
databases and, if you turn the assistant on, to the AI provider you choose. Passwords and API
keys stay in your operating system's credential store. Details: [privacy](docs/privacy.md).
Report security problems privately, as described in [SECURITY.md](SECURITY.md).

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

End-to-end UI tests (Playwright, the interface on fixture data):

```sh
pnpm e2e                                   # layout, accessibility, keyboard
pnpm --filter @aidb/desktop e2e:performance  # production build, large database
```

Database integration tests need a disposable PostgreSQL (Docker, or local PostgreSQL binaries):

```sh
scripts/postgres-fixture.sh start 17
source target/postgres-fixture/env
cargo test -p aidb --test postgres -- --ignored
scripts/postgres-fixture.sh stop
```

Install smoke tests for built packages: `node scripts/smoke-test.mjs <format> <package>
<newer-package>` (see the script for formats).

## Documentation

- [Supported platforms](docs/supported-platforms.md) and [performance](docs/performance.md)
- [Privacy](docs/privacy.md), [security model](docs/security-model.md),
  [threat model](docs/threat-model.md)
- [Architecture](docs/architecture.md) and [decision log](docs/decisions.md)
- [Release checklist](docs/release-checklist.md)

## Repository layout

```text
apps/desktop/   React + Vite frontend
src-tauri/      Rust application core (Tauri 2)
docs/           Implementation plan and design documents
scripts/        Developer tooling
```

## License

MIT
