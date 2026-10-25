# Threat model

Scope: the aidb desktop app (Rust core, webview UI, isolation hook), its local data, and its
connections to a user's PostgreSQL database and optional AI provider. Controls are described in
[security-model.md](./security-model.md). Review this document whenever a feature adds a new
input, output, permission or dependency.

Status legend: **Done** (implemented and tested) · **Planned** (task number) · **Accepted**
(residual risk, with reason).

## Assets

| Asset                                     | Why it matters                                          |
| ----------------------------------------- | ------------------------------------------------------- |
| Database credentials                      | Full access to the user's data within the role's grants |
| AI provider API keys                      | Billing and account abuse                               |
| Database contents                         | Confidential data; integrity of production systems      |
| Local metadata (profiles, history, audit) | Reveals infrastructure and past queries                 |
| The user's machine                        | The app must not become a path to code execution        |

## Threat sources

- Hostile **model output**: hallucinated or destructive SQL, invented tools, false risk labels.
- **Prompt injection** in database rows, comments, column names, attachments or provider replies.
- A **compromised npm or crate dependency** running code in the webview or core.
- **Hostile content rendered in the UI** (database values, file contents) attempting script
  injection.
- A **malicious or compromised endpoint**: database server or AI provider returning crafted data.
- **User error**: acting on the wrong connection, especially production.
- **Network attackers** between the app and its database or provider.

## Threats and mitigations

| #   | Threat                                                                 | Mitigation                                                                                                                                        | Status                                |
| --- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| T1  | Injected script in the webview calls native commands                   | CSP blocks inline and remote scripts; React escapes rendered text; isolation hook and capability ACL limit reachable commands                     | Done                                  |
| T2  | Compromised frontend dependency calls plugin APIs or unlisted commands | Isolation hook allows only app commands, event plugin and channel transport; ACL grants only listed commands; unencrypted IPC rejected            | Done                                  |
| T3  | Webview navigated to an attacker page that then uses IPC               | Navigation guard blocks non-app URLs; capabilities apply only to the local app origin                                                             | Done                                  |
| T4  | Webview used to exfiltrate data over the network                       | CSP `connect-src` allows IPC only; images and fonts only from the bundle                                                                          | Done                                  |
| T5  | Malicious link opens a dangerous scheme or credential-bearing URL      | `open_external_url` allows only `https`, loopback `http`, `mailto`, and rejects embedded credentials                                              | Done                                  |
| T6  | Secrets leak through errors or logs                                    | Error contract never serializes causes; redaction on messages, causes and panics; commands skip arguments in logs                                 | Done                                  |
| T7  | Secrets persisted in plaintext                                         | OS credential store only, session-only fallback, tests scanning SQLite, logs and exports                                                          | Planned (3.2)                         |
| T8  | Connection to a spoofed database server                                | TLS with certificate verification by default; explicit override for disabled TLS                                                                  | Planned (4.1, 4.2)                    |
| T9  | Model produces destructive SQL                                         | Model never executes; deterministic classification; writes need exact approval; R4 blocked for the agent; database role grants as final boundary  | Planned (6.1, 9.1–9.3, 10.3)          |
| T10 | Model claims a low risk or swaps the target after approval             | Risk computed in Rust, not taken from the model; approval bound to connection, environment, statement, parameters, schema fingerprint; single-use | Planned (9.1, 9.2)                    |
| T11 | Prompt injection in rows, comments or attachments changes behavior     | Content framed as data; cannot add tools, change policy or approve; tool calls validated against a fixed registry                                 | Planned (8.1, 8.2, 8.4)               |
| T12 | Sensitive data sent to a hosted model without consent                  | Schema-first context; row values and files only with explicit opt-in; visible context list before sending                                         | Planned (8.1, 8.3)                    |
| T13 | AI endpoint over plain HTTP intercepted                                | HTTPS required except loopback; explicit acknowledgement otherwise                                                                                | Planned (7.2)                         |
| T14 | Accidental write to production                                         | Environment shown as text and icon; read-only default; approval shows the target; switching connection invalidates approvals                      | Partly done (1.3); Planned (4.3, 9.2) |
| T15 | Unbounded result or file exhausts memory or freezes the UI             | Row/byte caps, pagination, virtualized grid, file size limits, cancellation                                                                       | Planned (5.3, 6.1, 10.4)              |
| T16 | Partial failure leaves the database in an unknown state                | Transactions; uncertain commits reported, never retried automatically; verification step after writes                                             | Planned (10.1, 11.2)                  |
| T17 | SQL injection through identifiers or filter values from the UI         | Identifiers validated against live metadata and quoted; values always bound as parameters                                                         | Planned (5.3)                         |
| T18 | Supply-chain compromise of a Rust crate                                | Locked versions, reviewed additions, minimal plugins, dependency auditing in CI                                                                   | Partly done; audit in 12.2            |
| T19 | CSS injection via inline styles                                        | Cannot execute script under the CSP; inline styles are required by Radix and Sonner                                                               | Accepted                              |

## Out of scope

- An attacker who already controls the user's OS account: local data is protected by the operating
  system, and aidb does not add its own encryption layer (PRD: no bespoke cryptography).
- Over-privileged database roles: aidb cannot add safety the database role does not enforce. Users
  are told to connect with least-privilege roles.
- Provider-side data handling once a user chooses to send context to a hosted model.
