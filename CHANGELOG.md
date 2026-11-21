# Changelog

All notable changes to aidb. The format follows [Keep a Changelog](https://keepachangelog.com/),
and versions follow [Semantic Versioning](https://semver.org/). The release workflow publishes a
version's section as its release notes.

## [0.1.0] - unreleased

The first release, for a private beta.

### Connections and data

- PostgreSQL 15–18 connection profiles with TLS verified by default, a production / staging /
  development label shown everywhere, and read-only access until you turn writes on.
- Passwords and API keys in the operating system's credential store, or for the session only.
- Schema browser, a paginated data grid with sorting and filters, row details, copy, and
  CSV/JSON export.
- SQL editor with tabs, read-only execution, cancellation, history and saved queries.
- Query advisor: plans with suggestions grounded in the plan's own numbers.

### Changes, imports and migrations

- Row edits, inserts and deletes previewed with before/after values, applied only after your
  approval, in a transaction.
- CSV import with column mapping, validation and a preview before anything is written.
- Schema comparison between connections or with a `.sql` file, and reviewable migration scripts
  applied with explicit approval and verified afterwards.
- An audit log of every change applied, refused or cancelled.

### Assistant (optional)

- Ask questions about a database in plain language with Anthropic, OpenAI, Ollama or any
  OpenAI-compatible provider. Read queries are checked and run by aidb; changes the assistant
  proposes wait for your approval; destructive operations are refused.
- A review of exactly what will be sent to the provider; query results and files only with
  your permission.

### Platforms

- Windows x64 (MSI, NSIS), macOS Apple Silicon and Intel (`.dmg`), Linux x64 (`.deb`, `.rpm`,
  AppImage). See [supported platforms](docs/supported-platforms.md).
- Light and dark themes; layouts from 320 px phones to large desktops; keyboard and screen
  reader support to WCAG 2.2 AA.
