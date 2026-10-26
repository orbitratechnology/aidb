# Security model

This document states the rules aidb enforces, the controls that implement them today, and the
controls that must exist before each risky feature ships. It is written before any database
writes exist and is updated with every feature that touches a boundary. Threats and their
mitigations are listed in [threat-model.md](./threat-model.md).

## Principles

1. **The Rust core is the only authority.** It holds credentials, opens connections, classifies
   operations, issues and checks approvals, and executes. The UI and the model only request.
2. **The model never executes anything.** It proposes typed plans; deterministic Rust code
   validates them, and writes need a fresh, exact, single-use approval.
3. **Secrets never leave their store.** Database passwords and API keys live in the OS credential
   store or in memory for the session. Never in SQLite, logs, exports, prompts, errors or the UI
   after entry.
4. **Untrusted by default.** Database content, comments, files, model output and provider
   responses are data, never instructions, and never change policy.
5. **Fail closed.** Unknown commands, unclassifiable statements, malformed plans and failed
   approval checks are rejected and audited.
6. **Local only.** No app server, account, telemetry or cloud sync. Network traffic goes only to
   the database and AI provider the user configured.

## Trust boundaries

| Boundary                | Trusted side | Untrusted side                              | Enforced by                                              |
| ----------------------- | ------------ | ------------------------------------------- | -------------------------------------------------------- |
| Webview → core (IPC)    | Rust core    | UI code, npm dependencies, rendered content | Isolation hook, capability ACL, input validation in Rust |
| Core → PostgreSQL       | Rust core    | Server responses, row data, error text      | TLS, role grants, bounded reads, redaction (from 4.2)    |
| Core → AI provider      | Rust core    | Model output, tool calls, provider errors   | Typed tool registry, plan validation, policy (from 7.1)  |
| User files → core       | Rust core    | CSV, SQL, JSON, Markdown, text attachments  | Size caps, parsers with limits, no execution (from 8.3)  |
| Webview → outside world | —            | Any URL                                     | CSP, navigation guard, validated `open_external_url`     |

## Controls in place

| Control                 | What it does                                                                                                                                                                                                                                                                                                                                                                | Where                                                                                   |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Isolation pattern       | Every IPC message passes a sandboxed hook that allows only app commands, the event plugin and channel transport, then is encrypted with a per-launch key. Unencrypted messages are rejected.                                                                                                                                                                                | `src-tauri/isolation/`, `tauri.conf.json`                                               |
| Capability ACL          | Each command has an `allow-*` permission; the main window is granted only `core:event:default` and the app's own commands. Unknown, ungranted-window and ungranted-plugin calls are rejected (tested).                                                                                                                                                                      | `src-tauri/build.rs`, `src-tauri/capabilities/`                                         |
| Content Security Policy | Scripts only from the bundle (hashed), no plugins/objects, no form posts, network limited to IPC.                                                                                                                                                                                                                                                                           | `tauri.conf.json`                                                                       |
| Prototype freezing      | `Object.prototype` is frozen to block prototype pollution.                                                                                                                                                                                                                                                                                                                  | `tauri.conf.json`                                                                       |
| Navigation guard        | The webview cannot leave bundled content; blocked attempts are logged by scheme and host only.                                                                                                                                                                                                                                                                              | `src-tauri/src/navigation.rs`                                                           |
| External links          | Only `https`, loopback `http` and `mailto` URLs without credentials open, and only in the system browser.                                                                                                                                                                                                                                                                   | `src-tauri/src/commands/external.rs`                                                    |
| Error contract          | The UI receives code, safe message and retry hint only; causes stay in local logs.                                                                                                                                                                                                                                                                                          | `src-tauri/src/errors/`                                                                 |
| Redaction               | Connection-URI passwords, key/value secrets, auth headers, bearer tokens and common API key formats are scrubbed from messages, logged causes and panics.                                                                                                                                                                                                                   | `src-tauri/src/errors/redact.rs`                                                        |
| Logging discipline      | Fallible commands use `#[instrument(skip_all, err(Debug))]`, so arguments are not logged (a few non-sensitive values, such as which data was cleared, are named explicitly) and errors are logged once, redacted. The webview cannot write to logs.                                                                                                                         | `src-tauri/src/commands/mod.rs`, `src-tauri/src/logging.rs`                             |
| Response validation     | The UI validates every native response against its schema before use.                                                                                                                                                                                                                                                                                                       | `apps/desktop/src/lib/native.ts`                                                        |
| No telemetry            | Nothing is sent anywhere except user-configured endpoints.                                                                                                                                                                                                                                                                                                                  | —                                                                                       |
| Local store integrity   | `STRICT` tables and CHECK constraints mirror domain limits; the audit table rejects updates; `secret_ref` columns hold references only; a backup is written before schema upgrades.                                                                                                                                                                                         | `src-tauri/migrations/`, `src-tauri/src/adapters/local_store/`                          |
| Credential storage      | Secrets go to the OS store via `keyring` (Keychain, Credential Manager, Secret Service) or, if none answers at startup, to memory for the session, which Diagnostics states. Secrets stay in `SecretString` (redacted `Debug`, zeroed on drop); keyring errors that carry secret bytes are zeroed and dropped; the database records a reference only for durable secrets.   | `src-tauri/src/adapters/credentials.rs`, `src-tauri/src/services/connection_secrets.rs` |
| Settings and local data | Settings are validated in Rust before saving (ranges, lock timeout within statement timeout, unknown sections rejected); a damaged stored value loads as defaults. Clearing accepts only three fixed targets (task history, audit log, schema cache) mapped to literal statements, is confirmed in the UI, and is recorded in the file log, which the webview cannot alter. | `src-tauri/src/domain/settings.rs`, `src-tauri/src/adapters/local_store/maintenance.rs` |

Accepted risk: `style-src` allows inline styles (Radix overlays and Sonner inject `<style>`
elements). This permits CSS injection, not script execution. See [decisions.md](./decisions.md).

## Gates before features ship

Each gate must be implemented and tested before the feature that depends on it is enabled.

| Gate                                                                                                                                                                                                                  | Required before                   | Task                             |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | -------------------------------- |
| OS credential store with session-only fallback; no plaintext fallback; tests prove secrets never reach SQLite, logs or exports                                                                                        | Saving any connection or API key  | Done (3.2); export test with 4.1 |
| TLS on by default with certificate verification; disabling TLS for a remote host needs an explicit, recorded override                                                                                                 | First PostgreSQL connection       | 4.1, 4.2                         |
| New connections read-only by default; statement timeouts and row/byte caps on every read                                                                                                                              | Running queries                   | 4.2, 6.1                         |
| Statement classification with `sqlparser` plus database-side checks; unknown or multi-statement agent SQL rejected                                                                                                    | Manual SQL execution, agent reads | 6.1, 8.1                         |
| AI endpoints: HTTPS required except loopback; explicit acknowledgement for insecure remote endpoints; disclosure of what is sent                                                                                      | Any provider request              | 7.2                              |
| Context minimization: schema-first, row data and files only with explicit consent; budgets with visible truncation                                                                                                    | Agent with hosted providers       | 8.1, 8.3                         |
| Deterministic R0–R4 risk classifier; approval state machine; approvals bound to connection, environment, statement, parameters, schema fingerprint and policy version; short-lived and single-use; redacted audit log | **Any write, agent or manual**    | 9.1–9.3                          |
| R4 (destructive/privileged) blocked in agent tools                                                                                                                                                                    | Agent writes                      | 9.1, 10.3                        |
| Transactions with rollback on failure; uncertain commit outcomes reported, never retried automatically                                                                                                                | Data changes, imports, migrations | 10.1–11.2                        |

## Rules for contributors

**Adding a command:** validate every argument in Rust, return `AppResult`, instrument with
`skip_all`, list it in `build.rs`, grant it in a capability, add its name to `COMMANDS` in the
contracts package, and add an ACL test if it is sensitive.

**Adding a plugin:** only official or well-maintained plugins; review its permissions; grant the
minimum; prefer calling it from Rust after validation rather than granting webview access. Plugin
commands reachable from the webview must also be allowed in the isolation hook.

**Handling secrets:** never format a secret into a message, log line, error or event. Wrap secret
values in a type whose `Debug` and `Display` print `[redacted]`.

**Rendering untrusted content:** render data as text. Never use `dangerouslySetInnerHTML` for
database values, model output or file contents.
