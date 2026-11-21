# Privacy

aidb is a desktop app. It has no account, no server of its own and no cloud sync. This page
says what it keeps on your computer, what it sends elsewhere and when, and how to remove it.

## In short

- **Your databases**: aidb connects straight to the PostgreSQL servers you add. Nothing passes
  through a service run by aidb's makers; there is none.
- **Passwords and API keys**: kept in your operating system's credential store (Windows
  Credential Manager, the macOS Keychain, or the Secret Service on Linux), or only in memory
  for the session if there is none. Never in aidb's files, logs or exports.
- **The AI assistant is optional** and off until you set up a provider. When you use it, your
  question and a description of the database's structure go to the provider you chose, and
  row data or files only when you allow it. Your provider's own terms and privacy policy apply
  to what it receives.
- **No telemetry**: aidb sends no analytics, usage data or crash reports, to anyone.

## Where your data is

| What                                                     | Where                                                                  |
| -------------------------------------------------------- | ---------------------------------------------------------------------- |
| Settings, connection profiles, saved queries and history | `aidb.sqlite3` in the data folder below                                |
| Logs                                                     | The log folder below; one file a day, the last seven kept              |
| Passwords and API keys                                   | Your operating system's credential store, under `com.orbitratech.aidb` |
| Theme and SQL editor drafts                              | The app's web view storage, in the data folder                         |

| System  | Data folder                                          | Log folder                                 |
| ------- | ---------------------------------------------------- | ------------------------------------------ |
| Windows | `%APPDATA%\com.orbitratech.aidb`                     | `%LOCALAPPDATA%\com.orbitratech.aidb\logs` |
| macOS   | `~/Library/Application Support/com.orbitratech.aidb` | `~/Library/Logs/com.orbitratech.aidb`      |
| Linux   | `~/.local/share/com.orbitratech.aidb`                | `~/.local/share/com.orbitratech.aidb/logs` |

Settings › Diagnostics shows the exact folders on your computer.

## What aidb keeps

- **Connection profiles**: name, host, port, database, user, TLS settings, environment and
  access mode. Not the password: that is in the credential store, and only if you chose to
  save it.
- **AI provider profiles**: name, endpoint and model. Not the API key, which is kept like a
  password.
- **Query history** (Library): the SQL you ran, when, how many rows and whether it worked. Not
  the results. Literal passwords in statements such as `ALTER ROLE … PASSWORD '…'` are removed
  before the SQL is saved.
- **Saved queries**: the name and SQL you saved.
- **Assistant tasks**: each question, the steps the assistant took (including the SQL it
  proposed or ran) and its answer. Not query results or attached files.
- **Audit log** (Changes): one record per change applied, refused or cancelled: when, which
  connection and environment, the risk level, the outcome, the number of rows, and a
  fingerprint of the statement. Not the statement's values.
- **Schema cache freshness**: when each connection's schema was last read. The schema itself
  stays in memory while aidb runs.
- **Migrations**: when a migration was applied to which connection and schema, what it was
  compared with, a fingerprint of its script, its risk and its outcome.
- **Logs**: what aidb did and what failed, with counts, timings and error codes. They never
  contain passwords, keys, row data or file contents: command arguments are not logged, error
  text is redacted, and an automated test checks every file aidb writes for leaked secrets.

Query results, table rows and imported files are not written to disk, except where you export
them yourself.

## What leaves your computer, and when

| Destination             | When                                              | What is sent                                                                                                                                                                                                                                                                                                 |
| ----------------------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Your PostgreSQL servers | When you connect, browse, query or apply a change | Your credentials and the statements aidb runs, over TLS unless you turned it off for that connection                                                                                                                                                                                                         |
| Your AI provider        | Only when you ask the assistant a question        | Your question; the schema's tables, columns, keys and comments; earlier exchanges in the conversation. Query results and attached files only if you turned on sharing for that conversation (local providers on this computer need no permission). The context review in Ask shows exactly what will be sent |
| Your web browser        | When you click a link                             | The link opens in your browser, outside aidb                                                                                                                                                                                                                                                                 |

Nothing else: no update checks, no analytics, no error reporting. A "local model" provider on
this computer (such as Ollama) keeps everything on the machine. A hosted provider (Anthropic,
OpenAI or another service) receives what is listed above under its own terms; aidb cannot
control what it does with it.

## How aidb protects what it keeps

- The data folder is protected by your operating system account, like your other files: anyone
  who can sign in as you, or read your disk, can read it. Use your system's disk encryption
  (BitLocker, FileVault or LUKS) to protect it at rest.
- Secrets are left to the operating system's credential store, which uses your login to
  protect them. aidb does not invent its own encryption.
- The assistant never sees passwords or keys and cannot connect to a database or run SQL
  itself: aidb runs only read queries it has checked, and every change waits for your explicit
  approval.

## Removing your data

- **Settings › Storage** clears query history, assistant tasks, the audit log, cached schema
  information, and the SQL editor drafts kept by the web view.
- **Settings › Connections** and **Settings › AI**: forgetting a saved password or key removes
  it from the credential store; deleting a profile can remove its password with it.
- **Uninstalling** removes the program and leaves the data folder, as installers do on every
  platform. Delete the folders above to remove the rest.

## Questions and security reports

Privacy questions: open an issue in the [repository](https://github.com/orbitratechnology/aidb).
Security problems: please report them privately, as [SECURITY.md](../SECURITY.md) explains.
