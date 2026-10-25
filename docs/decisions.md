# Decision log

Technical decisions for aidb, with the reason for each. Add an entry whenever a dependency,
pattern or default is chosen or changed.

## How decisions are made

1. **Reliability and maturity first.** Use stable releases only. No alpha, beta, release-candidate
   or nightly packages for shipped functionality.
2. **Official recommendations next.** Prefer what a tool's maintainers recommend: official
   templates, plugins, CLIs and documented patterns over community alternatives.
3. **Smallest sufficient surface.** Add a dependency, plugin or permission only when a feature
   needs it, and remove unused ones.
4. **Pin and record.** Lock versions (`Cargo.lock`, `pnpm-lock.yaml`, `rust-toolchain.toml`) and
   write the decision here.

When these conflict, safety and reliability win, and the trade-off is written down.

## Platform and toolchain

| Decision                                | Why                                                                                                                                              | Alternatives                                          |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------- |
| Tauri 2.12 (stable line)                | Required by the PRD; 3.0 is alpha.                                                                                                               | Tauri 3 alpha (not stable).                           |
| Rust 1.99.1, pinned                     | Current stable; `rust-toolchain.toml` makes local and CI builds identical.                                                                       | Floating `stable`.                                    |
| Node.js 24 LTS in CI, `>=22.12` locally | Node recommends Active LTS for production; 22.12 is the Vite 8 minimum.                                                                          | Node 26 (becomes LTS later).                          |
| pnpm 10.28                              | Mature, already the lockfile format here; a jump to pnpm 12 is evaluated with release hardening (task 12.2).                                     | pnpm 12.                                              |
| TypeScript 6.0                          | The official `create-vite` template pins 6.0; TypeScript 7.0 (the native port) is new and tools such as typescript-eslint do not support it yet. | TypeScript 7.0.                                       |
| oxlint                                  | The linter the official `create-vite` React template ships.                                                                                      | ESLint (would be added only for a rule oxlint lacks). |
| Prettier + official Tailwind plugin     | De facto formatter; class sorting from the Tailwind team's plugin. shadcn component files are excluded so CLI updates diff cleanly.              | Biome.                                                |

## Frontend

| Decision                                                                  | Why                                                                                                                              | Alternatives                      |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- |
| shadcn/ui via the official CLI, Radix base, `vega` style, neutral palette | PRD asks for shadcn on Radix with the `new-york` look; `vega` is its successor in the current CLI. Components stay CLI-managed.  | Base UI primitives.               |
| Local theme provider                                                      | shadcn's official Vite dark-mode guide uses a small provider rather than `next-themes`, whose inline script the CSP would block. | `next-themes`.                    |
| System font stacks                                                        | PRD: system UI for the interface, monospace for SQL and numbers; no bundled web font.                                            | Inter (preset default).           |
| React Router, hash history, lazy routes                                   | Mature router; hash history needs no server-side fallback from the bundled `tauri://` origin.                                    | Browser history; TanStack Router. |
| Zustand for cross-screen UI state                                         | PRD choice; small and stable.                                                                                                    | React context only.               |
| Zod 4 for runtime validation                                              | PRD choice; validates every native response.                                                                                     | Valibot.                          |
| Vitest for pure logic only                                                | Maintainer decision: no React Testing Library component tests. UI behavior is verified end to end in the running app.            | RTL component tests.              |

## Native core

| Decision                                                         | Why                                                                                                                                                                                                                       | Alternatives                                    |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Tauri isolation pattern                                          | Tauri recommends it whenever feasible, especially for apps with many frontend dependencies. Every IPC message passes a sandboxed hook and is encrypted per launch, which limits what a compromised npm package can reach. | Brownfield (default) pattern.                   |
| App command ACL via `tauri-build` `AppManifest`                  | Official mechanism: each command gets an `allow-*` permission that a capability must grant.                                                                                                                               | Unchecked app commands (Tauri's default).       |
| Navigation guard plugin (`on_navigation`)                        | Official hook; the webview never leaves bundled content.                                                                                                                                                                  | Relying on CSP alone.                           |
| `tauri-plugin-opener` 2.x, used only from Rust                   | Official plugin for opening URLs. Its webview commands are not granted and its injected link script is disabled; Rust validates URLs first.                                                                               | `open` crate directly.                          |
| `ts-rs` 12 for TypeScript contracts                              | Stable release that generates types from Rust DTOs. `tauri-specta`/`specta` would also type commands but are still release candidates.                                                                                    | `tauri-specta` (RC).                            |
| `thiserror` for typed errors, one `AppError` at the IPC boundary | Standard in the Rust ecosystem; stable error codes and redacted messages are the UI contract.                                                                                                                             | `anyhow` everywhere.                            |
| `tracing` with a rotating file appender                          | The Rust ecosystem standard (Tokio project). Logging stays inside the core; the webview cannot write to logs, which keeps untrusted content out.                                                                          | `tauri-plugin-log` (also lets the webview log). |
| Regex-based redaction                                            | Defense in depth for untrusted driver/provider text; `regex` is the standard crate.                                                                                                                                       | No redaction.                                   |

## Security configuration

| Decision                                                                                           | Why                                                                                                                                                                                                                                     |
| -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Strict CSP: `script-src 'self'`, `object-src 'none'`, `form-action 'none'`, IPC-only `connect-src` | Blocks injected scripts and outbound requests from the webview. All network traffic goes through the Rust core.                                                                                                                         |
| `style-src 'self' 'unsafe-inline'` with `dangerousDisableAssetCspModification: ["style-src"]`      | Radix overlays and Sonner insert `<style>` elements at runtime; Tauri's nonces would otherwise disable `'unsafe-inline'` and break them. Accepted risk: CSS injection without script execution. Scripts keep Tauri's hashes and nonces. |
| No explicit `frame-src`                                                                            | The isolation iframe's origin is generated per build and appended to `default-src` by Tauri; an explicit `frame-src` would block it.                                                                                                    |
| `freezePrototype: true`                                                                            | Official option that blocks prototype pollution from dependencies.                                                                                                                                                                      |
| Minimal capability                                                                                 | Main window gets `core:event:default` plus the app's own commands; nothing else.                                                                                                                                                        |
