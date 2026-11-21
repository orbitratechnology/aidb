# Release checklist

How a version of aidb is released, and the evidence that it is ready. Work through it top to
bottom for every release; the PRD's release gates and MVP acceptance criteria are at the end,
each with where it is proven.

## 1. Prepare

- [ ] Every task for the release is done in [the plan](IMPLEMENTATION_PLAN.md), and `main` has
      the release commit.
- [ ] Set the version in all five places, then run `node scripts/check-version.mjs`:
      `Cargo.toml` (`[workspace.package]`), `src-tauri/tauri.conf.json`, `package.json`,
      `apps/desktop/package.json`, `packages/contracts/package.json`. Versions are
      `major.minor.patch` (MSI installers accept nothing else).
- [ ] Write the version's section in [CHANGELOG.md](../CHANGELOG.md) (`## [x.y.z] - date`); it
      becomes the release notes.
- [ ] Re-check the dependency baseline: `cargo audit` and `pnpm audit --prod` are clean (CI
      runs both), and nothing in the lockfiles is a prerelease.
- [ ] Update [supported platforms](supported-platforms.md): the PostgreSQL versions the release
      commit's CI run tested (its `Start PostgreSQL` steps print them), and any new limitation.
- [ ] Update [performance](performance.md) from the release commit's CI run: start times and
      memory from each package job's summary, the interface timings from the end-to-end job,
      the core timings from the PostgreSQL 17 job.
- [ ] Review the [threat model](threat-model.md): no open threat rated critical or high.

## 2. Check

On the release commit, every CI job is green. Together they cover:

- [ ] Web checks, Rust checks, generated contracts current.
- [ ] End-to-end UI tests (layout from 320 px, WCAG 2.2 AA in both themes, keyboard) and the
      interface performance tests.
- [ ] PostgreSQL 15–18 integration, policy, data change, migration, advisor and scripted
      assistant tests on Linux; PostgreSQL 17 integration tests on Windows, macOS Apple Silicon
      and macOS Intel.
- [ ] The OS credential store on Linux (Secret Service), Windows and macOS.
- [ ] The secret scan on all three platforms.
- [ ] Packages built for every platform and smoke-tested: install, start, crash recovery,
      quit, upgrade, uninstall (Windows MSI and NSIS, macOS on both architectures, `.deb` and
      AppImage on Ubuntu 22.04 and 24.04, `.rpm` on Fedora).

Then by hand, on at least one real machine per platform with the release's packages:

- [ ] Install, add a connection to a real PostgreSQL server, browse, run a query, edit a row
      with approval, export CSV, import a CSV, and run the assistant with a local or hosted
      provider.
- [ ] Save a password, quit, start again: it is still saved (OS credential store).
- [ ] Install over the previous release: profiles, saved queries and settings are kept.

## 3. Signing

Until certificates are in place, macOS bundles are signed ad hoc and Windows installers are
unsigned; the release notes must say so, with how to open them
([supported platforms](supported-platforms.md#not-supported-yet)).

- **macOS** (Developer ID, notarization): add repository secrets `APPLE_CERTIFICATE` (the
  `.p12` export, base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (for example
  `Developer ID Application: Orbitra Tech (TEAMID)`), and for notarization `APPLE_ID`,
  `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. The package job passes them
  to Tauri when present. Then confirm on the built `.dmg`:
  `spctl --assess --type execute --verbose aidb.app` reports "Notarized Developer ID".
- **Windows** (Authenticode): Tauri signs through `bundle.windows.certificateThumbprint` (a
  certificate in the runner's store) or `bundle.windows.signCommand` (for example Azure
  Trusted Signing). Configure one, then check the installers with
  `Get-AuthenticodeSignature`. Not wired yet: it depends on which certificate is bought.
- **Linux**: packages are not signed; checksums are published with the release.

## 4. Publish

- [ ] Tag the release commit and push the tag: `git tag v0.1.0 && git push origin v0.1.0`.
- [ ] CI runs everything again on the tag; the **Draft the release** job then creates a draft
      GitHub release with every package, `SHA256SUMS`, and the changelog section as notes.
      If it fails, fix the cause and push a new tag; never edit packages by hand.
- [ ] Review the draft: all packages present (Windows MSI and NSIS, two `.dmg`, `.deb`,
      `.rpm`, AppImage), notes correct, the signing status stated. Publish it.
- [ ] Enable GitHub private vulnerability reporting (Settings › Code security) if it is not on,
      so the link in [SECURITY.md](../SECURITY.md) works.

## 5. After

- [ ] Download each package from the published release and verify it against `SHA256SUMS`.
- [ ] Start a new `## [Unreleased]` section in the changelog.

## Release gates (PRD)

| Gate                                                                                  | Evidence                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No known critical or high security issue in the MVP threat model                      | [Threat model](threat-model.md): every threat is mitigated and tested, except T19 and T34 (accepted, low impact) and T109 (signing, waiting for certificates)                                                                                                                                 |
| No known authorization bypass; every unauthorized action is rejected by the core      | Capability and IPC tests (`src-tauri/src/commands/mod.rs`: `nothing_in_the_webview_can_approve_or_write`, `windows_without_a_capability_cannot_invoke_commands`); policy tests (`tests/policy.rs`)                                                                                            |
| Write paths enforce approval even if the UI is bypassed or the model lies about risk  | `tests/data_changes.rs` (`changes_since_the_preview_are_never_applied`, `read_only_and_production_connections_are_guarded`); scripted assistant cases `model-claims-low-risk`, `injection-asks-for-approval`, `injected-delete-only-proposes`                                                 |
| PostgreSQL integration and packaging smoke tests pass on Windows, macOS and Linux     | CI jobs: PostgreSQL 15–18, Windows x64 / macOS tests, the package jobs and the Ubuntu and Fedora smoke tests                                                                                                                                                                                  |
| No credentials or API keys in logs, local history or exports                          | The secret scan (`src-tauri/src/commands/secret_scan.rs`), on all three platforms                                                                                                                                                                                                             |
| Read, query and assistant work can be cancelled without freezing or corrupting the UI | `tests/postgres.rs` (`running_queries_can_be_cancelled`, `page_reads_can_be_cancelled`, `exports_can_be_cancelled`); `cancelling_stops_the_running_query_in_the_database` (assistant); `tests/ai_providers.rs` (`stops_when_cancelled`)                                                       |
| Import and export are tested with malformed and large files and type errors           | `src-tauri/src/domain/csv_import.rs` (`bad_input_is_refused`, `the_wrong_encoding_is_reported_not_hidden`, `short_rows_are_padded_and_reported`), `tests/data_changes.rs`, connection file tests (`whole_files_are_refused_for_the_right_reasons`, `files_with_too_many_entries_are_refused`) |
| Known platform limitations are documented                                             | [Supported platforms](supported-platforms.md#not-supported-yet)                                                                                                                                                                                                                               |

## MVP acceptance criteria (PRD)

| Criterion                                                                | Evidence                                                                                                       |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| Create, test, save, edit and delete multiple PostgreSQL profiles         | Tasks 4.1–4.3; `session_commands_work_over_ipc`                                                                |
| No plain-text credentials in settings, SQLite, logs or exported profiles | Secret scan; `passwords_in_hand_edited_files_are_ignored`; `exports_carry_format_version_and_no_local_details` |
| Active connection and environment always shown                           | Top bar (task 4.3), checked at every width by the end-to-end tests                                             |
| Browse schema metadata; bounded, paginated table view                    | Tasks 5.1–5.4; performance tests (10,000 objects, 500-row pages)                                               |
| Sort, filter, inspect values, export to CSV/JSON                         | Tasks 5.3–5.5; export tests in `tests/postgres.rs`                                                             |
| Read-only SQL, cancellable                                               | Task 6.1; `writes_hidden_in_functions_are_stopped_by_the_read_only_transaction`; cancellation tests            |
| AI provider or local Ollama; manual features need no AI                  | Tasks 7.1–7.3; the app starts and works with the assistant off                                                 |
| Schema questions and bounded read queries grounded in the schema         | Task 8.4 evaluation suite                                                                                      |
| Assumptions and generated SQL shown                                      | Ask screen (task 8.3)                                                                                          |
| Malformed or unsupported tool calls rejected safely                      | Scripted cases `malformed-and-smuggled-arguments`, `relentless-unknown-tools`                                  |
| The model cannot reach credentials or open a connection                  | Architecture (the model only calls the core's tools); secret scan; `session-and-server-functions`              |
| Row edit with before/after preview                                       | Task 10.2; `changes_are_previewed_applied_and_verified`                                                        |
| CSV import with mapping and validation preview                           | Task 10.4                                                                                                      |
| Agent writes need a fresh, exact approval, invalidated by any change     | Tasks 9.2, 10.3; `changes_since_the_preview_are_never_applied`                                                 |
| High-risk operations blocked in agent tools                              | Task 9.1; scripted `comment-injection-asks-for-a-drop`                                                         |
| Migration SQL readable, reviewable, never applied without approval       | Tasks 11.1–11.2                                                                                                |
| Post-operation status and verification match the database                | Data change and migration verification tests                                                                   |
| Neutral light theme and dark theme                                       | Accessibility tests run every screen in both                                                                   |
| 320 px to desktop without clipped primary actions                        | Responsive tests at six widths                                                                                 |
| Bottom navigation and 44 px touch targets on phones                      | Responsive tests                                                                                               |
| Windows, macOS and Linux builds install and pass smoke tests             | Package and smoke-test jobs                                                                                    |
| No hosted backend, app authentication or cloud sync                      | Architecture; [privacy](privacy.md)                                                                            |
