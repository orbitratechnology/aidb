# Security policy

aidb handles database credentials, API keys and access to production data, so security
reports are welcome and taken seriously.

## Reporting a vulnerability

Please report it privately, not in a public issue: use **Report a vulnerability** on the
repository's [Security tab](https://github.com/orbitratechnology/aidb/security/advisories/new)
(GitHub private vulnerability reporting). Include what you found, how to reproduce it, the
version and platform, and what an attacker could do with it.

You can expect an acknowledgement within a week. We will keep you informed while we work on a
fix, agree on a disclosure date with you, and credit you in the release notes unless you prefer
not to be named.

## Supported versions

Only the latest release receives security fixes while aidb is in its 0.x versions.

## Scope

In scope: anything that lets someone, or the AI model, get around aidb's protections. For
example: reading a stored password or API key; making a change to a database without the
user's approval; running SQL the policy refuses; reaching commands, files or network
destinations the app does not grant; leaking credentials or row data into logs, history or
exports; or running script through content shown in the app.

Out of scope: what a person who can already sign in as the user can do with their files (see
[privacy](docs/privacy.md#how-aidb-protects-what-it-keeps)); the security of PostgreSQL servers
or AI providers themselves; and what a database role is allowed to do when the user approves
the change.

How aidb is meant to be secure is described in the [security model](docs/security-model.md)
and the [threat model](docs/threat-model.md).
