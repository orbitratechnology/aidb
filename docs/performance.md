# Performance

The PRD sets engineering targets, not guarantees: a usable window within 3 seconds of a cold
launch, screens that respond within 100 ms once loaded, progressive loading for large schemas,
bounded grids, and bundle size and memory kept visible. This page records how aidb measures up
and how each number is produced, so a regression shows up in CI rather than in use.

## How it is measured

| What                            | How                                                                                                                                                                               | Where it runs                            |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| Native start, memory            | The install smoke test launches the installed app and reads, from its own log, when the interface first reached the core; memory is the app's resident set five seconds later     | Every package, every push (job summary)  |
| Core with a large database      | `src-tauri/tests/performance.rs`: 2,000 tables, 200 views and a 300,000-row table, through the services the interface calls                                                       | PostgreSQL 17 job, every push            |
| Interface with a large database | `apps/desktop/e2e/performance/`: the production build in Chromium, 10,000 catalog objects and 500-row pages, timed inside the page from action to the frame that shows the result | End-to-end job, every push (job summary) |
| Interface bundle size           | `scripts/size-report.mjs`                                                                                                                                                         | Web job, every push (job summary)        |
| Package sizes                   | Listed after packaging                                                                                                                                                            | Every package job (job summary)          |

Budgets in the tests are several times the targets, so a slow CI machine does not fail a run
but a regression of an order of magnitude does. The figures below come from CI machines and a
development container; a developer laptop is usually faster.

## Results

### Starting the app

Time from launching the installed app to its interface talking to the core. The first start of
a fresh install includes one-time work (the webview's first start, creating the local
database); later starts are the everyday case.

CI run 44877450530 (21 November 2026), GitHub-hosted runners:

| Package                | First start | After a crash | After an upgrade | Memory after start |
| ---------------------- | ----------- | ------------- | ---------------- | ------------------ |
| Windows MSI            | 8.7 s       | 0.9 s         | 1.5 s            | 406 MB             |
| Windows NSIS           | 0.9 s       | 1.6 s         | 1.3 s            | 395 MB             |
| macOS Apple Silicon    | 5.2 s       | 2.3 s         | 2.2 s            | 105 MB             |
| macOS Intel            | 4.0 s       | 1.0 s         | 1.1 s            | 52 MB              |
| Ubuntu 22.04, `.deb`   | 0.5 s       | 0.5 s         | 0.5 s            | 428 MB             |
| Ubuntu 22.04, AppImage | 1.2 s       | 0.7 s         | 1.2 s            | 457 MB             |
| Ubuntu 24.04, `.deb`   | 1.1 s       | 0.9 s         | 0.9 s            | 409 MB             |
| Ubuntu 24.04, AppImage | 1.4 s       | 0.6 s         | 0.9 s            | 450 MB             |
| Fedora 45, `.rpm`      | 0.5 s       | 0.5 s         | 0.5 s            | (not in this run)  |

Every start after the first is within the 3-second target. The first start of a fresh install
is slower on Windows and macOS: WebView2 creates its profile (the MSI's 8.7 s; the NSIS
install ran second, on a machine where WebView2 had already started), and macOS checks the
newly installed app. The AppImage unpacks itself on each start, as it does on machines without
FUSE.

Memory counts the app and, on Linux and Windows, the webview processes it starts (most of the
total); on macOS WebKit's processes run under the system and are not counted, so the macOS
figures are the app's own process only.

### The core, with a large database

PostgreSQL 16 in a development container (CI runs PostgreSQL 17 with the same budgets):

| Step                                           | Took   | Budget |
| ---------------------------------------------- | ------ | ------ |
| Catalog overview, cold (2,200+ objects)        | 106 ms | 5 s    |
| Catalog overview, cached                       | < 1 ms | 500 ms |
| Describe a table                               | 4 ms   | 2 s    |
| First page, primary key order                  | 5 ms   | 1 s    |
| First page, sorted by an unindexed column      | 79 ms  | 5 s    |
| A page 250,000 rows in                         | 149 ms | 5 s    |
| Query: first page of `SELECT *` (300,000 rows) | 16 ms  | 2 s    |
| Export 300,000 rows to CSV (33 MB)             | 2.6 s  | 30 s   |

A query returns its first page without reading the rest of the result; the grid and exports
stream rows; the catalog is read once and then rechecked by fingerprint.

### The interface, with a large database

Production build in Chromium, 1440×900:

| Measurement                            | Result    | Target or budget             |
| -------------------------------------- | --------- | ---------------------------- |
| Interface ready after the page loads   | 760 ms    | budget 3 s                   |
| Switching screens, first visit         | 15–219 ms | (loads the screen's code)    |
| Switching screens, later visits        | 12–83 ms  | target 100 ms, budget 500 ms |
| Tree links rendered for 10,000 objects | 200       | at most 200 per schema       |
| Search over 10,000 objects             | 37 ms     | budget 500 ms                |
| Grid rows in the page for 500 loaded   | 29        | under 80 (virtualized)       |
| Scrolling 500 rows to the end          | 8 ms      | budget 500 ms                |

The SQL screen is the slowest to revisit (83 ms) because the editor is rebuilt for the active
tab.

### Size

| Interface bundle            | Size   | Gzip   |
| --------------------------- | ------ | ------ |
| Loaded for the first screen | 879 KB | 246 KB |
| All screens                 | 2.9 MB | 691 KB |

The bundle is read from the app itself, not downloaded. Screens load their code on first visit;
the SQL editor (443 KB) is the largest. Package sizes per platform are listed in each CI run's
summary: about 11 MB for the `.deb`, `.rpm` and MSI, 8 MB for the NSIS installer, and 85 MB for
the AppImage, which carries WebKitGTK.
